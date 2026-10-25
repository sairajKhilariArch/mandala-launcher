//! Module store: catalog fetch, offline cache, and the verified installer.
//!
//! The security model is layered: the catalog is discovery only (it comes over
//! HTTPS from GitHub Pages), a download must match the catalog's advertised
//! SHA-256 and ELF ABI, and — the actual trust boundary — must carry an
//! Ed25519 signature that verifies against the launcher's baked-in release
//! key. Only then are the bytes moved into `modules/`, and the loader verifies
//! the signature *again* on every load.
//!
//! Everything here is blocking; the shell runs it on the blocking pool from
//! `Effect::FetchCatalog` / `Effect::InstallModule`, never on the UI thread.

use mandala_core::catalog::{self, Catalog, CatalogEntry, MAX_MODULE_BYTES};
use mandala_core::crypto::{sha256, sha256_hex};
use mandala_core::loader::ModuleHost;
use mandala_core::state::{StoreRow, StoreStatus};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Published catalog + binaries (GitHub Pages project site).
pub const DEFAULT_STORE_URL: &str = "https://sairajkhilariarch.github.io/mandala-launcher";
const USER_AGENT: &str = "Mandala-Store/1.0";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CATALOG_BYTES: usize = 512 * 1024;
const MAX_SIGNATURE_BYTES: usize = 4096;

#[derive(Debug)]
pub struct FetchOutcome {
    pub rows: Vec<StoreRow>,
    /// Set when the live fetch failed and the last cached catalog is shown.
    pub note: Option<String>,
}

/// One store instance per launcher: owns the resolved base URL and the last
/// catalog the installer can act on.
pub struct Store {
    base: String,
    catalog: Mutex<Option<Catalog>>,
}

impl Store {
    /// Resolve the catalog base: `MANDALA_STORE_URL` (desktop/dev), then
    /// `<home>/store.url` (on-device dev override — app-private, and a
    /// tampered value can only redirect downloads that still must pass the
    /// signature check), else the published Pages site.
    pub fn new(root: &Path) -> Self {
        let env = std::env::var("MANDALA_STORE_URL").unwrap_or_default();
        let base = if env.trim().is_empty() {
            std::fs::read_to_string(root.join("store.url"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_STORE_URL.to_string())
        } else {
            env.trim().to_string()
        };
        Self::with_base(&base)
    }

    pub fn with_base(base: &str) -> Self {
        Self {
            base: base.to_string(),
            catalog: Mutex::new(None),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Fetch the catalog for the running platform and compute row statuses.
    /// Desktop builds have no published registry, so they get an explanatory
    /// error instead of a broken store.
    pub fn fetch(&self, host: &ModuleHost) -> Result<FetchOutcome, String> {
        let abi = catalog::current_abi().ok_or_else(|| {
            "The module catalog is published for Android builds only. \
             On desktop, install modules with scripts/dev-setup.sh."
                .to_string()
        })?;
        self.fetch_abi(host, abi)
    }

    fn fetch_abi(&self, host: &ModuleHost, abi: &str) -> Result<FetchOutcome, String> {
        let url = format!("{}/registry/{abi}.json", self.trimmed_base());
        match http_get(&url, MAX_CATALOG_BYTES) {
            Ok(bytes) => {
                // A served-but-invalid catalog is a publisher error worth
                // showing verbatim: falling back to cached rows would hide
                // schema skew behind an "offline" label.
                let text =
                    String::from_utf8(bytes).map_err(|e| format!("catalog is not UTF-8: {e}"))?;
                let cat = Catalog::parse(&text, abi).map_err(|e| e.to_string())?;
                // Cache only a *validated* catalog: it is what offline mode
                // serves, and a corrupt one must never become the fallback.
                let _ = self.save_cache(&host.paths().store_dir(), abi, &cat);
                let rows = rows_from(host, &cat);
                *self.catalog() = Some(cat);
                Ok(FetchOutcome { rows, note: None })
            }
            Err(live_err) => match self.load_cache(&host.paths().store_dir(), abi) {
                Some(cat) => {
                    let rows = rows_from(host, &cat);
                    *self.catalog() = Some(cat);
                    Ok(FetchOutcome {
                        rows,
                        note: Some(format!(
                            "offline — showing the last catalog fetched ({live_err})"
                        )),
                    })
                }
                None => Err(live_err),
            },
        }
    }

    /// Rows for the loaded catalog (empty when nothing was fetched yet).
    pub fn rows(&self, host: &ModuleHost) -> Vec<StoreRow> {
        match self.catalog().as_ref() {
            Some(cat) => rows_from(host, cat),
            None => Vec::new(),
        }
    }

    /// Download, verify and atomically install one catalog entry.
    /// Returns `(ok, message)`; on failure the previous module is untouched.
    pub fn install(&self, host: &ModuleHost, id: &str) -> (bool, String) {
        match self.install_inner(host, id) {
            Ok(msg) => (true, msg),
            Err(e) => (false, e),
        }
    }

    fn install_inner(&self, host: &ModuleHost, id: &str) -> Result<String, String> {
        let (entry, abi) = {
            let guard = self.catalog();
            let cat = guard
                .as_ref()
                .ok_or_else(|| "no catalog loaded — open the store first".to_string())?;
            let entry = cat
                .entry(id)
                .cloned()
                .ok_or_else(|| format!("`{id}` is not in the catalog"))?;
            (entry, cat.abi.clone())
        };

        let tmp_dir = host.paths().store_dir();
        std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("store cache dir: {e}"))?;
        let tmp_so = tmp_dir.join(format!("{id}.so.part"));
        let tmp_sig = tmp_dir.join(format!("{id}.so.sig.part"));

        // Stage + verify entirely in `data/.store` first: any failure leaves
        // `modules/` untouched, so a bad update can never break a working one.
        if let Err(e) = self.stage(host, &entry, &abi, &tmp_so, &tmp_sig) {
            let _ = std::fs::remove_file(&tmp_so);
            let _ = std::fs::remove_file(&tmp_sig);
            return Err(e);
        }

        // Drop the loaded copy and any stale quarantine verdict *before* the
        // swap: a launch racing this install must not keep serving the old
        // handle, and the old verdict is about bytes we are replacing.
        host.unload(id);
        host.clear_quarantine(id);

        // Both renames are on the same filesystem (under the Mandala home).
        // The microsecond window where the new `.so` sits next to the old
        // `.sig` is recoverable: the loader re-verifies on every load,
        // quarantines a mismatch, and a re-install repairs it.
        std::fs::rename(&tmp_so, host.paths().module_file(id))
            .map_err(|e| format!("installing {id}: {e}"))?;
        std::fs::rename(&tmp_sig, host.paths().signature_file(id))
            .map_err(|e| format!("installing signature for {id}: {e}"))?;
        Ok(format!("{} {} installed", entry.label, entry.version))
    }

    /// Download + verify into `tmp_so` / `tmp_sig`. Touches nothing else.
    fn stage(
        &self,
        host: &ModuleHost,
        entry: &CatalogEntry,
        abi: &str,
        tmp_so: &Path,
        tmp_sig: &Path,
    ) -> Result<(), String> {
        let id = &entry.id;
        let bytes = http_get(&self.join(&entry.url), MAX_MODULE_BYTES as usize)
            .map_err(|e| format!("{id}: {e}"))?;
        if bytes.len() as u64 != entry.size {
            return Err(format!(
                "{id}: download is {} bytes, catalog says {}",
                bytes.len(),
                entry.size
            ));
        }
        if sha256_hex(&bytes) != entry.sha256 {
            return Err(format!(
                "{id}: downloaded binary does not match the catalog checksum"
            ));
        }
        check_elf(&bytes, abi).map_err(|e| format!("{id}: {e}"))?;

        let sig = http_get(&self.join(&entry.sig), MAX_SIGNATURE_BYTES)
            .map_err(|e| format!("{id} signature: {e}"))?;
        host.trust()
            .verify(&sha256(&bytes), &sig)
            .map_err(|e| format!("{id}: signature rejected: {e}"))?;

        std::fs::write(tmp_so, &bytes).map_err(|e| format!("{id}: staging binary: {e}"))?;
        std::fs::write(tmp_sig, &sig).map_err(|e| format!("{id}: staging signature: {e}"))?;
        Ok(())
    }

    fn trimmed_base(&self) -> &str {
        self.base.trim_end_matches('/')
    }

    fn join(&self, rel: &str) -> String {
        format!("{}/{}", self.trimmed_base(), rel)
    }

    fn catalog(&self) -> std::sync::MutexGuard<'_, Option<Catalog>> {
        self.catalog.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cache_path(dir: &Path, abi: &str) -> std::path::PathBuf {
        dir.join(format!("catalog-{abi}.json"))
    }

    fn save_cache(&self, dir: &Path, abi: &str, cat: &Catalog) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(cat).map_err(std::io::Error::other)?;
        std::fs::write(Self::cache_path(dir, abi), json)
    }

    fn load_cache(&self, dir: &Path, abi: &str) -> Option<Catalog> {
        let text = std::fs::read_to_string(Self::cache_path(dir, abi)).ok()?;
        // Re-validate instead of trusting the file: the cache is on the same
        // app-private disk as the modules, but validation is cheap.
        Catalog::parse(&text, abi).ok()
    }
}

fn rows_from(host: &ModuleHost, cat: &Catalog) -> Vec<StoreRow> {
    cat.modules.iter().map(|e| row_for(host, e)).collect()
}

/// Status of one entry: hash the installed file (the loader's own ground
/// truth is the bytes, so no separate version bookkeeping can drift).
fn row_for(host: &ModuleHost, e: &CatalogEntry) -> StoreRow {
    let status = match std::fs::read(host.paths().module_file(&e.id)) {
        Ok(bytes) if sha256_hex(&bytes) == e.sha256 => StoreStatus::Installed,
        Ok(_) => StoreStatus::Update,
        Err(_) => StoreStatus::NotInstalled,
    };
    StoreRow {
        id: e.id.clone(),
        label: e.label.clone(),
        description: e.description.clone(),
        version: e.version.clone(),
        status,
    }
}

/// GET `url` capped at `max` bytes (ureq's default body cap is 10 MB).
fn http_get(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .config()
        .timeout_global(Some(TIMEOUT))
        .build()
        .call()
        .map_err(|e| format!("fetch {url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("fetch {url}: HTTP {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(max as u64)
        .read_to_vec()
        .map_err(|e| format!("fetch {url}: {e}"))
}

/// A module must be a 64-bit little-endian ELF whose machine matches the ABI
/// the catalog claims — cheap insurance against serving the wrong artifact
/// (or a payload that is not a shared object at all).
fn check_elf(bytes: &[u8], abi: &str) -> Result<(), String> {
    if bytes.len() < 20 || &bytes[..4] != b"\x7fELF" {
        return Err("download is not an ELF shared object".into());
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("download is not a 64-bit little-endian ELF".into());
    }
    let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    let want = catalog::abi_e_machine(abi).ok_or_else(|| format!("unknown ABI `{abi}`"))?;
    if machine != want {
        return Err(format!(
            "download targets ELF machine {machine}, expected {want} for {abi}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_core::crypto::{parse_signing_key, sign_binary, TrustStore};
    use mandala_core::ipc_bus::{AccessControlList, MicrokernelIpcBroker};
    use mandala_core::paths::HostPaths;
    use std::io::{Read as _, Write as _};
    use std::sync::Arc;

    const ABI: &str = "android-arm64";

    fn seed_hex(seed: u8) -> String {
        format!("{seed:02x}").repeat(32)
    }

    fn test_host(dir: &Path, seed: u8) -> ModuleHost {
        let paths = HostPaths::new(dir);
        paths.ensure().unwrap();
        let sk = parse_signing_key(&seed_hex(seed)).unwrap();
        let (broker, _rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
        ModuleHost::new(
            paths,
            TrustStore::new(vec![sk.verifying_key()]),
            Arc::new(broker),
        )
    }

    /// Minimal ELF64-LE header with `machine`, plus payload bytes.
    fn fake_elf(machine: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 64];
        v[..4].copy_from_slice(b"\x7fELF");
        v[4] = 2; // ELFCLASS64
        v[5] = 1; // ELFDATA2LSB
        v[18..20].copy_from_slice(&machine.to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn catalog_json(id: &str, abi: &str, so: &[u8]) -> String {
        serde_json::json!({
            "schema": 1,
            "abi": abi,
            "generated": "test fixture",
            "modules": [{
                "id": id,
                "label": "Notes",
                "description": "Local notes.",
                "version": "1.0.0",
                "sha256": sha256_hex(so),
                "size": so.len(),
                "url": format!("modules/{abi}/{id}.so"),
                "sig": format!("modules/{abi}/{id}.so.sig"),
            }]
        })
        .to_string()
    }

    fn sig_for(seed: u8, so: &[u8]) -> Vec<u8> {
        let sk = parse_signing_key(&seed_hex(seed)).unwrap();
        sign_binary(&sk, so).to_bytes().to_vec()
    }

    /// Serve `files` (path -> body) on 127.0.0.1 until the process exits.
    fn mock_server(files: Vec<(String, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    /// A base URL nothing listens on (bind then drop the listener).
    fn dead_base() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    fn served(so: &[u8], sig: &[u8]) -> Vec<(String, Vec<u8>)> {
        vec![
            (format!("/modules/{ABI}/app-notes.so"), so.to_vec()),
            (format!("/modules/{ABI}/app-notes.so.sig"), sig.to_vec()),
            (
                format!("/registry/{ABI}.json"),
                catalog_json("app-notes", ABI, so).into_bytes(),
            ),
        ]
    }

    #[test]
    fn install_places_verified_files_and_marks_installed() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let so = fake_elf(183, b"module payload");
        let base = mock_server(served(&so, &sig_for(7, &so)));
        let store = Store::with_base(&base);

        let out = store.fetch_abi(&host, ABI).unwrap();
        assert!(out.note.is_none());
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].status, StoreStatus::NotInstalled);

        let (ok, msg) = store.install(&host, "app-notes");
        assert!(ok, "{msg}");
        assert_eq!(
            std::fs::read(host.paths().module_file("app-notes")).unwrap(),
            so
        );
        assert!(host.paths().signature_file("app-notes").exists());
        assert_eq!(store.rows(&host)[0].status, StoreStatus::Installed);
        // No staging leftovers.
        let store_dir = host.paths().store_dir();
        assert!(!store_dir.join("app-notes.so.part").exists());
        assert!(!store_dir.join("app-notes.so.sig.part").exists());
    }

    #[test]
    fn tampered_download_is_rejected_and_previous_module_kept() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let old = fake_elf(183, b"old version");
        std::fs::write(host.paths().module_file("app-notes"), &old).unwrap();

        // Catalog announces `announced` (correctly signed) but the server
        // serves `evil` under the same URL. Same length, so this exercises
        // the checksum (not the size) rejection path.
        let announced = fake_elf(183, b"good");
        let evil = fake_elf(183, b"evil");
        let mut files = served(&announced, &sig_for(7, &announced));
        files[0].1 = evil;
        let store = Store::with_base(&mock_server(files));

        // Still shows as an available update before the failed install.
        assert_eq!(
            store.fetch_abi(&host, ABI).unwrap().rows[0].status,
            StoreStatus::Update
        );
        let (ok, msg) = store.install(&host, "app-notes");
        assert!(!ok);
        assert!(msg.contains("checksum"), "{msg}");
        assert_eq!(
            std::fs::read(host.paths().module_file("app-notes")).unwrap(),
            old
        );
        assert!(!host.paths().store_dir().join("app-notes.so.part").exists());
    }

    #[test]
    fn wrong_abi_binary_is_rejected_even_when_signed() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        // Catalog claims arm64; the (validly signed) binary is x86_64.
        let so = fake_elf(62, b"x86 payload");
        let store = Store::with_base(&mock_server(served(&so, &sig_for(7, &so))));
        store.fetch_abi(&host, ABI).unwrap();

        let (ok, msg) = store.install(&host, "app-notes");
        assert!(!ok);
        assert!(msg.contains("ELF machine"), "{msg}");
        assert!(!host.paths().module_file("app-notes").exists());
    }

    #[test]
    fn foreign_signature_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let so = fake_elf(183, b"payload");
        // Signed by key 9, but the launcher only trusts key 7.
        let store = Store::with_base(&mock_server(served(&so, &sig_for(9, &so))));
        store.fetch_abi(&host, ABI).unwrap();

        let (ok, msg) = store.install(&host, "app-notes");
        assert!(!ok);
        assert!(msg.contains("signature"), "{msg}");
        assert!(!host.paths().module_file("app-notes").exists());
    }

    #[test]
    fn truncated_download_fails_the_size_check() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let so = fake_elf(183, b"payload");
        let mut files = served(&so, &sig_for(7, &so));
        files[0].1.truncate(40);
        let store = Store::with_base(&mock_server(files));
        store.fetch_abi(&host, ABI).unwrap();

        let (ok, msg) = store.install(&host, "app-notes");
        assert!(!ok);
        assert!(msg.contains("bytes"), "{msg}");
        assert!(!host.paths().module_file("app-notes").exists());
    }

    #[test]
    fn unknown_module_and_unloaded_catalog_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let so = fake_elf(183, b"payload");
        let store = Store::with_base(&mock_server(served(&so, &sig_for(7, &so))));

        let (ok, msg) = store.install(&host, "app-notes");
        assert!(!ok);
        assert!(msg.contains("open the store"), "{msg}");

        store.fetch_abi(&host, ABI).unwrap();
        let (ok, msg) = store.install(&host, "app-missing");
        assert!(!ok);
        assert!(msg.contains("not in the catalog"), "{msg}");
    }

    #[test]
    fn offline_fetch_falls_back_to_the_cached_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let so = fake_elf(183, b"payload");
        let live = Store::with_base(&mock_server(served(&so, &sig_for(7, &so))));
        let first = live.fetch_abi(&host, ABI).unwrap();
        assert!(first.note.is_none());
        assert!(Store::cache_path(&host.paths().store_dir(), ABI).exists());

        // A second launcher run with no connectivity, same home.
        let offline = Store::with_base(&dead_base());
        let second = offline.fetch_abi(&host, ABI).unwrap();
        assert!(second.note.unwrap().contains("offline"));
        assert_eq!(second.rows.len(), 1);
        // The cache is enough to install from later once the network is back.
        assert!(offline.rows(&host)[0].status == StoreStatus::NotInstalled);
    }

    #[test]
    fn corrupt_cache_is_ignored_not_served() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        let cache = Store::cache_path(&host.paths().store_dir(), ABI);
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, "{ truncated").unwrap();

        let offline = Store::with_base(&dead_base());
        let err = offline.fetch_abi(&host, ABI).unwrap_err();
        assert!(err.contains("fetch"), "{err}");
    }

    #[test]
    fn invalid_live_catalog_is_an_error_not_a_cache_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let host = test_host(dir.path(), 7);
        // A good catalog is cached first…
        let so = fake_elf(183, b"payload");
        let live = Store::with_base(&mock_server(served(&so, &sig_for(7, &so))));
        live.fetch_abi(&host, ABI).unwrap();

        // …then the publisher serves a broken one. Loud failure: a tampered
        // registry must not silently degrade to stale data.
        let mut files = served(&so, &sig_for(7, &so));
        files[2].1 = b"{\"schema\": 99, \"abi\": \"android-arm64\", \"modules\": []}".to_vec();
        let bad = Store::with_base(&mock_server(files));
        let err = bad.fetch_abi(&host, ABI).unwrap_err();
        assert!(err.contains("schema 99"), "{err}");
    }

    #[test]
    fn elf_check_rejects_garbage_and_wrong_machine() {
        assert!(check_elf(b"not an elf at all, just garbage", ABI).is_err());
        let mut short = vec![0u8; 10];
        short[..4].copy_from_slice(b"\x7fELF");
        assert!(check_elf(&short, ABI).is_err());
        assert!(check_elf(&fake_elf(183, b""), ABI).is_ok());
        assert!(check_elf(&fake_elf(62, b""), ABI).is_err());
        assert!(check_elf(&fake_elf(183, b""), "android-x86_64").is_err());
        assert!(check_elf(&fake_elf(183, b""), "windows").is_err());
    }

    #[test]
    fn base_url_falls_back_to_the_published_site() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        assert_eq!(store.base(), DEFAULT_STORE_URL);

        std::fs::write(dir.path().join("store.url"), "http://127.0.0.1:9/dev\n").unwrap();
        assert_eq!(Store::new(dir.path()).base(), "http://127.0.0.1:9/dev");
    }
}
