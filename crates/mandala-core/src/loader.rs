//! Signed dynamic module loader and process monitor.
//!
//! Load pipeline (matches the design flow chart):
//! read bytes -> SHA-256 -> Ed25519 verify -> (fail: quarantine) ->
//! write the *verified bytes* to a private runtime copy -> `dlopen` ->
//! ABI/id check -> `init`.
//!
//! The runtime copy closes the time-of-check/time-of-use gap: what is loaded
//! is byte-for-byte what was verified, even if the file in `modules/` is
//! swapped afterwards.

use crate::crypto::{sha256, sha256_hex, TrustStore};
use crate::error::{MandalaError, Result};
use crate::ipc_bus::{IpcEnvelope, MicrokernelIpcBroker};
use crate::paths::HostPaths;
use libloading::{Library, Symbol};
use mandala_module_api::{
    cstr_lossy, EntryFn, HostApi, ModuleReply, ModuleVTable, ABI_VERSION, ENTRY_SYMBOL,
    SEND_ACL_DENIED, SEND_BAD_ARGS, SEND_OK, SEND_QUEUE_FULL,
};
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleState {
    NotInstalled,
    /// Present on disk, not loaded.
    Installed,
    Loaded {
        sha256: String,
    },
    Quarantined {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleStatus {
    pub id: String,
    pub state: ModuleState,
}

struct LoadedModule {
    // Field order matters: the library must outlive nothing that points into
    // it, and is dropped (dlclose) last.
    vtable: *const ModuleVTable,
    sha256: String,
    _lib: Library,
}

// SAFETY: every call into a module is serialized by `ModuleHost::loaded`'s
// mutex, and the vtable points at immutable static data inside `_lib`.
unsafe impl Send for LoadedModule {}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process-wide sink used by the `extern "C"` host callback handed to modules.
static BROKER_SINK: Mutex<Option<Arc<MicrokernelIpcBroker>>> = Mutex::new(None);

extern "C" fn host_send(
    sender: *const c_char,
    target: *const c_char,
    action: *const c_char,
    payload: *const c_char,
) -> i32 {
    // SAFETY: the module API contract requires NUL-terminated strings.
    let (sender, target, action, payload) = unsafe {
        (
            cstr_lossy(sender),
            cstr_lossy(target),
            cstr_lossy(action),
            cstr_lossy(payload),
        )
    };
    if sender.is_empty() || target.is_empty() {
        return SEND_BAD_ARGS;
    }
    let Some(broker) = lock(&BROKER_SINK).clone() else {
        return SEND_QUEUE_FULL;
    };
    let env = broker.envelope(&sender, &target, &action, &payload);
    match broker.try_dispatch(env) {
        Ok(()) => SEND_OK,
        Err(MandalaError::AclViolation { .. }) => {
            log::warn!("ACL denied module message {sender} -> {target}");
            SEND_ACL_DENIED
        }
        Err(_) => SEND_QUEUE_FULL,
    }
}

static HOST_API: HostApi = HostApi {
    abi_version: ABI_VERSION,
    send: host_send,
};

pub struct ModuleHost {
    paths: HostPaths,
    trust: TrustStore,
    loaded: Mutex<HashMap<String, LoadedModule>>,
    quarantined: Mutex<HashMap<String, String>>,
}

impl ModuleHost {
    /// Creates the host and registers `broker` as the target of module
    /// `host_send` callbacks.
    pub fn new(paths: HostPaths, trust: TrustStore, broker: Arc<MicrokernelIpcBroker>) -> Self {
        *lock(&BROKER_SINK) = Some(broker);
        Self {
            paths,
            trust,
            loaded: Mutex::new(HashMap::new()),
            quarantined: Mutex::new(HashMap::new()),
        }
    }

    pub fn paths(&self) -> &HostPaths {
        &self.paths
    }

    /// Signature verification keys this host accepts. The store installer
    /// verifies downloads against exactly the set the loader will enforce at
    /// load time — one owner of trust, no second guess.
    pub fn trust(&self) -> &TrustStore {
        &self.trust
    }

    /// Verify and load `id` (no-op if already loaded).
    pub fn ensure_loaded(&self, id: &str) -> Result<()> {
        let mut map = lock(&self.loaded);
        self.ensure_loaded_locked(&mut map, id)
    }

    fn ensure_loaded_locked(
        &self,
        map: &mut HashMap<String, LoadedModule>,
        id: &str,
    ) -> Result<()> {
        if map.contains_key(id) {
            return Ok(());
        }
        validate_module_id(id)?;
        if let Some(reason) = lock(&self.quarantined).get(id) {
            return Err(MandalaError::Quarantined {
                module: id.into(),
                reason: reason.clone(),
            });
        }
        let m = self.load_verified(id)?;
        map.insert(id.to_string(), m);
        Ok(())
    }

    fn load_verified(&self, id: &str) -> Result<LoadedModule> {
        let file = self.paths.module_file(id);
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(MandalaError::NotInstalled(id.into()))
            }
            Err(e) => return Err(MandalaError::io(&file, e)),
        };

        let sig_file = self.paths.signature_file(id);
        let verdict = match std::fs::read(&sig_file) {
            Ok(sig) => self.trust.verify(&sha256(&bytes), &sig),
            Err(_) => Err(MandalaError::Crypto("missing signature file".into())),
        };
        if let Err(e) = verdict {
            return Err(self.quarantine(id, &e.to_string()));
        }

        let digest_hex = sha256_hex(&bytes);
        let runtime_copy = self.paths.runtime_dir().join(format!(
            "{id}-{}.{}",
            &digest_hex[..16],
            std::env::consts::DLL_EXTENSION
        ));
        if !runtime_copy.exists() {
            write_private(&runtime_copy, &bytes).map_err(|e| MandalaError::io(&runtime_copy, e))?;
        }

        let load_err = |reason: String| MandalaError::LoadFailed {
            module: id.into(),
            reason,
        };

        // SAFETY: the binary was just verified against a trusted key.
        let lib = unsafe { Library::new(&runtime_copy) }.map_err(|e| load_err(e.to_string()))?;
        let vtable = unsafe {
            let entry: Symbol<EntryFn> = lib
                .get(ENTRY_SYMBOL)
                .map_err(|e| load_err(format!("missing entry symbol: {e}")))?;
            entry()
        };
        if vtable.is_null() {
            return Err(load_err("entry returned null".into()));
        }
        // SAFETY: non-null vtable pointing at static data in `lib`.
        let vt = unsafe { &*vtable };
        if vt.abi_version != ABI_VERSION {
            return Err(load_err(format!(
                "ABI version {} (host expects {ABI_VERSION})",
                vt.abi_version
            )));
        }
        let reported = unsafe { cstr_lossy((vt.id)()) };
        if reported != id {
            return Err(load_err(format!("module reports id `{reported}`")));
        }

        let data_dir = self.paths.data_dir(id);
        std::fs::create_dir_all(&data_dir).map_err(|e| MandalaError::io(&data_dir, e))?;
        let c_dir = CString::new(data_dir.to_string_lossy().replace('\0', "")).unwrap_or_default();
        let rc = (vt.init)(c_dir.as_ptr(), &HOST_API);
        if rc != 0 {
            return Err(load_err(format!("init failed with code {rc}")));
        }
        log::info!("loaded module {id} sha256={digest_hex}");
        Ok(LoadedModule {
            vtable,
            sha256: digest_hex,
            _lib: lib,
        })
    }

    fn quarantine(&self, id: &str, reason: &str) -> MandalaError {
        let file = self.paths.module_file(id);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = self
            .paths
            .quarantine_dir()
            .join(format!("{id}-{ts}.quarantined"));
        let moved = std::fs::rename(&file, &dest)
            .or_else(|_| std::fs::copy(&file, &dest).and_then(|_| std::fs::remove_file(&file)));
        let full_reason = match moved {
            Ok(()) => reason.to_string(),
            Err(e) => format!("{reason} (could not move file: {e})"),
        };
        log::error!("quarantined {id}: {full_reason}");
        lock(&self.quarantined).insert(id.to_string(), full_reason.clone());
        MandalaError::Quarantined {
            module: id.into(),
            reason: full_reason,
        }
    }

    /// Call `action` on module `id`, loading it first if necessary.
    pub fn call(&self, id: &str, action: &str, payload: &str) -> Result<ModuleReply> {
        let mut map = lock(&self.loaded);
        self.ensure_loaded_locked(&mut map, id)?;
        let m = map.get(id).expect("just loaded");
        let c = |s: &str| CString::new(s.replace('\0', "")).unwrap_or_default();
        let (a, p) = (c(action), c(payload));
        // SAFETY: vtable is valid while the library is loaded; we hold the
        // map lock for the duration of the call, so it cannot be unloaded.
        let vt = unsafe { &*m.vtable };
        let raw = (vt.handle)(a.as_ptr(), p.as_ptr());
        if raw.is_null() {
            return Ok(ModuleReply::err("module returned null"));
        }
        let json = unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        (vt.free_string)(raw);
        Ok(ModuleReply::from_json(&json))
    }

    pub fn unload(&self, id: &str) -> bool {
        lock(&self.loaded).remove(id).is_some()
    }

    /// Forget a quarantine verdict for `id` so a corrected binary (for
    /// example one freshly installed from the store) can be retried. Without
    /// this the only recovery would be the monitor's global RESTART.
    pub fn clear_quarantine(&self, id: &str) -> bool {
        lock(&self.quarantined).remove(id).is_some()
    }

    pub fn unload_all(&self) -> usize {
        let mut map = lock(&self.loaded);
        let n = map.len();
        map.clear();
        n
    }

    /// Unload everything and clear quarantine verdicts so files can be retried.
    pub fn restart_all(&self) -> usize {
        let n = self.unload_all();
        lock(&self.quarantined).clear();
        n
    }

    pub fn status_of(&self, id: &str) -> ModuleStatus {
        let state = if let Some(m) = lock(&self.loaded).get(id) {
            ModuleState::Loaded {
                sha256: m.sha256.clone(),
            }
        } else if let Some(r) = lock(&self.quarantined).get(id) {
            ModuleState::Quarantined { reason: r.clone() }
        } else if self.paths.module_file(id).exists() {
            ModuleState::Installed
        } else {
            ModuleState::NotInstalled
        };
        ModuleStatus {
            id: id.to_string(),
            state,
        }
    }

    pub fn status(&self, ids: impl IntoIterator<Item = &'static str>) -> Vec<ModuleStatus> {
        ids.into_iter().map(|id| self.status_of(id)).collect()
    }
}

/// Module ids become file names: only `[a-z0-9-_]` is accepted.
pub fn validate_module_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(MandalaError::LoadFailed {
            module: id.into(),
            reason: "invalid module id".into(),
        })
    }
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o700)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Render the status list for the process-monitor overlay.
pub fn format_monitor(statuses: &[ModuleStatus]) -> String {
    let mut out = String::from("MODULE PROCESS MONITOR\n\n");
    for s in statuses {
        let line = match &s.state {
            ModuleState::Loaded { sha256 } => format!("[RUN ] {:<16} {}", s.id, &sha256[..12]),
            ModuleState::Installed => format!("[IDLE] {:<16} installed", s.id),
            ModuleState::NotInstalled => format!("[ -- ] {:<16} not installed", s.id),
            ModuleState::Quarantined { reason } => format!("[QUAR] {:<16} {}", s.id, reason),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    match crate::sys::process_rss_kb() {
        Some(kb) => out.push_str(&format!(
            "process RSS: {kb} KiB (design target < 5120 KiB)\n"
        )),
        None => out.push_str("process RSS: not exposed by this OS\n"),
    }
    out
}

/// Convenience for the router: the error text shown to the user.
pub fn describe_error(e: &MandalaError) -> String {
    match e {
        MandalaError::NotInstalled(id) => {
            format!("{id} is not installed. Place a signed module in the modules directory.")
        }
        other => other.to_string(),
    }
}

/// Used by the router to deliver module replies back through the broker.
pub fn reply_envelope(
    broker: &MicrokernelIpcBroker,
    request: &IpcEnvelope,
    ok: bool,
    json: &str,
) -> IpcEnvelope {
    broker.envelope(
        &request.target_module,
        crate::ipc_bus::HOST,
        if ok { "reply" } else { "error" },
        json,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc_bus::AccessControlList;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn host_with(sk: &SigningKey) -> (tempfile::TempDir, ModuleHost) {
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::new(dir.path());
        paths.ensure().unwrap();
        let (broker, _rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
        let host = ModuleHost::new(
            paths,
            TrustStore::new(vec![sk.verifying_key()]),
            Arc::new(broker),
        );
        (dir, host)
    }

    #[test]
    fn missing_module_is_not_installed() {
        let sk = SigningKey::generate(&mut OsRng);
        let (_d, host) = host_with(&sk);
        assert!(matches!(
            host.call("app-notes", "launch", ""),
            Err(MandalaError::NotInstalled(_))
        ));
        assert_eq!(host.status_of("app-notes").state, ModuleState::NotInstalled);
    }

    #[test]
    fn unsigned_module_is_quarantined() {
        let sk = SigningKey::generate(&mut OsRng);
        let (_d, host) = host_with(&sk);
        let file = host.paths().module_file("app-notes");
        std::fs::write(&file, b"not really a library").unwrap();
        let err = host.call("app-notes", "launch", "").unwrap_err();
        assert!(matches!(err, MandalaError::Quarantined { .. }), "{err}");
        assert!(!file.exists(), "binary must be moved out of modules/");
        assert_eq!(
            std::fs::read_dir(host.paths().quarantine_dir())
                .unwrap()
                .count(),
            1
        );
        // Subsequent calls keep failing without touching disk.
        assert!(matches!(
            host.call("app-notes", "launch", ""),
            Err(MandalaError::Quarantined { .. })
        ));
        assert!(matches!(
            host.status_of("app-notes").state,
            ModuleState::Quarantined { .. }
        ));
    }

    #[test]
    fn tampered_module_is_quarantined() {
        let sk = SigningKey::generate(&mut OsRng);
        let (_d, host) = host_with(&sk);
        let file = host.paths().module_file("app-notes");
        std::fs::write(&file, b"original bytes").unwrap();
        let sig = crate::crypto::sign_binary(&sk, b"original bytes");
        std::fs::write(
            host.paths().signature_file("app-notes"),
            hex::encode(sig.to_bytes()),
        )
        .unwrap();
        // Attacker swaps the binary after signing.
        std::fs::write(&file, b"evil bytes").unwrap();
        assert!(matches!(
            host.call("app-notes", "launch", ""),
            Err(MandalaError::Quarantined { .. })
        ));
    }

    #[test]
    fn validly_signed_garbage_fails_to_load_but_is_not_quarantined() {
        let sk = SigningKey::generate(&mut OsRng);
        let (_d, host) = host_with(&sk);
        let bytes = b"signed but not a shared object";
        std::fs::write(host.paths().module_file("app-notes"), bytes).unwrap();
        let sig = crate::crypto::sign_binary(&sk, bytes);
        std::fs::write(
            host.paths().signature_file("app-notes"),
            hex::encode(sig.to_bytes()),
        )
        .unwrap();
        assert!(matches!(
            host.call("app-notes", "launch", ""),
            Err(MandalaError::LoadFailed { .. })
        ));
    }

    #[test]
    fn path_traversal_ids_are_rejected() {
        let sk = SigningKey::generate(&mut OsRng);
        let (_d, host) = host_with(&sk);
        for bad in ["../evil", "a/b", "", "UPPER", "a b"] {
            assert!(host.call(bad, "launch", "").is_err(), "{bad}");
        }
    }

    #[test]
    fn monitor_formatting_lists_every_state() {
        let s = vec![
            ModuleStatus {
                id: "a".into(),
                state: ModuleState::Installed,
            },
            ModuleStatus {
                id: "b".into(),
                state: ModuleState::NotInstalled,
            },
            ModuleStatus {
                id: "c".into(),
                state: ModuleState::Quarantined {
                    reason: "bad sig".into(),
                },
            },
            ModuleStatus {
                id: "d".into(),
                state: ModuleState::Loaded {
                    sha256: "0123456789abcdef".into(),
                },
            },
        ];
        let t = format_monitor(&s);
        for needle in ["[IDLE]", "[ -- ]", "[QUAR]", "[RUN ]", "bad sig"] {
            assert!(t.contains(needle), "{needle}");
        }
    }
}
