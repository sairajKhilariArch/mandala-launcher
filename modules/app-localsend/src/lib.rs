//! C3 LocalSend: one-shot, token-guarded file transfer over the local network.
//!
//! Roadmap deviation, stated plainly: this is not the LocalSend protocol.
//! LocalSend is a full discovery + multicast + REST implementation; Mandala
//! ships a smaller direct-address protocol with the same properties that
//! matter here: nothing leaves the LAN, no account, no telemetry, and the
//! receiver must know the shared token.
//!
//! Protocol (TCP, one file per connection):
//!
//! ```text
//! sender   -> "MANDALA-LOCALSEND/1\n"
//! sender   -> {"name":"x","size":123,"sha256":"…","token":"…"}\n
//! receiver -> "OK\n" | "ERR <reason>\n"
//! sender   -> <size raw bytes>
//! receiver -> "DONE\n" | "ERR <reason>\n"
//! ```
//!
//! Actions
//! * `launch` – usage + received files
//! * `run`    – `send <path> <host:port> <token>` / `recv <port> <token>`
//!
//! `recv` binds immediately (so a busy port is an error the user sees) and
//! answers after the first connection arrives, via a `notice` message.

use mandala_module_api::{declare_module, Host, Module, ModuleContext, ModuleReply};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAGIC: &str = "MANDALA-LOCALSEND/1";
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_HEADER: u64 = 8 * 1024;
const MAX_TOKEN: usize = 64;
const MIN_TOKEN: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const HOST: &str = "host";

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Header {
    name: String,
    size: u64,
    sha256: String,
    token: String,
}

pub struct LocalSend {
    root: Option<PathBuf>,
    host: Host,
}

impl LocalSend {
    pub fn new() -> Self {
        Self {
            root: None,
            host: Host::detached("app-localsend"),
        }
    }

    fn downloads(&self) -> Result<PathBuf, String> {
        let root = self
            .root
            .as_deref()
            .ok_or_else(|| "LocalSend storage not initialised".to_string())?;
        Ok(root.join("downloads"))
    }

    fn listing(&self) -> String {
        let dir = match self.downloads() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        e.metadata()
                            .ok()
                            .filter(|m| m.is_file())
                            .map(|_| e.file_name().to_string_lossy().into_owned())
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let body = if names.is_empty() {
            "  (no received files)".to_string()
        } else {
            names
                .iter()
                .map(|n| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "LOCALSEND\n\n  recv <port> <token>               wait for one incoming file\n  send <path> <host:port> <token>   send a file\n  list                              received files\n\nReceived files land in {}:\n{body}\n\nLAN only. No discovery beacons, no accounts, no telemetry.",
            dir.display()
        )
    }

    fn start_receive(&self, port: u16) -> Result<TcpListener, String> {
        TcpListener::bind(("0.0.0.0", port))
            .map_err(|e| format!("recv: cannot listen on {port}: {e}"))
    }
}

impl Default for LocalSend {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute one command line.
pub fn execute(m: &mut LocalSend, line: &str, host: Host) -> ModuleReply {
    let line = line.trim();
    if line.is_empty() {
        return ModuleReply::ok(m.listing());
    }
    let (head, rest) = split_head(line);
    let rest = rest.trim();
    match head.to_ascii_lowercase().as_str() {
        "help" | "?" => ModuleReply::ok(HELP.to_string()),
        "list" | "ls" => ModuleReply::ok(m.listing()),
        "recv" | "receive" => {
            let (port, token) = split_head(rest);
            let token = token.trim().to_string();
            let port: u16 = match port.parse() {
                Ok(p) if p >= 1024 => p,
                _ => return ModuleReply::err("usage: recv <port 1024-65535> <token>"),
            };
            if let Err(e) = check_token(&token) {
                return ModuleReply::err(e);
            }
            let dir = match m.downloads().and_then(|d| {
                std::fs::create_dir_all(&d).map_err(|e| format!("recv: {}", e))?;
                Ok(d)
            }) {
                Ok(d) => d,
                Err(e) => return ModuleReply::err(e),
            };
            let listener = match m.start_receive(port) {
                Ok(l) => l,
                Err(e) => return ModuleReply::err(e),
            };
            std::thread::spawn(move || {
                let outcome = match accept_one(&listener, &token, &dir, IO_TIMEOUT) {
                    Ok(path) => format!("LocalSend: received {}", path.display()),
                    Err(e) => format!("LocalSend receive failed: {e}"),
                };
                // Reaches the launcher UI as a `notice` message.
                let _ = host.send(HOST, "notice", &ModuleReply::ok(outcome).to_json());
            });
            ModuleReply::ok(format!(
                "listening on port {port} — waiting for one file (token required)"
            ))
        }
        "send" | "push" => {
            let mut parts = rest.split_whitespace();
            let (path, addr, token) = (parts.next(), parts.next(), parts.next());
            match (path, addr, token) {
                (Some(path), Some(addr), Some(token)) => {
                    match send_file(Path::new(path), addr, token, MAX_FILE) {
                        Ok(msg) => ModuleReply::ok(msg),
                        Err(e) => ModuleReply::err(e),
                    }
                }
                _ => ModuleReply::err("usage: send <path> <host:port> <token>"),
            }
        }
        other => ModuleReply::err(format!("unknown command `{other}` (try `help`)")),
    }
}

const HELP: &str = "LOCALSEND

  recv <port> <token>               listen and accept one file
  send <path> <host:port> <token>   push a file to a listener
  list                              show received files

Tokens must be 4-64 characters and must match on both ends. Files are
verified with SHA-256 and capped at 64 MiB. Nothing is uploaded anywhere.";

fn check_token(token: &str) -> Result<(), String> {
    if token.len() < MIN_TOKEN || token.len() > MAX_TOKEN {
        return Err(format!("token must be {MIN_TOKEN}-{MAX_TOKEN} characters"));
    }
    Ok(())
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// Basename only: no separators, no `.`/`..`, bounded length.
pub fn sanitize_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .take(128)
        .collect();
    let cleaned = cleaned.trim_matches('.').trim();
    if cleaned.is_empty() {
        "received.bin".to_string()
    } else {
        cleaned.to_string()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Send one file to `addr` (`host:port`). Blocking; called on the blocking pool.
pub fn send_file(path: &Path, addr: &str, token: &str, max: u64) -> Result<String, String> {
    check_token(token)?;
    let meta = std::fs::metadata(path).map_err(|e| format!("send: {e}"))?;
    if !meta.is_file() {
        return Err("send: not a regular file".into());
    }
    if meta.len() > max {
        return Err(format!("send: file is larger than {max} bytes"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("send: {e}"))?;
    let name = sanitize_name(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let header = Header {
        name: name.clone(),
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        token: token.to_string(),
    };

    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("send: bad address `{addr}`: {e}"))?
        .next()
        .ok_or_else(|| format!("send: `{addr}` resolved to nothing"))?;
    let mut stream =
        TcpStream::connect_timeout(&sock_addr, IO_TIMEOUT).map_err(|e| format!("send: {e}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("send: {e}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("send: {e}"))?;

    let mut out = String::new();
    out.push_str(MAGIC);
    out.push('\n');
    out.push_str(&serde_json::to_string(&header).map_err(|e| format!("send: {e}"))?);
    out.push('\n');
    stream
        .write_all(out.as_bytes())
        .map_err(|e| format!("send: {e}"))?;
    stream.flush().map_err(|e| format!("send: {e}"))?;

    let mut reader = BufReader::new(stream.try_clone().map_err(|e| format!("send: {e}"))?);
    match read_line(&mut reader)? {
        l if l.starts_with("OK") => {}
        l => {
            return Err(format!(
                "send: receiver refused: {}",
                l.trim().trim_start_matches("ERR").trim()
            ))
        }
    }

    stream.write_all(&bytes).map_err(|e| format!("send: {e}"))?;
    stream.flush().map_err(|e| format!("send: {e}"))?;

    match read_line(&mut reader)? {
        l if l.starts_with("DONE") => Ok(format!("sent {name} ({} bytes) to {addr}", bytes.len())),
        l => Err(format!("send: receiver reported: {}", l.trim())),
    }
}

fn read_line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let n = reader
        .take(MAX_HEADER)
        .read_line(&mut line)
        .map_err(|e| format!("io: {e}"))?;
    if n == 0 {
        return Err("connection closed".into());
    }
    Ok(line)
}

/// Accept exactly one connection, verify it and store the file. Blocking.
pub fn accept_one(
    listener: &TcpListener,
    token: &str,
    dir: &Path,
    timeout: Duration,
) -> Result<PathBuf, String> {
    let (mut stream, peer) = listener.accept().map_err(|e| format!("accept: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("socket: {e}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| format!("socket: {e}"))?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| format!("socket: {e}"))?);

    let magic = read_line(&mut reader)?;
    if magic.trim() != MAGIC {
        let _ = stream.write_all(b"ERR unsupported protocol\n");
        return Err(format!("{peer} spoke an unknown protocol"));
    }
    let header_line = read_line(&mut reader)?;
    let header: Header =
        serde_json::from_str(&header_line).map_err(|_| "malformed header".to_string())?;
    if header.token != token {
        let _ = stream.write_all(b"ERR bad token\n");
        return Err(format!("{peer} presented a wrong token"));
    }
    if header.size > MAX_FILE {
        let _ = stream.write_all(b"ERR file too large\n");
        return Err("file too large".into());
    }
    stream
        .write_all(b"OK\n")
        .map_err(|e| format!("socket: {e}"))?;

    std::fs::create_dir_all(dir).map_err(|e| format!("storage: {e}"))?;
    let name = sanitize_name(&header.name);
    let mut target = dir.join(&name);
    if target.exists() {
        let stem = target
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "received".into());
        let ext = target
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        target = dir.join(format!("{stem}-1{ext}"));
    }
    let part = dir.join(format!(".incoming-{}", std::process::id()));

    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut file = std::fs::File::create(&part).map_err(|e| format!("storage: {e}"))?;
    let mut buf = [0u8; 64 * 1024];
    while written < header.size {
        let want = (header.size - written).min(buf.len() as u64) as usize;
        let n = reader
            .read(&mut buf[..want])
            .map_err(|e| format!("receive: {e}"))?;
        if n == 0 {
            let _ = std::fs::remove_file(&part);
            let _ = stream.write_all(b"ERR truncated\n");
            return Err("sender disconnected mid-file".into());
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("storage: {e}"))?;
        written += n as u64;
    }
    drop(file);

    let digest = hex::encode(hasher.finalize());
    if digest != header.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&part);
        let _ = stream.write_all(b"ERR checksum mismatch\n");
        return Err("checksum mismatch".into());
    }
    std::fs::rename(&part, &target).map_err(|e| format!("storage: {e}"))?;
    let _ = stream.write_all(b"DONE\n");
    Ok(target)
}

impl Module for LocalSend {
    fn id(&self) -> &'static str {
        "app-localsend"
    }
    fn name(&self) -> &'static str {
        "LocalSend"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(dir.join("downloads")).map_err(|e| e.to_string())?;
        self.root = Some(dir);
        self.host = ctx.host;
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" if self.root.is_some() => ModuleReply::ok(self.listing()),
            "launch" => ModuleReply::err("LocalSend storage not initialised"),
            "run" => execute(self, payload, self.host),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(LocalSend::new);

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> (tempfile::TempDir, LocalSend) {
        let dir = tempfile::tempdir().unwrap();
        let mut m = LocalSend::new();
        m.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-localsend"),
        })
        .unwrap();
        (dir, m)
    }

    #[test]
    fn name_sanitising_blocks_traversal() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\Windows\\evil.dll"), "evil.dll");
        assert_eq!(sanitize_name(".."), "received.bin");
        assert_eq!(sanitize_name(""), "received.bin");
        assert_eq!(sanitize_name("  report.pdf  "), "report.pdf");
        assert!(!sanitize_name("a\u{0}b").contains('\u{0}'));
    }

    fn roundtrip(
        token_send: &str,
        token_recv: &str,
        payload: &[u8],
        name: &str,
    ) -> (tempfile::TempDir, Result<String, String>) {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let file = src.path().join(name);
        std::fs::write(&file, payload).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let out_dir = dst.path().to_path_buf();
        let recv_token = token_recv.to_string();
        let handle = std::thread::spawn(move || {
            accept_one(&listener, &recv_token, &out_dir, Duration::from_secs(5))
        });
        let sent = send_file(&file, &format!("127.0.0.1:{port}"), token_send, MAX_FILE);
        let received = handle.join().unwrap();
        match (sent, received) {
            (Ok(s), Ok(_path)) => (dst, Ok(s)),
            (Err(e), _) => (dst, Err(e)),
            (_, Err(e)) => (dst, Err(e)),
        }
    }

    #[test]
    fn transfer_roundtrip_verifies_and_lands_in_downloads() {
        let payload = b"hello local network".repeat(100);
        let (dst, result) = roundtrip("shared-token", "shared-token", &payload, "notes.txt");
        assert!(result.is_ok(), "{result:?}");
        let saved = dst.path().join("notes.txt");
        assert_eq!(std::fs::read(&saved).unwrap(), payload);
    }

    #[test]
    fn wrong_token_is_refused_and_nothing_is_written() {
        let (dst, result) = roundtrip("wrong-token", "right-token", b"secret", "x.bin");
        assert!(result.is_err());
        assert!(
            std::fs::read_dir(dst.path()).unwrap().next().is_none(),
            "no partial file may remain"
        );
    }

    #[test]
    fn checksum_mismatch_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            accept_one(&listener, "tok!", &out_dir, Duration::from_secs(5))
        });
        // Hand-rolled sender with a lying checksum.
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let header = Header {
            name: "a.bin".into(),
            size: 3,
            sha256: sha256_hex(b"zzz"),
            token: "tok!".into(),
        };
        writeln!(s, "{MAGIC}").unwrap();
        writeln!(s, "{}", serde_json::to_string(&header).unwrap()).unwrap();
        let mut line = String::new();
        BufReader::new(s.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "OK");
        s.write_all(b"abc").unwrap();
        let err = handle.join().unwrap().unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(
            std::fs::read_dir(dir.path()).unwrap().next().is_none(),
            "partial file must be removed"
        );
    }

    #[test]
    fn command_surface() {
        let (_d, mut m) = ready();
        assert!(m.handle("launch", "").text.contains("LOCALSEND"));
        assert!(!m.handle("run", "send").ok);
        assert!(!m.handle("run", "send /nope 127.0.0.1:1 token").ok);
        assert!(
            !m.handle("run", "recv 80 short").ok,
            "port and token validated"
        );
        assert!(!m.handle("run", "recv 40000 123").ok, "token too short");
        assert!(!m.handle("run", "wat").ok);
        assert!(!m.handle("explode", "").ok);
    }

    #[test]
    fn token_validation() {
        assert!(check_token("abc").is_err());
        assert!(check_token("abcd").is_ok());
        assert!(check_token(&"x".repeat(MAX_TOKEN + 1)).is_err());
    }

    #[test]
    fn uninitialised_module_errors() {
        let m = LocalSend::new();
        assert!(m.downloads().is_err());
    }
}
