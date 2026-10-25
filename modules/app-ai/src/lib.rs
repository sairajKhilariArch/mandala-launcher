//! A5 Assistant: a bridge to any OpenAI-compatible chat-completions endpoint.
//!
//! Design constraints, straight from the roadmap:
//! * **Local-first** — nothing runs until you ask; no background calls, no
//!   telemetry, no vendor SDK. The endpoint is configurable (your own llama.cpp
//!   server, an internal gateway, or a hosted API you have already agreed to).
//! * **Key stays private** — the API key comes from the `MANDALA_AI_API_KEY`
//!   environment variable where one exists (Linux/macOS/Windows), or from
//!   `key <value>` on Android, where an installed app cannot inherit an
//!   exported variable from anyone's shell. The key is stored in
//!   `<data_dir>/key` (0600, app-private), is never echoed back, never logged
//!   and never written into `config.json`. Config on disk holds just
//!   `base_url`, `model` and `system`.
//! * **Streaming is out of scope** for the command bar; each question is one
//!   request/response, capped in size and time.
//!
//! Actions
//! * `launch` – show the last answer, or setup help
//! * `run`    – `ask <question>` · `set <base-url>` · `model <name>` ·
//!   `system <instruction>` · `clear` · `status` · `help`
//!
//! Request shape is the standard `POST {base}/chat/completions` with
//! `{"model", "messages": [...]}`; responses are parsed from
//! `choices[0].message.content`.

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const MAX_BODY: usize = 512 * 1024;
const MAX_PROMPT: usize = 4000;
const MAX_ANSWER_CHARS: usize = 4000;
const TIMEOUT: Duration = Duration::from_secs(60);
const HISTORY: usize = 12;
const KEY_ENV: &str = "MANDALA_AI_API_KEY";
const DEFAULT_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Config {
    #[serde(default = "default_base")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub system: String,
}

fn default_base() -> String {
    DEFAULT_BASE.into()
}
fn default_model() -> String {
    DEFAULT_MODEL.into()
}

impl Default for Config {
    fn default() -> Self {
        Self { base_url: default_base(), model: default_model(), system: String::new() }
    }
}

pub struct Ai {
    config: Option<PathBuf>,
    cfg: Config,
    history: Vec<(String, String)>,
    /// Test/embedding hook: when set, takes precedence over the environment.
    /// Never written to disk by `save`, so a caller cannot leak a key into
    /// `config.json` through it.
    key_override: Option<String>,
}

impl Ai {
    pub fn new() -> Self {
        Self { config: None, cfg: Config::default(), history: Vec::new(), key_override: None }
    }

    fn config_path(&self) -> Result<PathBuf, String> {
        self.config.clone().ok_or_else(|| "assistant storage not initialised".to_string())
    }

    fn save(&self) -> Result<(), String> {
        let path = self.config_path()?;
        write_private(&path, serde_json::to_string_pretty(&self.cfg).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())
    }

    fn key_path(&self) -> Result<PathBuf, String> {
        let config = self.config_path()?;
        config
            .parent()
            .map(|dir| dir.join("key"))
            .ok_or_else(|| "assistant storage not initialised".to_string())
    }

    fn api_key(&self) -> Result<String, String> {
        if let Some(k) = &self.key_override {
            return Ok(k.clone());
        }
        match std::env::var(KEY_ENV) {
            Ok(k) if !k.trim().is_empty() => return Ok(k.trim().to_string()),
            _ => {}
        }
        match self.key_path().and_then(|p| {
            std::fs::read_to_string(&p)
                .map(|s| s.trim().to_string())
                .map_err(|e| e.to_string())
        }) {
            Ok(k) if !k.is_empty() => Ok(k),
            _ => Err(format!(
                "no API key found.\n\nEither export one before starting the launcher:\n  export {KEY_ENV}=sk-...\nor set one from the command bar (this is what Android does — an installed app\ncannot inherit a shell variable):\n  key sk-...\n\nThe key is stored 0600 in the module's private directory, never logged and\nnever put in config. Point at your own endpoint with `set <base-url>`."
            )),
        }
    }

    fn key_state(&self) -> String {
        if self.key_override.is_some() {
            "present (injected)".to_string()
        } else if std::env::var(KEY_ENV).map(|k| !k.trim().is_empty()).unwrap_or(false) {
            "present (env)".to_string()
        } else if self.key_path().map(|p| p.exists()).unwrap_or(false) {
            "present (key file)".to_string()
        } else {
            "MISSING — export MANDALA_AI_API_KEY or run `key <value>`".to_string()
        }
    }

    /// `key <value>` persists a key Android can actually deliver;
    /// `key clear` removes it. The value is never echoed in any reply.
    fn set_key(&mut self, value: &str) -> Result<String, String> {
        let path = self.key_path()?;
        if value == "clear" || value == "-" {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            return Ok("key cleared.".into());
        }
        if value.is_empty() {
            return Err("usage: key <value>   (or `key clear`)".into());
        }
        if value.len() > 512 || value.chars().any(|c| c.is_whitespace()) {
            return Err("key looks wrong (no spaces, < 512 chars)".into());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        write_private(&path, value.as_bytes()).map_err(|e| e.to_string())?;
        Ok("key saved (0600, private directory). It is never shown again;\nUse `key clear` to remove it.".into())
    }

    fn set_base(&mut self, url: &str) -> Result<String, String> {
        let url = url.trim().trim_end_matches('/');
        if url.is_empty() {
            return Err("usage: set <base-url>   (e.g. http://127.0.0.1:8080/v1)".into());
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("base URL must start with https:// or http://".into());
        }
        if url.len() > 256 {
            return Err("base URL is too long".into());
        }
        self.cfg.base_url = url.to_string();
        self.save()?;
        Ok(format!("base URL: {url}\nUse `model <name>` to pick a model, then `ask <question>`.")
            + &format!("\nAPI key: {}", if std::env::var(KEY_ENV).is_ok() { "present" } else { "MISSING — export {KEY_ENV}" }))
    }

    fn set_model(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("usage: model <name>".into());
        }
        if name.len() > 120 || name.chars().any(|c| c.is_whitespace()) {
            return Err("model name looks wrong (no spaces, < 120 chars)".into());
        }
        self.cfg.model = name.to_string();
        self.save()?;
        Ok(format!("model: {name}"))
    }

    fn set_system(&mut self, text: &str) -> Result<String, String> {
        let text = text.trim();
        if text.len() > 2000 {
            return Err("system instruction too long (< 2000 chars)".into());
        }
        self.cfg.system = text.to_string();
        self.save()?;
        Ok(if text.is_empty() { "system instruction cleared.".into() } else { format!("system: {text}") })
    }

    fn ask(&mut self, question: &str) -> Result<String, String> {
        let question = question.trim();
        if question.is_empty() {
            return Err("usage: ask <question>".into());
        }
        if question.chars().count() > MAX_PROMPT {
            return Err(format!("question too long ({} > {MAX_PROMPT} chars)", question.chars().count()));
        }
        let key = self.api_key()?;
        let mut messages = Vec::new();
        if !self.cfg.system.is_empty() {
            messages.push(serde_json::json!({ "role": "system", "content": self.cfg.system }));
        }
        for (q, a) in &self.history {
            messages.push(serde_json::json!({ "role": "user", "content": q }));
            messages.push(serde_json::json!({ "role": "assistant", "content": a }));
        }
        messages.push(serde_json::json!({ "role": "user", "content": question }));

        let payload = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "max_tokens": 800,
            "temperature": 0.2,
        });
        let url = format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let body = http_post(&url, &key, &payload)?;
        let answer = parse_answer(&body)?;
        let answer: String = answer.chars().take(MAX_ANSWER_CHARS).collect();

        self.history.push((question.to_string(), answer.clone()));
        if self.history.len() > HISTORY {
            self.history.drain(0..self.history.len() - HISTORY);
        }
        Ok(format!("{} — {}\n\n{}", self.cfg.model, self.cfg.base_url, answer))
    }

    fn status(&self) -> String {
        let key_state = self.key_state();
        format!("base   {}\nmodel  {}\nkey    {}\nsystem {}\nhistory {} turns", self.cfg.base_url, self.cfg.model, key_state, clip(&self.cfg.system, 40), self.history.len())
    }

    fn last_or_help(&self) -> String {
        match self.history.last() {
            Some((q, a)) => format!("LAST ANSWER\n\nQ: {q}\n\n{a}"),
            None => format!("ASSISTANT (not configured)\n\n{}", HELP),
        }
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.config.is_none() {
            return ModuleReply::err("assistant storage not initialised");
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.last_or_help());
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "status" | "config" => Ok(self.status()),
            "set" => self.set_base(rest),
            "model" => self.set_model(rest),
            "system" => self.set_system(rest),
            "key" if rest.is_empty() => Ok(self.key_state()),
            "key" => self.set_key(rest),
            "clear" => {
                self.history.clear();
                Ok("conversation cleared.".into())
            }
            "ask" | "question" => self.ask(rest),
            // `ai explain rust lifetimes` reads as `ask explain rust lifetimes`.
            other => self.ask(&format!("{other} {rest}")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Ai {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "ASSISTANT

  set <base-url>        OpenAI-compatible endpoint (…/v1)
  model <name>          model name the endpoint serves
  system <instruction>  optional persona / rules
  ask <question>        one-shot question (keeps short context)
  key <value>           store an API key (0600, private; `key clear` removes)
  clear                 forget the conversation
  status                configuration and key state

The key comes from the MANDALA_AI_API_KEY environment variable, or from
`key <value>` where there is no shell to export one (Android). It is never
echoed, never logged and never written into config. Nothing is sent until you
ask.";

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

/// `POST <url>` with a bearer key and JSON body.
pub fn http_post(url: &str, key: &str, payload: &serde_json::Value) -> Result<String, String> {
    let body = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let mut response = ureq::post(url)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .config()
        .timeout_global(Some(TIMEOUT))
        .build()
        .send(body)
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        let hint = match status {
            401 | 403 => " — the key was rejected; re-export MANDALA_AI_API_KEY".to_string(),
            404 => format!(" — does this endpoint serve /chat/completions? (model {})", payload["model"]),
            429 => " — rate limited; try again shortly".to_string(),
            _ => String::new(),
        };
        return Err(format!("HTTP {status}{hint}\n{}", clip(&detail, 300)));
    }
    let text = response.body_mut().read_to_string().map_err(|e| format!("POST {url}: {e}"))?;
    if text.len() > MAX_BODY {
        return Err(format!("POST {url}: response larger than {MAX_BODY} bytes"));
    }
    Ok(text)
}

/// Extract `choices[0].message.content` from a chat-completions response.
pub fn parse_answer(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("endpoint response: {e}"))?;
    if let Some(err) = value["error"]["message"].as_str() {
        return Err(format!("endpoint error: {err}"));
    }
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "endpoint response had no choices[0].message.content".to_string())
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

impl Module for Ai {
    fn id(&self) -> &'static str {
        "app-ai"
    }
    fn name(&self) -> &'static str {
        "Assistant"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("config.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
                self.cfg = cfg;
            }
        }
        self.config = Some(path);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" if self.config.is_some() => ModuleReply::ok(self.last_or_help()),
            "launch" => ModuleReply::err("assistant storage not initialised"),
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Ai::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn ready() -> (tempfile::TempDir, Ai) {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Ai::new();
        a.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-ai") }).unwrap();
        (dir, a)
    }

    fn ok(a: &mut Ai, cmd: &str) -> String {
        let out = a.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    /// Serve one canned chat-completions response, asserting the request
    /// carried the expected bearer key.
    fn serve_chat(answer: &str, expect_key: &str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = format!(r#"{{"id":"c1","choices":[{{"message":{{"role":"assistant","content":"{answer}"}}}}]}}"#);
        let expected = format!("Authorization: Bearer {expect_key}");
        std::thread::spawn(move || {
            for _ in 0..8 {
                if let Ok((mut sock, _)) = listener.accept() {
                    let mut buf = vec![0u8; 16384];
                    let mut read = 0usize;
                    loop {
                        match sock.read(&mut buf[read..]) {
                            Ok(0) => break,
                            Ok(n) => {
                                read += n;
                                if buf[..read].windows(4).any(|w| w == b"\r\n\r\n") && buf[..read].contains(&b'{') {
                                    break;
                                }
                                if read >= buf.len() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();
                    assert!(
                        request.to_ascii_lowercase().contains(&expected.to_ascii_lowercase()),
                        "request must carry the bearer key:\n{request}"
                    );
                    assert!(request.contains("/chat/completions"), "{request}");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(response.as_bytes());
                }
            }
        });
        port
    }

    fn login(a: &mut Ai, hs: &str) {
        a.cfg.base_url = hs.to_string();
    }

    #[test]
    fn config_defaults_and_persistence() {
        let (dir, mut a) = ready();
        assert_eq!(a.cfg.base_url, DEFAULT_BASE);
        assert_eq!(a.cfg.model, DEFAULT_MODEL);
        assert!(!a.execute("set example.com").ok, "scheme required");
        assert!(!a.execute("set").ok);
        ok(&mut a, "set http://127.0.0.1:9999/v1");
        ok(&mut a, "model llama3");
        ok(&mut a, "system Answer tersely.");

        let mut second = Ai::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-ai") })
            .unwrap();
        assert_eq!(second.cfg.base_url, "http://127.0.0.1:9999/v1");
        assert_eq!(second.cfg.model, "llama3");
        assert_eq!(second.cfg.system, "Answer tersely.");
        assert!(!second.execute("model has space").ok);
        assert!(!second.execute("model").ok);
    }

    #[test]
    fn missing_key_fails_with_instructions_and_sends_nothing() {
        std::env::remove_var(KEY_ENV);
        let (_d, mut a) = ready();
        let out = a.execute("ask hello");
        assert!(!out.ok, "{}", out.text);
        assert!(out.text.contains(KEY_ENV), "must name the env var: {}", out.text);
        let status = a.execute("status").text;
        assert!(status.contains("MISSING"), "{status}");
    }

    #[test]
    fn key_command_supplies_a_key_where_env_cannot() {
        // Android: an installed app inherits no shell variable, so `key` is the
        // only usable route. It must persist 0600 and never echo the value.
        std::env::remove_var(KEY_ENV);
        let (dir, mut a) = ready();
        assert!(a.execute("status").text.contains("MISSING"));
        assert!(a.execute("key").ok, "bare `key` reports state");

        let out = a.execute("key sk-android-123");
        assert!(out.ok, "{}", out.text);
        assert!(!out.text.contains("sk-android-123"), "the key must never be echoed: {}", out.text);
        let path = dir.path().join("key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(a.execute("status").text.contains("present (key file)"), "{}", a.execute("status").text);

        // A restart reloads it — the value lives on disk, not in the process.
        let mut second = Ai::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-ai") })
            .unwrap();
        assert_eq!(second.api_key().unwrap(), "sk-android-123");

        assert!(!a.execute("key sk-bad key").ok, "whitespace rejected");
        assert!(a.execute("key clear").text.contains("cleared"));
        assert!(!path.exists(), "clear removes the file");
        assert!(a.execute("status").text.contains("MISSING"));
    }

    #[test]
    fn ask_sends_bearer_and_renders_answer() {
        let (_d, mut a) = ready();
        a.key_override = Some("sk-test-123".into());
        let port = serve_chat("Hello there", "sk-test-123");
        login(&mut a, &format!("http://127.0.0.1:{port}/v1"));
        let out = ok(&mut a, "ask say hi");
        assert!(out.contains("Hello there"), "{out}");
        assert!(out.contains(DEFAULT_MODEL), "configured model shown: {out}");

        // Bare `ai ...` style: unknown first word is treated as the question.
        let out = ok(&mut a, "explain rust");
        assert!(out.contains("Hello there"), "{out}");
        assert!(a.execute("status").text.contains("2 turns"), "history tracked");
        ok(&mut a, "clear");
        assert!(a.execute("status").text.contains("0 turns"));
    }

    #[test]
    fn answer_parsing_edge_cases() {
        assert_eq!(parse_answer(r#"{"choices":[{"message":{"content":"ok"}}]}"#).unwrap(), "ok");
        assert!(parse_answer(r#"{"choices":[]}"#).is_err());
        assert!(parse_answer(r#"{"error":{"message":"model not found"}}"#).unwrap_err().contains("model not found"));
        assert!(parse_answer("not json").is_err());
        assert!(parse_answer(r#"{"choices":[{"message":{"content":"   "}}]}"#).is_err(), "blank answer rejected");
    }

    #[test]
    fn oversized_prompt_rejected() {
        let (_d, mut a) = ready();
        a.key_override = Some("sk-x".into());
        let long = "x".repeat(MAX_PROMPT + 1);
        let out = a.execute(&format!("ask {long}"));
        assert!(!out.ok && out.text.contains("too long"), "{}", out.text);
    }

    #[test]
    fn uninitialised_and_unknown_actions() {
        let mut a = Ai::new();
        assert!(!a.handle("launch", "").ok);
        assert!(!a.handle("run", "ask x").ok);
        assert!(!a.handle("bogus", "").ok);
        let (_d, mut a) = ready();
        assert!(a.execute("help").text.contains("ASSISTANT"));
        assert!(a.execute("").text.contains("not configured"));
    }
}
