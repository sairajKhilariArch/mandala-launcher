//! C1 2FA Authenticator: offline TOTP (RFC 6238) with `otpauth://` import.
//!
//! This is the in-process counterpart of an Aegis-style authenticator: it
//! imports the same `otpauth://` URIs that Aegis exports, keeps the accounts
//! on the device and never talks to the network.
//!
//! Actions
//! * `launch` – list accounts with their current codes
//! * `run`    – one command line, e.g. `add otpauth://totp/GitHub:me?secret=...`
//!
//! Storage: `<home>/data/app-aegis/accounts.json`, mode 0600. Secrets are
//! stored unencrypted (the launcher has no unlock ceremony yet); the file is
//! only as private as the Mandala home directory.

use hmac::{Hmac, Mac};
use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SECRET_CHARS: usize = 256;
const DEFAULT_DIGITS: u32 = 6;
const DEFAULT_PERIOD: u64 = 30;

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha512 = Hmac<Sha512>;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum Algorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "SHA1" => Some(Self::Sha1),
            "SHA256" => Some(Self::Sha256),
            "SHA512" => Some(Self::Sha512),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub issuer: String,
    pub account: String,
    /// Base32, as used by every authenticator app.
    pub secret: String,
    #[serde(default = "default_digits")]
    pub digits: u32,
    #[serde(default = "default_period")]
    pub period: u64,
    #[serde(default)]
    pub algorithm: Algorithm,
}

fn default_digits() -> u32 {
    DEFAULT_DIGITS
}
fn default_period() -> u64 {
    DEFAULT_PERIOD
}

impl Account {
    fn label(&self) -> String {
        if self.issuer.is_empty() {
            self.account.clone()
        } else if self.account.is_empty() {
            self.issuer.clone()
        } else {
            format!("{} ({})", self.issuer, self.account)
        }
    }

    fn secret_bytes(&self) -> Result<Vec<u8>, String> {
        base32_decode(&self.secret)
    }

    /// Code at `unix` seconds plus seconds left in the window.
    pub fn code_at(&self, unix: u64) -> Result<(String, u64), String> {
        let secret = self.secret_bytes()?;
        let counter = unix / self.period;
        let code = hotp(&secret, counter, self.digits, self.algorithm);
        let left = self.period - (unix % self.period);
        Ok((code, left))
    }
}

pub struct Aegis {
    file: Option<PathBuf>,
    accounts: Vec<Account>,
    warning: Option<String>,
}

impl Aegis {
    pub fn new() -> Self {
        Self {
            file: None,
            accounts: Vec::new(),
            warning: None,
        }
    }

    fn path(&self) -> Result<&Path, String> {
        self.file
            .as_deref()
            .ok_or_else(|| "2FA storage not initialised".to_string())
    }

    fn load(file: &Path) -> (Vec<Account>, Option<String>) {
        match std::fs::read_to_string(file) {
            Ok(text) if text.trim().is_empty() => (Vec::new(), None),
            Ok(text) => match serde_json::from_str::<Vec<Account>>(&text) {
                Ok(a) => (a, None),
                Err(e) => (Vec::new(), Some(format!("could not read accounts: {e}"))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(e) => (Vec::new(), Some(format!("could not read accounts: {e}"))),
        }
    }

    fn save(&self) -> Result<(), String> {
        let path = self.path()?;
        let json =
            serde_json::to_string_pretty(&self.accounts).map_err(|e| format!("save: {e}"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("save: {e}"))?;
        }
        write_private(path, json.as_bytes()).map_err(|e| format!("save: {e}"))
    }

    fn listing(&self) -> String {
        let mut out = String::from("AEGIS 2FA\n\n");
        if let Some(w) = &self.warning {
            out.push_str(&format!("! {w}\n\n"));
        }
        if self.accounts.is_empty() {
            out.push_str("No accounts yet.\n\nadd otpauth://totp/Issuer:me?secret=BASE32...\naddraw <issuer> <account> <base32-secret>");
            return out;
        }
        let now = now_secs();
        for (i, a) in self.accounts.iter().enumerate() {
            match a.code_at(now) {
                Ok((code, left)) => {
                    out.push_str(&format!(
                        "{:>2}. {:<28} {}  {:>2}s\n",
                        i + 1,
                        truncate(&a.label(), 28),
                        group(&code),
                        left
                    ));
                }
                Err(e) => out.push_str(&format!(
                    "{:>2}. {:<28} ! {e}\n",
                    i + 1,
                    truncate(&a.label(), 28)
                )),
            }
        }
        out.push_str("\nCodes are generated on-device; nothing leaves the phone.");
        out
    }

    fn find(&self, needle: &str) -> Vec<usize> {
        let q = needle.trim().to_ascii_lowercase();
        if q.is_empty() {
            return (0..self.accounts.len()).collect();
        }
        self.accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.label().to_ascii_lowercase().contains(&q) || a.issuer.to_ascii_lowercase() == q
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn add(&mut self, account: Account) -> Result<(), String> {
        if account.secret_bytes().is_err() {
            return Err("secret is not valid base32".into());
        }
        if self.accounts.iter().any(|a| {
            a.issuer.eq_ignore_ascii_case(&account.issuer)
                && a.account.eq_ignore_ascii_case(&account.account)
        }) {
            return Err(format!("`{}` is already registered", account.label()));
        }
        self.accounts.push(account);
        self.warning = None;
        self.save()
    }

    fn remove(&mut self, needle: &str) -> Result<String, String> {
        let hits = self.find(needle);
        match hits.len() {
            0 => Err(format!("no account matches `{needle}`")),
            1 => {
                let removed = self.accounts.remove(hits[0]);
                self.save()?;
                Ok(format!("removed {}", removed.label()))
            }
            _ => Err(format!(
                "`{needle}` matches {} accounts; be more specific",
                hits.len()
            )),
        }
    }

    fn show_code(&self, needle: &str) -> Result<String, String> {
        let hits = self.find(needle);
        let idx = match hits.len() {
            0 => return Err(format!("no account matches `{needle}`")),
            1 => hits[0],
            _ => {
                let names: Vec<String> = hits
                    .iter()
                    .map(|&i| format!("  - {}", self.accounts[i].label()))
                    .collect();
                return Err(format!(
                    "`{needle}` matches several accounts:\n{}",
                    names.join("\n")
                ));
            }
        };
        let (code, left) = self.accounts[idx].code_at(now_secs())?;
        Ok(format!(
            "{}\n{}   ({}s left)",
            self.accounts[idx].label(),
            group(&code),
            left
        ))
    }

    /// Execute one terminal-style command.
    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.file.is_none() {
            return ModuleReply::err("2FA storage not initialised");
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.listing());
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "list" | "ls" => Ok(self.listing()),
            "add" => parse_otpauth(rest).and_then(|a| self.add(a)).map(|_| {
                format!(
                    "added {}",
                    self.accounts.last().map(|a| a.label()).unwrap_or_default()
                )
            }),
            "addraw" => {
                let mut parts = rest.split_whitespace();
                let (issuer, account, secret) = (parts.next(), parts.next(), parts.next());
                match (issuer, account, secret) {
                    (Some(issuer), Some(account), Some(secret)) => {
                        let acct = Account {
                            issuer: issuer.into(),
                            account: account.into(),
                            secret: secret.into(),
                            digits: DEFAULT_DIGITS,
                            period: DEFAULT_PERIOD,
                            algorithm: Algorithm::Sha1,
                        };
                        self.add(acct).map(|_| format!("added {issuer}"))
                    }
                    _ => Err("usage: addraw <issuer> <account> <base32-secret>".into()),
                }
            }
            "rm" | "remove" | "delete" => self.remove(rest),
            "code" | "show" => self.show_code(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Aegis {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "AEGIS 2FA\n\n  list                 accounts and current codes\n  code <name>          one account's code\n  add <otpauth://...>  import a URI (Aegis/Google exports)\n  addraw <issuer> <account> <secret>\n  rm <name>            delete an account\n\nCodes are computed on-device. No network access.";

impl Module for Aegis {
    fn id(&self) -> &'static str {
        "app-aegis"
    }
    fn name(&self) -> &'static str {
        "2FA Authenticator"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("accounts.json");
        let (accounts, warning) = Aegis::load(&file);
        self.accounts = accounts;
        self.warning = warning;
        self.file = Some(file);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => {
                if self.file.is_none() {
                    return ModuleReply::err("2FA storage not initialised");
                }
                ModuleReply::ok(self.listing())
            }
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Aegis::new);

// ---------------------------------------------------------------------------
// TOTP / base32 primitives
// ---------------------------------------------------------------------------

/// RFC 4226 HOTP: `HOTP(secret, counter)` truncated to `digits` digits.
pub fn hotp(secret: &[u8], counter: u64, digits: u32, algorithm: Algorithm) -> String {
    let message = counter.to_be_bytes();
    let mac: Vec<u8> = match algorithm {
        Algorithm::Sha1 => hmac_sha1(secret, &message),
        Algorithm::Sha256 => hmac_sha256(secret, &message),
        Algorithm::Sha512 => hmac_sha512(secret, &message),
    };
    let offset = (mac[mac.len() - 1] & 0x0f) as usize;
    let binary = ((mac[offset] as u32 & 0x7f) << 24)
        | ((mac[offset + 1] as u32) << 16)
        | ((mac[offset + 2] as u32) << 8)
        | (mac[offset + 3] as u32);
    let digits = digits.clamp(1, 10);
    let modulus = 10u64.pow(digits);
    format!(
        "{:0width$}",
        binary as u64 % modulus,
        width = digits as usize
    )
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha512(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

/// Decode RFC 4648 base32 (case-insensitive, spaces and `=` ignored).
pub fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut count = 0usize;
    for c in input.chars() {
        if c == '=' || c.is_whitespace() || c == '-' {
            continue;
        }
        let v = match c.to_ascii_uppercase() {
            'A'..='Z' => c.to_ascii_uppercase() as u8 - b'A',
            '2'..='7' => c as u8 - b'2' + 26,
            _ => return Err(format!("invalid base32 character `{c}`")),
        };
        bits = (bits << 5) | v as u32;
        nbits += 5;
        count += 1;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    if count == 0 {
        return Err("empty base32 secret".into());
    }
    if nbits >= 5 {
        return Err("truncated base32 secret".into());
    }
    Ok(out)
}

/// Encode to base32 without padding (for tests and round-tripping).
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    for &b in bytes {
        bits = (bits << 8) | b as u32;
        nbits += 8;
        while nbits >= 5 {
            nbits -= 5;
            out.push(ALPHABET[((bits >> nbits) & 0x1f) as usize] as char);
        }
    }
    if nbits > 0 {
        out.push(ALPHABET[((bits << (5 - nbits)) & 0x1f) as usize] as char);
    }
    out
}

/// Parse an `otpauth://totp/...` URI (Aegis, Google Authenticator, ...).
pub fn parse_otpauth(uri: &str) -> Result<Account, String> {
    let uri = uri.trim();
    if uri.to_ascii_lowercase().starts_with("otpauth://hotp/") {
        return Err("counter-based HOTP URIs are not supported".into());
    }
    let rest = uri
        .strip_prefix("otpauth://totp/")
        .or_else(|| uri.strip_prefix("otpauth://TOTP/"))
        .ok_or_else(|| "expected an otpauth://totp/ URI".to_string())?;
    let (label, query) = match rest.split_once('?') {
        Some((l, q)) => (l, q),
        None => (rest, ""),
    };
    let label = percent_decode(label);
    let (mut issuer, account) = match label.split_once(':') {
        Some((i, a)) => (i.trim().to_string(), a.trim().to_string()),
        None => (String::new(), label.trim().to_string()),
    };

    let mut secret = None;
    let mut digits = DEFAULT_DIGITS;
    let mut period = DEFAULT_PERIOD;
    let mut algorithm = Algorithm::Sha1;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(v.trim().to_string()),
            "issuer" if !v.trim().is_empty() => issuer = v.trim().to_string(),
            "digits" => {
                digits = v
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "invalid digits".to_string())?
            }
            "period" => {
                period = v
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "invalid period".to_string())?
            }
            "algorithm" => {
                algorithm =
                    Algorithm::parse(&v).ok_or_else(|| format!("unsupported algorithm `{v}`"))?
            }
            _ => {}
        }
    }
    let secret = secret.ok_or_else(|| "otpauth URI has no `secret` parameter".to_string())?;
    if secret.len() > MAX_SECRET_CHARS {
        return Err("secret is too long".into());
    }
    if account.is_empty() {
        return Err("otpauth URI has no account label".into());
    }
    if digits == 0 || digits > 10 {
        return Err("digits must be between 1 and 10".into());
    }
    if period == 0 || period > 3600 {
        return Err("period must be between 1 and 3600 seconds".into());
    }
    let account = Account {
        issuer,
        account,
        secret,
        digits,
        period,
        algorithm,
    };
    account.secret_bytes()?;
    Ok(account)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

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
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// `123456` -> `123 456` so codes are easier to read out.
fn group(code: &str) -> String {
    let mid = code.len() / 2;
    if code.len() >= 6 {
        format!("{} {}", &code[..mid], &code[mid..])
    } else {
        code.to_string()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

    fn ready() -> (tempfile::TempDir, Aegis) {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Aegis::new();
        m.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-aegis"),
        })
        .unwrap();
        (dir, m)
    }

    #[test]
    fn rfc6238_sha1_vectors() {
        let secret = b"12345678901234567890";
        let vectors = [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ];
        for (t, want) in vectors {
            assert_eq!(hotp(secret, t / 30, 8, Algorithm::Sha1), want, "t={t}");
        }
    }

    #[test]
    fn rfc6238_sha256_vectors() {
        let secret = b"12345678901234567890123456789012";
        let vectors = [
            (59, "46119246"),
            (1_111_111_109, "68084774"),
            (2_000_000_000, "90698825"),
        ];
        for (t, want) in vectors {
            assert_eq!(hotp(secret, t / 30, 8, Algorithm::Sha256), want, "t={t}");
        }
    }

    #[test]
    fn base32_roundtrip_and_errors() {
        for raw in [
            b"a".as_slice(),
            b"abcd",
            b"12345678901234567890",
            &[0xff, 0x00, 0x7f],
        ] {
            assert_eq!(base32_decode(&base32_encode(raw)).unwrap(), raw);
        }
        assert_eq!(
            base32_decode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap(),
            b"12345678901234567890"
        );
        // lower case, spaces and padding are tolerated
        assert_eq!(
            base32_decode("gezd gnbv gy3t qojq====").unwrap(),
            b"1234567890"
        );
        assert!(base32_decode("MZXW6YQ!").is_err());
        assert!(base32_decode("").is_err());
        assert!(base32_decode("A").is_err(), "5 leftover bits is truncated");
    }

    #[test]
    fn otpauth_uri_parsing() {
        let a = parse_otpauth("otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&period=60&digits=8&algorithm=SHA256").unwrap();
        assert_eq!(a.issuer, "GitHub");
        assert_eq!(a.account, "octocat");
        assert_eq!(a.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(a.digits, 8);
        assert_eq!(a.period, 60);
        assert_eq!(a.algorithm, Algorithm::Sha256);
        // Percent-encoded label and issuer
        let b = parse_otpauth("otpauth://totp/My%20Bank:me%40example.com?secret=JBSWY3DPEHPK3PXP")
            .unwrap();
        assert_eq!(b.issuer, "My Bank");
        assert_eq!(b.account, "me@example.com");
        // Errors
        assert!(parse_otpauth("otpauth://hotp/x?secret=AA").is_err());
        assert!(parse_otpauth("https://example.com").is_err());
        assert!(parse_otpauth("otpauth://totp/x?secret=%%%").is_err());
        assert!(parse_otpauth("otpauth://totp/x").is_err());
        assert!(parse_otpauth("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=99").is_err());
    }

    #[test]
    fn add_list_code_remove_and_persist() {
        let (dir, mut m) = ready();
        assert!(m.handle("launch", "").text.contains("No accounts"));
        let out = m.handle(
            "run",
            "add otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP&issuer=GitHub",
        );
        assert!(out.ok, "{}", out.text);
        let listing = m.handle("launch", "").text;
        assert!(listing.contains("GitHub (octocat)"), "{listing}");
        let code = m.handle("run", "code github");
        assert!(code.ok && code.text.contains("GitHub"), "{:?}", code);
        // duplicate and ambiguous handling
        assert!(
            !m.handle(
                "run",
                "add otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP"
            )
            .ok
        );
        assert!(!m.handle("run", "code nobody").ok);
        assert!(m.handle("run", "rm github").ok);
        assert!(m.handle("launch", "").text.contains("No accounts"));

        // Persistence: a second instance re-reads the file.
        let mut second = Aegis::new();
        second
            .init(&ModuleContext {
                data_dir: dir.path().to_string_lossy().into(),
                host: Host::detached("app-aegis"),
            })
            .unwrap();
        assert!(second.handle("launch", "").text.contains("No accounts"));
    }

    #[test]
    fn addraw_and_bad_input() {
        let (_d, mut m) = ready();
        assert!(m.handle("run", "addraw Proton me JBSWY3DPEHPK3PXP").ok);
        assert!(!m.handle("run", "addraw only-one").ok);
        assert!(!m.handle("run", "addraw X y not-base32!").ok);
        assert!(!m.handle("run", "wat").ok);
        assert!(!m.handle("nope", "").ok);
    }

    #[test]
    fn uninitialised_module_errors() {
        let mut m = Aegis::new();
        assert!(!m.handle("launch", "").ok);
        assert!(!m.handle("run", "list").ok);
    }

    #[test]
    fn corrupt_file_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("accounts.json"), "{ not json").unwrap();
        let mut m = Aegis::new();
        m.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-aegis"),
        })
        .unwrap();
        assert!(m
            .handle("launch", "")
            .text
            .contains("could not read accounts"));
    }

    #[test]
    fn code_grouping() {
        assert_eq!(group("123456"), "123 456");
        assert_eq!(group("12345678"), "1234 5678");
        assert_eq!(group("123"), "123");
    }
}
