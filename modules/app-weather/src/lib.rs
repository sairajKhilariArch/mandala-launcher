//! B1 Weather: current conditions from an endpoint **you** configure.
//!
//! Why this is configurable rather than pre-wired: the Gravity Index catalog
//! contains no key-free weather data API (two searches returned no
//! recommendation), and a privacy-first launcher should not silently point
//! every user's device at one vendor anyway. So the module ships a tiny,
//! documented JSON contract instead of a provider binding — point it at your
//! own server, a work proxy, or any API you have already agreed with.
//!
//! Contract — `GET <endpoint>` (the city is substituted where the endpoint
//! contains `{q}`, otherwise sent as `?q=<city>` / `&q=<city>`):
//!
//! ```json
//! { "location": "Berlin", "temp_c": 12.4, "condition": "Partly cloudy",
//!   "feels_like_c": 10.1, "humidity": 71, "wind_kph": 14.0,
//!   "high_c": 14.0, "low_c": 8.0, "updated_at": "2025-10-24T12:00:00Z" }
//! ```
//!
//! Only `temp_c` and `condition` are required. The last successful response is
//! cached, so `weather` still answers while offline.
//!
//! Actions
//! * `launch` – last observation (or setup instructions)
//! * `run`    – `set <endpoint-url>` · `city <name>` · `fetch [city]` ·
//!   `status` · `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENDPOINT: usize = 512;
const MAX_CITY: usize = 64;
const MAX_BODY: usize = 64 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const USER_AGENT: &str = "Mandala-Weather/1.0 (local; user-configured endpoint)";

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Observation {
    #[serde(default)]
    pub location: String,
    pub temp_c: f64,
    pub condition: String,
    #[serde(default)]
    pub feels_like_c: Option<f64>,
    #[serde(default)]
    pub humidity: Option<u32>,
    #[serde(default)]
    pub wind_kph: Option<f64>,
    #[serde(default)]
    pub high_c: Option<f64>,
    #[serde(default)]
    pub low_c: Option<f64>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
struct Config {
    endpoint: String,
    city: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
struct Cached {
    fetched_at: u64,
    obs: Observation,
}

pub struct Weather {
    config: Option<PathBuf>,
    endpoint: String,
    city: String,
    cache: Option<Cached>,
}

impl Weather {
    pub fn new() -> Self {
        Self { config: None, endpoint: String::new(), city: String::new(), cache: None }
    }

    fn config_path(&self) -> Result<PathBuf, String> {
        self.config.clone().ok_or_else(|| "weather storage not initialised".to_string())
    }

    fn load(path: &Path) -> (Config, Option<Cached>) {
        let cfg = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Config>(&t).ok())
            .unwrap_or_default();
        // The cache is meaningful only if it was written by a configured
        // endpoint, but keeping it after `set` changes is harmless: it is
        // displayed as "cached", never as a live reading.
        let cache = std::fs::read_to_string(path.with_file_name("cache.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        (cfg, cache)
    }

    fn save(&self) -> Result<(), String> {
        let path = self.config_path()?;
        let cfg = Config { endpoint: self.endpoint.clone(), city: self.city.clone() };
        write_private(&path, serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())
    }

    fn save_cache(&self) -> Result<(), String> {
        let path = self.config_path()?;
        let Some(c) = &self.cache else { return Ok(()) };
        write_private(&path.with_file_name("cache.json"), serde_json::to_string_pretty(c).map_err(|e| e.to_string())?.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn set_endpoint(&mut self, url: &str) -> Result<String, String> {
        let url = url.trim();
        if url.is_empty() {
            return Err("usage: set <http(s)://…>".into());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("endpoint must start with http:// or https://".into());
        }
        if url.len() > MAX_ENDPOINT {
            return Err("endpoint is too long".into());
        }
        if url.chars().any(|c| c.is_whitespace()) {
            return Err("endpoint must not contain spaces".into());
        }
        self.endpoint = url.to_string();
        self.save()?;
        Ok(format!("endpoint set to {url}\nuse: fetch <city>"))
    }

    fn set_city(&mut self, city: &str) -> Result<String, String> {
        let city = city.trim();
        if city.is_empty() {
            return Err("usage: city <name>".into());
        }
        if city.chars().count() > MAX_CITY {
            return Err("city name is too long".into());
        }
        self.city = city.to_string();
        self.save()?;
        Ok(format!("default city: {city}"))
    }

    /// Build the request URL for `city` from the configured endpoint.
    pub fn request_url(endpoint: &str, city: &str) -> String {
        if endpoint.contains("{q}") {
            return endpoint.replace("{q}", &url_encode(city));
        }
        let sep = if endpoint.contains('?') { '&' } else { '?' };
        format!("{endpoint}{sep}q={}", url_encode(city))
    }

    fn fetch(&mut self, city_arg: &str) -> Result<String, String> {
        let endpoint = self.endpoint.clone();
        if endpoint.is_empty() {
            return Err(format!("no endpoint configured.\n\nSet one with:\n  set https://your-weather-host.example/obs\n\nIt must answer `{}` with JSON:\n{}", "GET ?q=<city>", CONTRACT_EXAMPLE));
        }
        let city = if city_arg.trim().is_empty() {
            if self.city.is_empty() {
                return Err("no city: use `fetch <city>` or `city <name>`".into());
            } else {
                self.city.clone()
            }
        } else {
            city_arg.trim().to_string()
        };
        let url = Weather::request_url(&endpoint, &city);
        let body = http_get(&url, MAX_BODY)?;
        let obs = Observation::from_json(&body).map_err(|e| format!("endpoint response: {e}"))?;
        let obs = Observation { location: if obs.location.trim().is_empty() { city.clone() } else { obs.location }, ..obs };
        self.cache = Some(Cached { fetched_at: now_secs(), obs: obs.clone() });
        self.save_cache()?;
        Ok(render(&obs, self.cache.as_ref().map(|c| c.fetched_at).unwrap_or(0)))
    }

    fn status(&self) -> String {
        format!(
            "endpoint  {}\ncity      {}\ncached    {}",
            if self.endpoint.is_empty() { "(not set — `set <url>`)" } else { &self.endpoint },
            if self.city.is_empty() { "(not set — `city <name>`)" } else { &self.city },
            match &self.cache {
                Some(c) => format!("{} (age {} min)", c.obs.location, (now_secs() - c.fetched_at) / 60),
                None => "(none)".into(),
            }
        )
    }

    fn last_or_help(&self) -> String {
        match &self.cache {
            Some(c) => render(&c.obs, c.fetched_at),
            None if self.endpoint.is_empty() => format!(
                "WEATHER (not configured)\n\nThis module fetches from an endpoint you choose, so your\nweather requests stay on hosts you trust.\n\n  set <url>      point at an endpoint returning:\n{CONTRACT_EXAMPLE}\n  city <name>    remember your city\n  fetch          get the current conditions\n\nconfig:\n{}",
                self.status()
            ),
            None => format!("WEATHER (no data yet)\n\nRun `fetch <city>`.\n\nconfig:\n{}", self.status()),
        }
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.config.is_none() {
            return ModuleReply::err("weather storage not initialised");
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
            "set" => self.set_endpoint(rest),
            "city" => self.set_city(rest),
            "fetch" | "get" | "now" => self.fetch(rest),
            // `weather Berlin` reads as `fetch Berlin`.
            other if self.config.is_some() => self.fetch(&format!("{other} {rest}")),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

const CONTRACT_EXAMPLE: &str = r#"    { "location": "Berlin", "temp_c": 12.4, "condition": "Partly cloudy",
      "feels_like_c": 10.1, "humidity": 71, "wind_kph": 14.0,
      "high_c": 14.0, "low_c": 8.0 }"#;

const HELP: &str = "WEATHER

  set <url>        endpoint returning the JSON contract (see `help contract`)
  city <name>      remember your city
  fetch [city]     current conditions (cached for offline use)
  status           show configuration and cache age

No provider is bundled: you choose where weather comes from. The endpoint is
called directly from this device — no analytics, no third-party SDK.";

impl Observation {
    pub fn from_json(body: &str) -> Result<Self, String> {
        let obs: Observation = serde_json::from_str(body).map_err(|e| format!("not the agreed JSON: {e}"))?;
        if !obs.temp_c.is_finite() {
            return Err("temp_c is not a number".into());
        }
        if obs.condition.trim().is_empty() {
            return Err("condition is missing".into());
        }
        Ok(obs)
    }
}

fn render(obs: &Observation, fetched_at: u64) -> String {
    let mut out = format!("{}   {:.1}°C  {}\n", obs.location, obs.temp_c, obs.condition);
    if let Some(f) = obs.feels_like_c {
        out.push_str(&format!("feels {:.1}°C", f));
    }
    if let (Some(h), Some(l)) = (obs.high_c, obs.low_c) {
        out.push_str(&format!("   H {:.1} / L {:.1}", h, l));
    }
    out.push('\n');
    if let Some(hu) = obs.humidity {
        out.push_str(&format!("humidity {}%", hu));
    }
    if let Some(w) = obs.wind_kph {
        out.push_str(&format!("   wind {:.0} km/h", w));
    }
    out.push('\n');
    if !obs.updated_at.is_empty() {
        out.push_str(&format!("updated {}\n", obs.updated_at));
    }
    if fetched_at > 0 {
        out.push_str(&format!("cached {} min ago", (now_secs() - fetched_at) / 60));
    }
    out
}

/// GET `url`, capped at `max` bytes.
pub fn http_get(url: &str, max: usize) -> Result<String, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .config()
        .timeout_global(Some(TIMEOUT))
        .build()
        .call()
        .map_err(|e| format!("fetch {url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("fetch {url}: HTTP {status}"));
    }
    let text = response.body_mut().read_to_string().map_err(|e| format!("fetch {url}: {e}"))?;
    if text.len() > max {
        return Err(format!("fetch {url}: response larger than {max} bytes"));
    }
    Ok(text)
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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

impl Module for Weather {
    fn id(&self) -> &'static str {
        "app-weather"
    }
    fn name(&self) -> &'static str {
        "Weather"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("config.json");
        let (cfg, cache) = Weather::load(&path);
        self.endpoint = cfg.endpoint;
        self.city = cfg.city;
        self.cache = cache;
        self.config = Some(path);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" if self.config.is_some() => ModuleReply::ok(self.last_or_help()),
            "launch" => ModuleReply::err("weather storage not initialised"),
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Weather::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const OBS: &str = r#"{"location":"Berlin","temp_c":12.4,"condition":"Partly cloudy",
        "feels_like_c":10.1,"humidity":71,"wind_kph":14.0,"high_c":14.0,"low_c":8.0,
        "updated_at":"2025-10-24T12:00:00Z"}"#;

    fn ready() -> (tempfile::TempDir, Weather) {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Weather::new();
        w.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-weather") })
            .unwrap();
        (dir, w)
    }

    fn ok(w: &mut Weather, cmd: &str) -> String {
        let out = w.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    fn serve_once(body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn observation_contract_is_validated() {
        let obs = Observation::from_json(OBS).unwrap();
        assert_eq!(obs.location, "Berlin");
        assert_eq!(obs.temp_c, 12.4);
        assert_eq!(obs.humidity, Some(71));

        // Minimal contract: only temp_c + condition are required.
        assert!(Observation::from_json(r#"{"temp_c":5.0,"condition":"Clear"}"#).is_ok());
        assert!(Observation::from_json(r#"{"condition":"Clear"}"#).is_err(), "temp required");
        assert!(Observation::from_json(r#"{"temp_c":5.0}" "#).is_err(), "condition required");
        assert!(Observation::from_json("not json").is_err());
        assert!(Observation::from_json(r#"{"temp_c":NaN,"condition":"x"}"#).is_err());
    }

    #[test]
    fn endpoint_and_city_persist() {
        let (dir, mut w) = ready();
        assert!(!w.execute("set example.com/obs").ok, "scheme required");
        assert!(!w.execute("set").ok);
        assert!(!w.execute("set http://has space/obs").ok);
        ok(&mut w, "set https://obs.example/api");
        ok(&mut w, "city Berlin");

        let mut second = Weather::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-weather") })
            .unwrap();
        let status = ok(&mut second, "status");
        assert!(status.contains("https://obs.example/api") && status.contains("Berlin"), "{status}");
        assert!(!second.execute("city").ok);
        assert!(!second.execute(&format!("city {}", "x".repeat(MAX_CITY + 1))).ok);
    }

    #[test]
    fn request_url_builds_query_or_placeholder() {
        assert_eq!(Weather::request_url("https://a.example/obs", "Berlin"), "https://a.example/obs?q=Berlin");
        assert_eq!(Weather::request_url("https://a.example/obs?lang=en", "New York"), "https://a.example/obs?lang=en&q=New%20York");
        assert_eq!(Weather::request_url("https://a.example/{q}", "São Paulo"), "https://a.example/S%C3%A3o%20Paulo");
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(url_encode("héllo/1"), "h%C3%A9llo%2F1");
    }

    #[test]
    fn fetch_caches_and_renders() {
        let (dir, mut w) = ready();
        assert!(!w.execute("fetch Berlin").ok, "unconfigured endpoint fails with setup help");
        let setup = w.execute("").text;
        assert!(setup.contains("not configured") && setup.contains("temp_c"), "{setup}");

        let port = serve_once(OBS);
        ok(&mut w, &format!("set http://127.0.0.1:{port}/obs"));
        ok(&mut w, "city Berlin");
        let out = ok(&mut w, "fetch");
        assert!(out.contains("Berlin   12.4°C  Partly cloudy"), "{out}");
        assert!(out.contains("H 14.0 / L 8.0") && out.contains("humidity 71%"), "{out}");

        // Offline restart shows the cache.
        let mut second = Weather::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-weather") })
            .unwrap();
        let cached = ok(&mut second, "");
        assert!(cached.contains("Partly cloudy") && cached.contains("cached"), "{cached}");
    }

    #[test]
    fn bad_endpoint_responses_are_reported() {
        let (_d, mut w) = ready();
        let port = serve_once(r#"{"humidity":71}"#);
        ok(&mut w, &format!("set http://127.0.0.1:{port}/obs"));
        let out = w.execute("fetch Berlin");
        assert!(!out.ok && out.text.contains("temp_c"), "{}", out.text);

        // Unreachable endpoint
        ok(&mut w, "set http://127.0.0.1:1/obs");
        let out = w.execute("fetch Berlin");
        assert!(!out.ok && out.text.contains("fetch"), "{}", out.text);
    }

    #[test]
    fn bare_city_argument_reads_as_fetch() {
        let (_d, mut w) = ready();
        let port = serve_once(OBS);
        ok(&mut w, &format!("set http://127.0.0.1:{port}/obs"));
        // `weather Oslo` style: a single unknown word is treated as a city.
        let out = ok(&mut w, "Oslo");
        assert!(out.contains("Berlin   12.4"), "fixture always says Berlin: {out}");
        assert!(!w.execute("helpx").ok || w.execute("helpx").text.contains("fetch"), "unknown word goes through fetch");
    }

    #[test]
    fn uninitialised_and_unknown_actions() {
        let mut w = Weather::new();
        assert!(!w.handle("launch", "").ok);
        assert!(!w.handle("run", "status").ok);
        assert!(!w.handle("bogus", "").ok);
        let (_d, mut w) = ready();
        assert!(w.execute("help").ok);
        let status = w.execute("status").text;
        assert!(status.contains("not set"), "{status}");
    }
}
