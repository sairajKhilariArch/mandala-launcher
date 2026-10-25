//! C2 Password Vault: an encrypted, on-device credential store.
//!
//! Roadmap deviation, stated plainly: this module does **not** parse KeePass
//! KDBX databases. KDBX needs a large crypto/XML stack and its own format
//! versions; instead the vault stores its own file and imports the plain
//! KeePassDX CSV export (`title,username,password`) that the app can produce.
//!
//! Crypto:
//! * key = Argon2id(passphrase, 16-byte random salt, m=19 MiB, t=2, p=1)
//! * body = XChaCha20-Poly1305(JSON entries) with a random 24-byte nonce
//! * the envelope (salt, nonce, ciphertext) is stored as JSON with mode 0600
//!
//! Actions
//! * `launch` – lock screen or entry list
//! * `run`    – one command line (see `help`)

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const MAX_NAME: usize = 64;
const MAX_USER: usize = 128;
const MAX_PASS: usize = 256;
const MAX_ENTRIES: usize = 200;
const MAX_IMPORT_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub username: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Envelope {
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// Parsed command line result, kept separate from the module so it is testable.
pub struct Vault {
    file: Option<PathBuf>,
    key: Option<[u8; KEY_LEN]>,
    /// Salt the current key was derived from; reused for every rewrite.
    salt: Option<[u8; SALT_LEN]>,
    entries: Vec<Entry>,
}

impl Vault {
    pub fn new() -> Self {
        Self {
            file: None,
            key: None,
            salt: None,
            entries: Vec::new(),
        }
    }

    fn path(&self) -> Result<&Path, String> {
        self.file
            .as_deref()
            .ok_or_else(|| "vault storage not initialised".to_string())
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.is_some()
    }

    /// Derive the vault key from a passphrase and salt.
    fn derive(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
        let params = Params::new(19 * 1024, 2, 1, Some(KEY_LEN))
            .map_err(|e| format!("argon2 params: {e}"))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut key = [0u8; KEY_LEN];
        argon
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|e| format!("key derivation failed: {e}"))?;
        Ok(key)
    }

    fn encrypt(key: &[u8; KEY_LEN], salt: &[u8], entries: &[Entry]) -> Result<Envelope, String> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let plaintext = serde_json::to_vec(entries).map_err(|e| format!("encode: {e}"))?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce_bytes), plaintext.as_slice())
            .map_err(|_| "encryption failed".to_string())?;
        Ok(Envelope {
            version: 1,
            salt: hex::encode(salt),
            nonce: hex::encode(nonce_bytes),
            ciphertext: hex::encode(ciphertext),
        })
    }

    fn decrypt(key: &[u8; KEY_LEN], envelope: &Envelope) -> Result<Vec<Entry>, String> {
        let nonce =
            hex::decode(&envelope.nonce).map_err(|_| "corrupted vault nonce".to_string())?;
        if nonce.len() != NONCE_LEN {
            return Err("corrupted vault nonce".into());
        }
        let ciphertext =
            hex::decode(&envelope.ciphertext).map_err(|_| "corrupted vault body".to_string())?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext.as_slice())
            .map_err(|_| "wrong passphrase or corrupted vault".to_string())?;
        serde_json::from_slice(&plaintext).map_err(|_| "corrupted vault contents".to_string())
    }

    fn read_envelope(&self) -> Result<Envelope, String> {
        let path = self.path()?;
        let text = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "no vault yet: run `init <passphrase>`".to_string(),
            _ => format!("could not read vault: {e}"),
        })?;
        serde_json::from_str(&text).map_err(|e| format!("corrupted vault header: {e}"))
    }

    fn persist(&self) -> Result<(), String> {
        let Some(key) = self.key else {
            return Err("vault is locked".into());
        };
        let salt = self.salt.ok_or_else(|| "vault is locked".to_string())?;
        let path = self.path()?;
        let envelope = Vault::encrypt(&key, &salt, &self.entries)?;
        let json = serde_json::to_string_pretty(&envelope).map_err(|e| format!("encode: {e}"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("save: {e}"))?;
        }
        write_private(path, json.as_bytes()).map_err(|e| format!("save: {e}"))
    }

    /// The user alone decides the master passphrase; it is never stored.
    fn create(&mut self, passphrase: &str) -> Result<String, String> {
        if passphrase.trim().len() < 8 {
            return Err("passphrase must be at least 8 characters".into());
        }
        if self.path()?.exists() {
            return Err("a vault already exists; use `unlock <passphrase>`".into());
        }
        let mut salt = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);
        let key = Vault::derive(passphrase, &salt)?;
        self.key = Some(key);
        self.salt = Some(salt);
        self.entries.clear();
        self.persist()?;
        Ok("vault created and unlocked".into())
    }

    fn unlock(&mut self, passphrase: &str) -> Result<String, String> {
        let envelope = self.read_envelope()?;
        let salt = hex::decode(&envelope.salt).map_err(|_| "corrupted vault salt".to_string())?;
        if salt.len() != SALT_LEN {
            return Err("corrupted vault salt".into());
        }
        let key = Vault::derive(passphrase, &salt)?;
        let entries = Vault::decrypt(&key, &envelope)?;
        let mut arr = [0u8; SALT_LEN];
        arr.copy_from_slice(&salt);
        self.key = Some(key);
        self.salt = Some(arr);
        self.entries = entries;
        Ok(format!("vault unlocked ({} entries)", self.entries.len()))
    }

    fn lock(&mut self) -> String {
        self.key = None;
        self.salt = None;
        self.entries.clear();
        "vault locked".into()
    }

    fn listing(&self) -> String {
        if !self.is_unlocked() {
            return "VAULT LOCKED\n\nunlock <passphrase>\n(no vault yet? `init <passphrase>`)"
                .into();
        }
        if self.entries.is_empty() {
            return "VAULT (unlocked, empty)\n\nadd <name> <username> <password>\nimport <csv-path>   (KeePassDX CSV export)".into();
        }
        let rows: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                format!(
                    "  {:<24} {}",
                    truncate(&e.name, 24),
                    truncate(&e.username, 32)
                )
            })
            .collect();
        format!(
            "VAULT (unlocked, {} entries)\n\n{}\n\nget <name> reveals a password",
            self.entries.len(),
            rows.join("\n")
        )
    }

    fn add(&mut self, name: &str, username: &str, password: &str) -> Result<String, String> {
        if !self.is_unlocked() {
            return Err("vault is locked".into());
        }
        let name = name.trim();
        if name.is_empty() || username.trim().is_empty() || password.is_empty() {
            return Err("usage: add <name> <username> <password>".into());
        }
        if name.len() > MAX_NAME || username.len() > MAX_USER || password.len() > MAX_PASS {
            return Err("entry field too long".into());
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(format!("vault is full ({MAX_ENTRIES} entries)"));
        }
        if self
            .entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("`{name}` already exists (remove it first)"));
        }
        self.entries.push(Entry {
            name: name.into(),
            username: username.trim().into(),
            password: password.into(),
        });
        self.entries.sort_by_key(|e| e.name.to_ascii_lowercase());
        self.persist()?;
        Ok(format!("saved `{name}`"))
    }

    fn find(&self, needle: &str) -> Result<&Entry, String> {
        let q = needle.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Err("missing entry name".into());
        }
        let hits: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().contains(&q))
            .collect();
        match hits.len() {
            0 => Err(format!("no entry matches `{needle}`")),
            1 => Ok(hits[0]),
            _ => Err(format!(
                "`{needle}` matches {} entries; be more specific",
                hits.len()
            )),
        }
    }

    fn get(&self, needle: &str) -> Result<String, String> {
        if !self.is_unlocked() {
            return Err("vault is locked".into());
        }
        let e = self.find(needle)?;
        Ok(format!(
            "{}\nuser: {}\npass: {}",
            e.name, e.username, e.password
        ))
    }

    fn remove(&mut self, needle: &str) -> Result<String, String> {
        if !self.is_unlocked() {
            return Err("vault is locked".into());
        }
        let name = self.find(needle)?.name.clone();
        self.entries.retain(|e| e.name != name);
        self.persist()?;
        Ok(format!("removed `{name}`"))
    }

    /// Import a KeePassDX CSV export: `title,username,password` with optional
    /// quotes. Extra CSV columns are ignored.
    fn import_csv(&mut self, path: &str) -> Result<String, String> {
        if !self.is_unlocked() {
            return Err("vault is locked".into());
        }
        let path = PathBuf::from(path.trim());
        let meta = std::fs::metadata(&path).map_err(|e| format!("import: {e}"))?;
        if !meta.is_file() {
            return Err("import: not a file".into());
        }
        if meta.len() > MAX_IMPORT_BYTES {
            return Err(format!(
                "import: file is larger than {MAX_IMPORT_BYTES} bytes"
            ));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("import: {e}"))?;
        let mut added = 0usize;
        let mut skipped = 0usize;
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = parse_csv_line(line);
            if fields.len() < 3 {
                skipped += 1;
                continue;
            }
            let (title, user, pass) = (fields[0].trim(), fields[1].trim(), &fields[2]);
            // KeePassDX CSV headers and group lines are skipped.
            if i == 0
                && (title.eq_ignore_ascii_case("title") || title.eq_ignore_ascii_case("account"))
            {
                continue;
            }
            if title.is_empty() {
                skipped += 1;
                continue;
            }
            let name = truncate(title, MAX_NAME);
            if self.entries.iter().any(|e| e.name == name) {
                skipped += 1;
                continue;
            }
            if self.entries.len() >= MAX_ENTRIES {
                skipped += 1;
                continue;
            }
            self.entries.push(Entry {
                name,
                username: truncate(user, MAX_USER),
                password: truncate(pass, MAX_PASS),
            });
            added += 1;
        }
        self.entries.sort_by_key(|e| e.name.to_ascii_lowercase());
        self.persist()?;
        Ok(format!("imported {added} entries ({skipped} skipped)"))
    }

    /// Execute one command line.
    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.file.is_none() {
            return ModuleReply::err("vault storage not initialised");
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.listing());
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "init" => self.create(rest),
            "unlock" => self.unlock(rest),
            "lock" => Ok(self.lock()),
            "list" | "ls" => Ok(self.listing()),
            "add" => {
                let (name, rest) = split_head(rest);
                let (user, pass) = split_head(rest.trim());
                if name.is_empty() || user.is_empty() || pass.trim().is_empty() {
                    Err("usage: add <name> <username> <password>".into())
                } else {
                    self.add(name, user, pass.trim_start())
                }
            }
            "get" | "show" => self.get(rest),
            "rm" | "remove" | "delete" => self.remove(rest),
            "import" => self.import_csv(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Vault {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "VAULT

  init <passphrase>                        create the vault
  unlock <passphrase>                      decrypt it for this session
  lock                                     drop the key from memory
  add <name> <username> <password>         store an entry
  get <name>                               reveal username + password
  rm <name>                                delete an entry
  import <csv-path>                        KeePassDX CSV export
  list                                     entry names

Encryption: Argon2id + XChaCha20-Poly1305. The passphrase is never stored.
Note: input typed in the command bar is visible on screen; use the vault for
long-lived secrets, not for a one-time show.";

impl Module for Vault {
    fn id(&self) -> &'static str {
        "app-vault"
    }
    fn name(&self) -> &'static str {
        "Password Vault"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.file = Some(dir.join("vault.bin"));
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => {
                if self.file.is_none() {
                    return ModuleReply::err("vault storage not initialised");
                }
                ModuleReply::ok(self.listing())
            }
            "run" => self.execute(payload),
            "lock" => ModuleReply::ok(self.lock()),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Vault::new);

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

/// Split one CSV record, honouring `"quoted, fields"` and `""` escapes.
pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn unlocked(dir: &Path) -> Vault {
        let mut v = Vault::new();
        v.init(&ModuleContext {
            data_dir: dir.to_string_lossy().into(),
            host: Host::detached("app-vault"),
        })
        .unwrap();
        let r = v.handle("run", "init correct horse battery");
        assert!(r.ok, "{}", r.text);
        v
    }

    #[test]
    fn init_unlock_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::new();
        v.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-vault"),
        })
        .unwrap();
        assert!(v.handle("launch", "").text.contains("LOCKED"));
        let r = v.handle("run", "init correct horse battery");
        assert!(r.ok, "{}", r.text);
        assert!(v.handle("run", "add mail me@example.com s3cret!").ok);
        assert!(v.handle("launch", "").text.contains("mail"));

        // A fresh instance starts locked and needs the same passphrase.
        let mut second = Vault::new();
        second
            .init(&ModuleContext {
                data_dir: dir.path().to_string_lossy().into(),
                host: Host::detached("app-vault"),
            })
            .unwrap();
        assert!(!second.handle("run", "unlock wrong password").ok);
        assert!(second.handle("run", "unlock correct horse battery").ok);
        let got = second.handle("run", "get mail");
        assert!(got.ok && got.text.contains("s3cret!"), "{:?}", got);
    }

    #[test]
    fn locking_hides_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = unlocked(dir.path());
        v.handle("run", "lock");
        assert!(!v.handle("run", "list").text.contains("unlocked"));
        assert!(!v.handle("run", "get anything").ok);
        assert!(!v.handle("run", "add a b c").ok);
    }

    #[test]
    fn duplicate_and_ambiguous_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = unlocked(dir.path());
        assert!(v.handle("run", "add github me hunter2").ok);
        assert!(!v.handle("run", "add GitHub me other").ok);
        assert!(v.handle("run", "add gitlab me hunter3").ok);
        assert!(!v.handle("run", "get git").ok, "ambiguous prefix");
        assert!(v.handle("run", "get github").ok);
        assert!(v.handle("run", "rm github").ok);
        assert!(!v.handle("run", "rm github").ok);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = unlocked(dir.path());
        v.handle("run", "add bank me pw");
        let file = dir.path().join("vault.bin");
        let text = std::fs::read_to_string(&file).unwrap();
        let mut env: Envelope = serde_json::from_str(&text).unwrap();
        let mut body = hex::decode(&env.ciphertext).unwrap();
        body[0] ^= 0xff;
        env.ciphertext = hex::encode(body);
        std::fs::write(&file, serde_json::to_string(&env).unwrap()).unwrap();

        let mut second = Vault::new();
        second
            .init(&ModuleContext {
                data_dir: dir.path().to_string_lossy().into(),
                host: Host::detached("app-vault"),
            })
            .unwrap();
        assert!(!second.handle("run", "unlock correct horse battery").ok);
    }

    #[test]
    fn keeppassdx_csv_import() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = unlocked(dir.path());
        let csv = dir.path().join("export.csv");
        std::fs::write(
            &csv,
            "Title,Username,Password\n\"Bank, Inc\",alice,\"pa,ss\"\nEmpty,,x\n\"Quoted \"\"name\"\"\",bob,pw\nbadline\n",
        )
        .unwrap();
        let r = v.handle("run", &format!("import {}", csv.display()));
        assert!(r.ok, "{}", r.text);
        assert!(r.text.starts_with("imported 3"), "{}", r.text);
        let got = v.handle("run", "get Quoted \"name\"");
        assert!(got.ok, "{:?}", got);
        assert!(!v.handle("run", "import /nope/missing.csv").ok);
    }

    #[test]
    fn csv_parser() {
        assert_eq!(parse_csv_line("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(parse_csv_line("\"a,b\",c"), vec!["a,b", "c"]);
        assert_eq!(parse_csv_line("\"a\"\"b\",c"), vec!["a\"b", "c"]);
        assert_eq!(parse_csv_line("a"), vec!["a"]);
        assert_eq!(parse_csv_line(""), vec![""]);
    }

    #[test]
    fn short_passphrases_and_missing_command_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::new();
        v.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-vault"),
        })
        .unwrap();
        assert!(!v.handle("run", "init short").ok);
        assert!(!v.handle("run", "unlock whatever").ok);
        assert!(!v.handle("run", "nonsense").ok);
        assert!(!v.handle("bogus", "").ok);
    }

    #[test]
    fn vault_file_is_private_and_not_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = unlocked(dir.path());
        v.handle("run", "add secret me hunter2");
        let file = dir.path().join("vault.bin");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("hunter2"),
            "password must not be stored in the clear"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn uninitialised_module_errors() {
        let mut v = Vault::new();
        assert!(!v.handle("launch", "").ok);
        assert!(!v.handle("run", "list").ok);
    }
}
