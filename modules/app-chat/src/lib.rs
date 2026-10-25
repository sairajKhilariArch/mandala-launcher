//! A4 Secure Chat: a minimal Matrix client over the public client-server API.
//!
//! Matrix was chosen because it is the design's "encrypted, open protocol"
//! requirement met by an actually decentralised network: any homeserver works
//! (Synapse, Conduit, your own), rooms are E2EE by default in modern clients,
//! and no vendor account is needed. The module speaks plain HTTP(S) with
//! `ureq` — no SDK, no telemetry, no push connection: it polls only when you
//! ask, so the radio is idle unless you use it.
//!
//! Actions
//! * `launch`    – sync once and show rooms / unread-ish latest messages
//! * `run`       – `login <homeserver> <user> [password]` ·
//!   `token <homeserver> <user> [token]` · `rooms` · `read <n|room>` ·
//!   `say <room|me> <text>` · `logout` · `status`
//!
//! Secrets are command arguments or `MANDALA_CHAT_PASSWORD` /
//! `MANDALA_CHAT_TOKEN`; the module never reads a terminal, so it can run on
//! the launcher's UI path without blocking.
//!
//! Credentials live in `<data>/session.json` at mode 0600 and are never
//! logged. Homeserver traffic is plain HTTPS-by-convention: the module does
//! not downgrade to `http://` on its own.
//!
//! This is deliberately not a full client — no media, no E2EE key management,
//! no notifications. It is the bridge the roadmap asks for: read and send
//! messages from the launcher, everything else stays in your existing client.

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HISTORY: usize = 25;
const PASSWORD_ENV: &str = "MANDALA_CHAT_PASSWORD";
const TOKEN_ENV: &str = "MANDALA_CHAT_TOKEN";

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Session {
    homeserver: String,
    user: String,
    user_id: String,
    access_token: String,
}

#[derive(Debug, Clone)]
struct Room {
    id: String,
    name: String,
    last_body: String,
    last_sender: String,
}

pub struct Chat {
    /// Private module directory; the session file lives here. Kept as state
    /// rather than an env var so two instances (tests, a restarted host)
    /// never share a credential file.
    data_dir: PathBuf,
    session: Option<Session>,
    rooms: Vec<Room>,
    since: String,
    txn: u64,
}

impl Chat {
    pub fn new() -> Self {
        Self { data_dir: PathBuf::new(), session: None, rooms: Vec::new(), since: String::new(), txn: 0 }
    }

    /// `GET <hs>/_matrix/client/v3/...` with the bearer token.
    fn get(&self, hs: &str, path: &str, token: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/_matrix/client/v3/{}", hs.trim_end_matches('/'), path.trim_start_matches('/'));
        let body = http(&url, "GET", Some(token), None, MAX_BODY)?;
        serde_json::from_str(&body).map_err(|e| format!("homeserver response: {e}"))
    }

    /// `PUT` a JSON body, returning the parsed response.
    fn put(&self, hs: &str, path: &str, token: &str, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
        let url = format!("{}/_matrix/client/v3/{}", hs.trim_end_matches('/'), path.trim_start_matches('/'));
        let body = http(&url, "PUT", Some(token), Some(payload), MAX_BODY)?;
        serde_json::from_str(&body).map_err(|e| format!("homeserver response: {e}"))
    }

    fn post(&self, hs: &str, path: &str, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
        let url = format!("{}/_matrix/client/v3/{}", hs.trim_end_matches('/'), path.trim_start_matches('/'));
        let body = http(&url, "POST", None, Some(payload), MAX_BODY)?;
        serde_json::from_str(&body).map_err(|e| format!("homeserver response: {e}"))
    }

    fn login(&mut self, homeserver: &str, user: &str, password: &str) -> Result<String, String> {
        let hs = normalize_homeserver(homeserver)?;
        if user.trim().is_empty() || password.is_empty() {
            return Err("usage: login <homeserver-url> <user>".into());
        }
        let payload = serde_json::json!({
            "type": "m.login.password",
            "identifier": { "type": "m.id.user", "user": user.trim() },
            "password": password,
            "initial_device_display_name": "Mandala launcher",
        });
        let resp = self.post(&hs, "login", &payload)?;
        let token = resp["access_token"].as_str().ok_or("login response had no access_token")?.to_string();
        let user_id = resp["user_id"].as_str().unwrap_or_default().to_string();
        let device_id = resp["device_id"].as_str().unwrap_or_default().to_string();
        let session = Session { homeserver: hs, user: user.trim().to_string(), user_id: user_id.clone(), access_token: token };
        save_session(&self.data_dir, &session)?;
        self.session = Some(session);
        self.since.clear();
        Ok(format!("logged in as {user_id} (device {device_id})\ncredentials saved to session.json (mode 0600).\nUse `rooms` next."))
    }

    fn logout(&mut self) -> Result<String, String> {
        let session = self.session.clone().ok_or("not logged in")?;
        let _ = self.get(&session.homeserver, "logout", &session.access_token);
        clear_session(&self.data_dir)?;
        self.session = None;
        self.rooms.clear();
        self.since.clear();
        Ok("logged out; local session removed.".into())
    }

    /// One `/sync` poll, refreshing the cached room list.
    fn sync_once(&mut self) -> Result<(), String> {
        let session = self.session.as_ref().ok_or("not logged in")?.clone();
        let since = if self.since.is_empty() { String::new() } else { format!("&since={}", urlencode(&self.since)) };
        let path = format!("sync?timeout=0&set_presence=offline{since}");
        let resp = self.get(&session.homeserver, &path, &session.access_token)?;
        if let Some(next) = resp["next_batch"].as_str() {
            self.since = next.to_string();
        }
        let mut rooms = Vec::new();
        if let Some(joined) = resp["rooms"]["join"].as_object() {
            for (room_id, data) in joined {
                // Last `m.room.name` event wins; unnamed rooms fall back to
                // their id, which is what every Matrix client shows too.
                let name = data["state"]["events"]
                    .as_array()
                    .and_then(|events| {
                        events.iter().rev().find_map(|e| {
                            if e["type"] != "m.room.name" {
                                return None;
                            }
                            e["content"]["name"].as_str().filter(|n| !n.trim().is_empty()).map(str::to_string)
                        })
                    })
                    .unwrap_or_else(|| room_id.clone());
                let (last_body, last_sender) = timeline_latest(&data["timeline"]);
                rooms.push(Room { id: room_id.clone(), name, last_body, last_sender });
            }
        }
        rooms.sort_by_key(|r| r.name.to_lowercase());
        self.rooms = rooms;
        Ok(())
    }

    fn rooms(&mut self) -> Result<String, String> {
        self.sync_once()?;
        if self.rooms.is_empty() {
            return Ok("No joined rooms (or none visible to this token).".into());
        }
        let mut out = format!("ROOMS ({} — polled just now)\n\n", self.rooms.len());
        for (i, r) in self.rooms.iter().enumerate() {
            let preview = if r.last_body.is_empty() { String::new() } else { format!(" — {}: {}", short(&r.last_sender), clip(&r.last_body, 40)) };
            out.push_str(&format!("{:>2}. {}{} [{}]\n", i + 1, clip(&r.name, 24), preview, clip(&r.id, 12)));
        }
        out.push_str("\nread <n>   history of room n\nsay <n> <text>   send as you\n");
        Ok(out)
    }

    fn resolve_room(&self, arg: &str) -> Option<&Room> {
        let arg = arg.trim();
        if let Ok(n) = arg.parse::<usize>() {
            return self.rooms.get(n.saturating_sub(1));
        }
        self.rooms
            .iter()
            .find(|r| r.id == arg || r.id.eq_ignore_ascii_case(arg) || r.name.eq_ignore_ascii_case(arg))
    }

    fn read(&mut self, arg: &str) -> Result<String, String> {
        if self.rooms.is_empty() {
            self.sync_once()?;
        }
        let session = self.session.clone().ok_or("not logged in")?;
        let arg = arg.trim();
        if arg.is_empty() {
            return Err("usage: read <n>".into());
        }
        let room = self.resolve_room(arg).ok_or_else(|| format!("no room `{arg}` — run `rooms` first"))?.clone();
        let path = format!("rooms/{}/messages?dir=b&limit={}", urlencode(&room.id), MAX_HISTORY);
        let resp = self.get(&session.homeserver, &path, &session.access_token)?;
        let mut lines = Vec::new();
        if let Some(chunk) = resp["chunk"].as_array() {
            for ev in chunk {
                if ev["type"] != "m.room.message" {
                    continue;
                }
                if ev["content"]["msgtype"] != "m.text" && ev["content"]["msgtype"] != "m.notice" {
                    let kind = ev["content"]["msgtype"].as_str().unwrap_or("attachment");
                    lines.push(format!("{:>16}: [{}] (not rendered)", "", clip(kind, 20)));
                    continue;
                }
                let body = ev["content"]["body"].as_str().unwrap_or_default();
                let sender = short(ev["sender"].as_str().unwrap_or("?"));
                lines.push(format!("{:>16}: {}", sender, body));
            }
        }
        lines.reverse();
        Ok(format!("HISTORY — {} ({})\n\n{}\n", room.name, clip(&room.id, 12), lines.join("\n")))
    }

    fn say(&mut self, arg: &str) -> Result<String, String> {
        if self.rooms.is_empty() {
            self.sync_once()?;
        }
        let session = self.session.clone().ok_or("not logged in")?;
        let arg = arg.trim();
        let (who, text) = arg.split_once(char::is_whitespace).ok_or("usage: say <n|me> <text>")?;
        let text = text.trim();
        if text.is_empty() {
            return Err("nothing to send".into());
        }
        let room_id = if who == "me" {
            self.rooms.first().ok_or("no rooms — run `rooms` first")?.id.clone()
        } else {
            self.resolve_room(who).ok_or_else(|| format!("no room `{who}` — run `rooms` first"))?.id.clone()
        };
        self.txn += 1;
        let txn = format!("mandala-{}-{}", std::process::id(), self.txn);
        let payload = serde_json::json!({ "msgtype": "m.text", "body": text });
        let path = format!("rooms/{}/send/m.room.message/{}", urlencode(&room_id), urlencode(&txn));
        self.put(&session.homeserver, &path, &session.access_token, &payload)?;
        Ok(format!("sent to {}: {}", clip(&room_id, 12), text))
    }

    fn status(&self) -> String {
        match &self.session {
            Some(s) => format!("homeserver  {}\nuser        {}\nrooms       {} cached", s.homeserver, s.user_id, self.rooms.len()),
            None => "not logged in (see `help`)".into(),
        }
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        let line = line.trim();
        if line.is_empty() {
            return match self.session.as_ref() {
                // Signed out is a normal state, not an error: the slot shows
                // setup instructions instead of an `! error` banner (this was
                // caught by the on-device probe, where `launch` must be `ok`).
                None => ModuleReply::ok(format!("CHAT (signed out)\n\n{}", HELP)),
                Some(_) => match self.rooms() {
                    Ok(text) => ModuleReply::ok(text),
                    Err(e) => ModuleReply::err(e),
                },
            };
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "status" => Ok(self.status()),
            "login" => {
                let parsed = parse_auth_args(rest, PASSWORD_ENV);
                match parsed {
                    Ok((hs, user, pass)) => self.login(hs, user, &pass),
                    Err(e) => Err(e),
                }
            }
            "token" => {
                let parsed = parse_auth_args(rest, TOKEN_ENV);
                match parsed {
                    Ok((hs, user, token)) => self.login_token(hs, user, &token),
                    Err(e) => Err(e),
                }
            }
            "logout" => self.logout(),
            "rooms" | "list" => self.rooms(),
            "read" | "history" => self.read(rest),
            "say" | "send" => self.say(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }

    /// Store a manually pasted access token instead of a password login.
    fn login_token(&mut self, homeserver: &str, user: &str, token: &str) -> Result<String, String> {
        let hs = normalize_homeserver(homeserver)?;
        let token = token.trim();
        if token.is_empty() {
            return Err("no token given".into());
        }
        let token = token.to_string();
        // Verify the token against whoami before saving it.
        let probe = self.get(&hs, "account/whoami", &token)?;
        let user_id = probe["user_id"].as_str().ok_or("token rejected (no user_id)")?.to_string();
        let session = Session { homeserver: hs, user: user.trim().to_string(), user_id: user_id.clone(), access_token: token };
        save_session(&self.data_dir, &session)?;
        self.session = Some(session);
        self.since.clear();
        Ok(format!("token accepted for {user_id}; saved to session.json."))
    }
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "CHAT (Matrix)

  login <hs-url> <user> [password]   password sign-in on any homeserver
  token <hs-url> <user> [token]      access token instead of a password

Omit the secret to read it from MANDALA_CHAT_PASSWORD / MANDALA_CHAT_TOKEN.
The launcher never prompts, so it cannot hang on a missing terminal.
  rooms                   poll /sync and list joined rooms
  read <n>                last messages of room n
  say <n|me> <text>       send a message
  logout                  end session and wipe the token

No provider is bundled: point it at the homeserver you trust. Polling is
on-demand only — no background connection, no telemetry.";

fn timeline_latest(timeline: &serde_json::Value) -> (String, String) {
    if let Some(events) = timeline["events"].as_array() {
        for ev in events.iter().rev() {
            if ev["type"] == "m.room.message" {
                let body = ev["content"]["body"].as_str().unwrap_or_default().to_string();
                let sender = ev["sender"].as_str().unwrap_or_default().to_string();
                if !body.is_empty() {
                    return (body, sender);
                }
            }
        }
    }
    (String::new(), String::new())
}

fn normalize_homeserver(hs: &str) -> Result<String, String> {
    let hs = hs.trim().trim_end_matches('/');
    if hs.is_empty() {
        return Err("usage: login <homeserver-url> <user>".into());
    }
    if !(hs.starts_with("https://") || hs.starts_with("http://")) {
        return Err("homeserver must start with https:// (or http:// for a local server)".into());
    }
    if hs.len() > 256 {
        return Err("homeserver URL is too long".into());
    }
    Ok(hs.to_string())
}

/// One HTTP call. `token` adds a bearer header; `body` switches to POST/PUT.
pub fn http(url: &str, method: &str, token: Option<&str>, body: Option<&serde_json::Value>, max: usize) -> Result<String, String> {
    let response = match (method, body) {
        ("GET", _) | (_, None) => {
            if method != "GET" {
                return Err(format!("{method} needs a JSON body"));
            }
            let request = ureq::get(url).header("Accept", "application/json").config().timeout_global(Some(TIMEOUT)).build();
            match token {
                Some(t) => request.header("Authorization", &format!("Bearer {t}")).call(),
                None => request.call(),
            }
        }
        ("PUT", Some(v)) | ("POST", Some(v)) => {
            let request = match method {
                "PUT" => ureq::put(url),
                _ => ureq::post(url),
            }
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(TIMEOUT))
            .build();
            let request = match token {
                Some(t) => request.header("Authorization", &format!("Bearer {t}")),
                None => request,
            };
            request.send(serde_json::to_string(v).map_err(|e| e.to_string())?)
        }
        (other, _) => return Err(format!("unsupported method {other}")),
    };
    let mut response = response.map_err(|e| format!("{method} {url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        let m = serde_json::from_str::<serde_json::Value>(&detail)
            .ok()
            .and_then(|v| v["errcode"].as_str().map(|c| format!(" [{c}]")))
            .unwrap_or_default();
        return Err(format!("{method} {url}: HTTP {status}{m}"));
    }
    let text = response.body_mut().read_to_string().map_err(|e| format!("{method} {url}: {e}"))?;
    if text.len() > max {
        return Err(format!("{method} {url}: response larger than {max} bytes"));
    }
    Ok(text)
}

/// Split `login`/`token` arguments: `<homeserver> <user> [secret]`, with the
/// secret falling back to an environment variable. The module never reads a
/// terminal — it runs on the launcher's UI path, where blocking on stdin would
/// freeze the shell (and there is no tty at all on Android).
fn parse_auth_args<'a>(rest: &'a str, secret_env: &str) -> Result<(&'a str, &'a str, String), String> {
    let mut parts = rest.splitn(3, char::is_whitespace).filter(|p| !p.is_empty());
    let hs = parts.next().ok_or_else(|| "usage: <homeserver-url> <user> [secret]".to_string())?;
    let user = parts.next().ok_or_else(|| "usage: <homeserver-url> <user> [secret]".to_string())?;
    if let Some(secret) = parts.next() {
        return Ok((hs, user, secret.trim().to_string()));
    }
    match std::env::var(secret_env) {
        Ok(v) if !v.trim().is_empty() => Ok((hs, user, v.trim().to_string())),
        _ => Err(format!(
            "no secret given.\n\nEither append it to the command, or export it first:\n  export {secret_env}=...\n\nThe launcher never prompts: it has no terminal to read from."
        )),
    }
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn short(sender: &str) -> String {
    sender.trim_start_matches('@').split(':').next().unwrap_or(sender).to_string()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn session_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("session.json")
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

fn save_session(data_dir: &std::path::Path, session: &Session) -> Result<(), String> {
    let path = session_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(session).map_err(|e| e.to_string())?;
    write_private(&path, &json).map_err(|e| e.to_string())
}

fn clear_session(data_dir: &std::path::Path) -> Result<(), String> {
    let path = session_path(data_dir);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Module for Chat {
    fn id(&self) -> &'static str {
        "app-chat"
    }
    fn name(&self) -> &'static str {
        "Chat"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        std::fs::create_dir_all(&ctx.data_dir).map_err(|e| e.to_string())?;
        self.data_dir = PathBuf::from(&ctx.data_dir);
        if let Ok(text) = std::fs::read_to_string(session_path(&self.data_dir)) {
            if let Ok(session) = serde_json::from_str::<Session>(&text) {
                if !session.access_token.is_empty() {
                    self.session = Some(session);
                }
            }
        }
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" | "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Chat::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Read one request: first the header block, then exactly `Content-Length`
    /// bytes of body.
    ///
    /// Answering as soon as `\r\n\r\n` arrives is what broke the Windows CI
    /// job: winsock delivers the body in a separate segment, so the client was
    /// still writing when the server closed the socket (ECONNRESET). Linux and
    /// macOS usually coalesce headers and body into one read, which hid it.
    fn read_request(sock: &mut std::net::TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        let mut body_start = None;
        while body_start.is_none() {
            match sock.read(&mut chunk) {
                Ok(0) | Err(_) => return buf,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            body_start = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4);
        }
        let body_start = body_start.unwrap();
        let head = String::from_utf8_lossy(&buf[..body_start]).to_ascii_lowercase();
        let len = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < body_start + len {
            match sock.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        buf
    }

    fn ready() -> (tempfile::TempDir, Chat) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Chat::new();
        c.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-chat") }).unwrap();
        (dir, c)
    }

    fn ok(c: &mut Chat, cmd: &str) -> String {
        let out = c.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    /// A scripted homeserver: each request consumes the next canned response
    /// from `bodies`. Received request bodies are kept in `requests` so a test
    /// can prove the upload actually arrived.
    struct Server {
        port: u16,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    fn serve(bodies: Vec<String>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(bodies));
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = shared.clone();
        let seen = requests.clone();
        std::thread::spawn(move || {
            for _ in 0..64 {
                if let Ok((mut sock, _)) = listener.accept() {
                    let request = read_request(&mut sock);
                    seen.lock().unwrap().push(String::from_utf8_lossy(&request).into_owned());
                    let body = sink.lock().unwrap().pop().unwrap_or_else(|| "{}".into());
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(response.as_bytes());
                }
            }
        });
        Server { port, requests }
    }

    fn login_direct(c: &mut Chat, hs: &str) {
        c.session = Some(Session {
            homeserver: hs.to_string(),
            user: "ada".into(),
            user_id: "@ada:example.org".into(),
            access_token: "secret-token".into(),
        });
    }

    #[test]
    fn homeserver_and_path_building() {
        assert_eq!(normalize_homeserver(" https://matrix.example.org/ ").unwrap(), "https://matrix.example.org");
        assert!(normalize_homeserver("matrix.example.org").is_err(), "scheme required");
        assert!(normalize_homeserver("").is_err());
        assert_eq!(urlencode("!room:example.org"), "%21room%3Aexample.org");
    }

    #[test]
    fn missing_secret_is_an_error_not_a_prompt() {
        let (_d, mut c) = ready();
        std::env::remove_var(PASSWORD_ENV);
        std::env::remove_var(TOKEN_ENV);
        let out = c.execute("login https://matrix.example.org ada");
        assert!(!out.ok);
        assert!(out.text.contains(PASSWORD_ENV), "must name the env var: {}", out.text);
        let out = c.execute("token https://matrix.example.org ada");
        assert!(!out.ok && out.text.contains(TOKEN_ENV), "{}", out.text);

        // Environment fallback works without any terminal.
        std::env::set_var(PASSWORD_ENV, "from-env");
        let (hs, user, secret) = parse_auth_args("https://matrix.example.org ada", PASSWORD_ENV).unwrap();
        assert_eq!((hs, user, secret.as_str()), ("https://matrix.example.org", "ada", "from-env"));
        assert!(!parse_auth_args("https://matrix.example.org", PASSWORD_ENV).is_ok(), "user required");
        std::env::remove_var(PASSWORD_ENV);
    }

    #[test]
    fn logged_out_state_explains_how_to_log_in() {
        let (_d, mut c) = ready();
        // `launch` must succeed even signed out: it is the slot's first view.
        let launch = c.handle("launch", "");
        assert!(launch.ok && launch.text.contains("signed out"), "{}", launch.text);
        let out = c.execute("rooms");
        assert!(!out.ok && out.text.contains("not logged in"), "{}", out.text);
        let out = c.execute("say 1 hi");
        assert!(!out.ok);
        assert!(c.execute("status").text.contains("not logged in"));
        assert!(c.execute("help").text.contains("Matrix"));
        assert!(!c.execute("bogus").ok);
    }

    #[test]
    fn login_saves_a_0600_session_and_reloads() {
        let (dir, mut c) = ready();
        let server = serve(vec![r#"{"user_id":"@ada:example.org","access_token":"tok123","device_id":"DEV"}"#.into()]);
        let out = ok(&mut c, &format!("login http://127.0.0.1:{} ada hunter2", server.port));
        assert!(out.contains("@ada:example.org"), "{out}");
        assert!(
            server.requests.lock().unwrap().iter().any(|r| r.contains("hunter2")),
            "the POST body must reach the server: {:?}",
            server.requests.lock().unwrap()
        );

        let raw = std::fs::read_to_string(dir.path().join("session.json")).unwrap();
        assert!(raw.contains("tok123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("session.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "session must not be group/world readable");
        }

        let mut second = Chat::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-chat") })
            .unwrap();
        assert!(second.session.is_some(), "session reloads across restarts");

        ok(&mut c, "logout");
        assert!(!dir.path().join("session.json").exists(), "logout wipes the token");
        assert!(c.session.is_none());
    }

    #[test]
    fn bad_login_is_rejected_without_saving() {
        let (dir, mut c) = ready();
        let server = serve(vec![r#"{"errcode":"M_FORBIDDEN","error":"Bad password"}"#.into()]);
        let out = c.execute(&format!("login http://127.0.0.1:{} ada wrong-password", server.port));
        assert!(!out.ok, "{}", out.text);
        assert!(!dir.path().join("session.json").exists(), "failed login must not persist a token");
    }

    #[test]
    fn rooms_and_read_and_say() {
        let (_d, mut c) = ready();
        let sync = r#"{"next_batch":"s123_456","rooms":{"join":{
          "!room1:example.org":{"state":{"events":[{"type":"m.room.name","content":{"name":"Team"}}]},
            "timeline":{"events":[{"type":"m.room.message","sender":"@bob:example.org","content":{"msgtype":"m.text","body":"deploy done"}}]}}}}}"#;
        let messages = r#"{"chunk":[
          {"type":"m.room.message","sender":"@bob:example.org","content":{"msgtype":"m.text","body":"deploy done"}},
          {"type":"m.room.message","sender":"@ada:example.org","content":{"msgtype":"m.text","body":"nice!"}},
          {"type":"m.room.message","sender":"@ada:example.org","content":{"msgtype":"m.image","body":"photo.jpg"}}]}"#;
        // Responses are popped in reverse order of arrival.
        let server = serve(vec![
            r#"{"event_id":"$txn1"}"#.into(),
            messages.into(),
            sync.into(),
        ]);
        login_direct(&mut c, &format!("http://127.0.0.1:{}", server.port));

        let rooms = ok(&mut c, "rooms");
        assert!(rooms.contains("Team") && rooms.contains("deploy done"), "{rooms}");

        let history = ok(&mut c, "read 1");
        assert!(history.contains("bob: deploy done"), "{history}");
        assert!(history.contains("ada: nice!"), "chronological order expected: {history}");
        assert!(history.contains("[m.image] (not rendered)"), "non-text events are noted, not rendered: {history}");
        assert!(!c.execute("read 9").ok, "unknown room index");

        let sent = ok(&mut c, "say 1 ship it");
        assert!(sent.contains("ship it"), "{sent}");
        assert!(
            server.requests.lock().unwrap().iter().any(|r| r.contains("ship it")),
            "the message body must reach the server: {:?}",
            server.requests.lock().unwrap()
        );
        assert!(!c.execute("say 1").ok, "empty message rejected");
        assert!(!c.execute("say 9 hi").ok, "unknown room rejected");
    }

    #[test]
    fn unsupported_media_and_clipping() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert_eq!(short("@ada:example.org"), "ada");
    }
}
