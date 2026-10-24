//! B2 RSS / Atom Reader: fetch feeds, cache them locally, read them as text.
//!
//! Design intent (roadmap §2): plain-text article fetching *without*
//! recommendation loops — you only ever see the feeds you added, in the order
//! they were published, newest first. Nothing is personalised, tracked or
//! uploaded, and everything is cached so the list survives going offline.
//!
//! Actions
//! * `launch` – cached headlines
//! * `run`    – `add <url> [title]` · `rm <fragment>` · `feeds` · `fetch` ·
//!   `read <n>` · `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_FEED_BYTES: usize = 512 * 1024;
const MAX_ITEMS: usize = 120;
const MAX_TITLE: usize = 200;
const MAX_SUMMARY: usize = 1_000;
const USER_AGENT: &str = "Mandala-RSS/1.0 (local reader; no tracking)";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub url: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub source: String,
    pub summary: String,
    #[serde(default)]
    pub published: String,
}

pub struct Rss {
    data: Option<PathBuf>,
    feeds: Vec<Feed>,
    items: Vec<Item>,
    warning: Option<String>,
}

impl Rss {
    pub fn new() -> Self {
        Self { data: None, feeds: Vec::new(), items: Vec::new(), warning: None }
    }

    fn feeds_file(&self) -> Result<PathBuf, String> {
        self.data.as_ref().map(|d| d.join("feeds.json")).ok_or_else(|| "reader storage not initialised".to_string())
    }

    fn cache_file(&self) -> Result<PathBuf, String> {
        self.data.as_ref().map(|d| d.join("cache.json")).ok_or_else(|| "reader storage not initialised".to_string())
    }

    fn save_feeds(&self) -> Result<(), String> {
        let path = self.feeds_file()?;
        write_private(&path, serde_json::to_string_pretty(&self.feeds).map_err(|e| e.to_string())?.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn save_cache(&self) -> Result<(), String> {
        let path = self.cache_file()?;
        write_private(&path, serde_json::to_string_pretty(&self.items).map_err(|e| e.to_string())?.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn validate_url(url: &str) -> Result<String, String> {
        let url = url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("feed URL must start with http:// or https://".into());
        }
        if url.len() > 512 {
            return Err("feed URL is too long".into());
        }
        if url.chars().any(|c| c.is_whitespace()) {
            return Err("feed URL must not contain spaces".into());
        }
        Ok(url.to_string())
    }

    fn add(&mut self, args: &str) -> Result<String, String> {
        let args = args.trim();
        let (url, title) = match args.find(char::is_whitespace) {
            Some(i) => (&args[..i], args[i..].trim()),
            None => (args, ""),
        };
        let url = Rss::validate_url(url)?;
        if self.feeds.iter().any(|f| f.url == url) {
            return Err(format!("already subscribed: {url}"));
        }
        if self.feeds.len() >= 50 {
            return Err("feed list is full (50 feeds)".into());
        }
        self.feeds.push(Feed { url: url.clone(), title: title.to_string() });
        self.save_feeds()?;
        Ok(format!("subscribed to {url}{}", if title.is_empty() { String::new() } else { format!(" ({title})") }))
    }

    fn remove(&mut self, fragment: &str) -> Result<String, String> {
        let f = fragment.trim().to_ascii_lowercase();
        if f.is_empty() {
            return Err("usage: rm <url or title fragment>".into());
        }
        let hits: Vec<usize> = self
            .feeds
            .iter()
            .enumerate()
            .filter(|(_, feed)| feed.url.to_ascii_lowercase().contains(&f) || feed.title.to_ascii_lowercase().contains(&f))
            .map(|(i, _)| i)
            .collect();
        match hits.len() {
            0 => Err(format!("no feed matches `{fragment}`")),
            1 => {
                let removed = self.feeds.remove(hits[0]);
                self.save_feeds()?;
                Ok(format!("removed {} ({})", if removed.title.is_empty() { "feed" } else { &removed.title }, removed.url))
            }
            _ => Err(format!("`{fragment}` matches {} feeds; be more specific", hits.len())),
        }
    }

    /// Fetch every feed, merge results into the cache (newest first).
    pub fn fetch_all(&mut self) -> Result<String, String> {
        if self.feeds.is_empty() {
            return Err("no feeds yet: `add <feed-url>` first".into());
        }
        let mut fetched = 0usize;
        let mut failures: Vec<String> = Vec::new();
        let mut merged: Vec<Item> = Vec::new();
        for feed in &self.feeds {
            match http_get(&feed.url, MAX_FEED_BYTES) {
                Ok(xml) => match parse_feed(&xml, &feed.title) {
                    Ok(mut items) => {
                        for it in &mut items {
                            if it.source.is_empty() {
                                it.source = feed.title.clone();
                            }
                        }
                        fetched += 1;
                        merged.append(&mut items);
                    }
                    Err(e) => failures.push(format!("{}: {e}", feed.url)),
                },
                Err(e) => failures.push(e),
            }
        }
        // Newest first, de-duplicated by link/title, capped.
        merged.sort_by(|a, b| b.published.cmp(&a.published));
        merged.dedup_by(|a, b| a.link == b.link && !a.link.is_empty() || a.title == b.title);
        merged.truncate(MAX_ITEMS);
        if !merged.is_empty() {
            self.items = merged;
            self.save_cache()?;
        }
        let mut msg = format!("{fetched}/{} feed(s) updated, {} headline(s)", self.feeds.len(), self.items.len());
        if !failures.is_empty() {
            msg.push_str(&format!("\nfailed:\n  {}", failures.join("\n  ")));
        }
        if fetched == 0 {
            Err(msg)
        } else {
            Ok(msg)
        }
    }

    fn headlines(&self) -> String {
        if self.items.is_empty() {
            return format!(
                "RSS READER\n\n{}\n\nNo headlines cached yet.\n  add <feed-url>   subscribe\n  fetch            download headlines",
                self.feed_list()
            );
        }
        let mut out = format!("HEADLINES ({}), newest first\n\n", self.items.len());
        for (i, it) in self.items.iter().enumerate() {
            let date = if it.published.is_empty() { "—".to_string() } else { it.published.chars().take(16).collect() };
            let title = truncate(&it.title, 72);
            let source = truncate(&it.source, 24);
            out.push_str(&format!("{:>3}. [{date}] {title}  ({source})\n", i + 1));
        }
        out.push_str("\nread <n>   open a headline");
        out
    }

    fn feed_list(&self) -> String {
        if self.feeds.is_empty() {
            return "feeds: (none)".to_string();
        }
        self.feeds
            .iter()
            .map(|f| format!("  {}\n    {}", if f.title.is_empty() { "(untitled)" } else { &f.title }, f.url))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn read(&self, token: &str) -> Result<String, String> {
        let n: usize = token.trim().trim_end_matches('.').parse().map_err(|_| format!("`{token}` is not a headline number"))?;
        let it = self.items.get(n.wrapping_sub(1)).ok_or_else(|| format!("no headline {n}"))?;
        Ok(format!(
            "{title}\nsource  {source}\npublished {published}\nlink    {link}\n\n{summary}",
            title = it.title,
            source = if it.source.is_empty() { "unknown" } else { &it.source },
            published = if it.published.is_empty() { "unknown" } else { &it.published },
            link = it.link,
            summary = if it.summary.is_empty() { "(no summary in this feed)" } else { &it.summary }
        ))
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.data.is_none() {
            return ModuleReply::err("reader storage not initialised");
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.headlines());
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "feeds" | "ls" => Ok(self.feed_list()),
            "add" | "subscribe" => self.add(rest),
            "rm" | "remove" | "unsubscribe" => self.remove(rest),
            "fetch" | "refresh" | "update" => self.fetch_all(),
            "read" | "open" => self.read(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Rss {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "RSS READER

  add <url> [title]   subscribe to a feed
  feeds               list subscriptions
  rm <fragment>       unsubscribe
  fetch               download all feeds into the local cache
  headlines (default) cached headlines, newest first
  read <n>            open one headline with its summary

Only the feeds you add, in publication order — no ranking, no recommendations,
no analytics. Everything is cached in the module's private directory.";

/// GET `url`, returning at most `max` bytes of UTF-8 body text.
pub fn http_get(url: &str, max: usize) -> Result<String, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml, */*")
        .call()
        .map_err(|e| format!("fetch {url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("fetch {url}: HTTP {status}"));
    }
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("fetch {url}: {e}"))?;
    if text.len() > max {
        return Err(format!("fetch {url}: body larger than {max} bytes"));
    }
    Ok(text)
}

/// Parse an RSS 2.0 or Atom document into items. `source` is the feed title
/// the caller knows; channel/feed titles override it when present.
pub fn parse_feed(xml: &str, source: &str) -> Result<Vec<Item>, String> {
    let doc = Document::parse(xml).map_err(|e| format!("not XML: {e}"))?;
    let channel_title = doc
        .descendants()
        .find(|n| n.has_tag_name("channel"))
        .and_then(|c| c.children().find(|n| n.has_tag_name("title")))
        .and_then(|t| t.text())
        .unwrap_or("")
        .trim()
        .to_string();
    let feed_title = doc
        .descendants()
        .find(|n| n.has_tag_name("feed"))
        .and_then(|f| f.children().find(|n| n.has_tag_name("title")))
        .and_then(|t| t.text())
        .unwrap_or("")
        .trim()
        .to_string();
    let source = if !channel_title.is_empty() {
        channel_title
    } else if !feed_title.is_empty() {
        feed_title
    } else {
        source.to_string()
    };

    let mut items = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("item") || n.has_tag_name("entry")) {
        let title = text_of(node, "title");
        let link = if node.has_tag_name("item") {
            text_of(node, "link")
        } else {
            // Atom: prefer the alternate link.
            node.children()
                .filter(|n| n.has_tag_name("link"))
                .find(|l| l.attribute("rel").map(|r| r == "alternate").unwrap_or(true))
                .and_then(|l| l.attribute("href").map(|h| h.to_string()))
                .unwrap_or_default()
        };
        let published = ["published", "updated", "pubDate", "date"]
            .iter()
            .find_map(|tag| node.descendants().find(|n| n.has_tag_name(*tag)).and_then(|n| n.text()))
            .unwrap_or("")
            .trim()
            .to_string();
        let raw_summary = ["description", "summary", "content", "content:encoded", "subtitle"]
            .iter()
            .find_map(|tag| node.descendants().find(|n| n.has_tag_name(*tag)).and_then(|n| n.text()))
            .unwrap_or("");
        items.push(Item {
            title: if title.trim().is_empty() { "(untitled)".into() } else { truncate(title.trim(), MAX_TITLE) },
            link: link.trim().to_string(),
            source: source.clone(),
            summary: truncate(&strip_html(raw_summary), MAX_SUMMARY),
            published,
        });
    }
    if items.is_empty() {
        return Err("XML parsed but no <item> or <entry> elements found".into());
    }
    Ok(items)
}

fn text_of(node: roxmltree::Node, tag: &str) -> String {
    node.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).unwrap_or("").to_string()
}

/// Remove tags and decode the handful of entities feeds actually use.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    }
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
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

impl Module for Rss {
    fn id(&self) -> &'static str {
        "app-rss"
    }
    fn name(&self) -> &'static str {
        "RSS Reader"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let feeds = std::fs::read_to_string(dir.join("feeds.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let items = std::fs::read_to_string(dir.join("cache.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if dir.join("feeds.json").exists() && serde_json::from_str::<Vec<Feed>>(
            &std::fs::read_to_string(dir.join("feeds.json")).unwrap_or_default(),
        )
        .is_err()
        {
            self.warning = Some("feeds.json was unreadable; starting empty".into());
        }
        self.feeds = feeds;
        self.items = items;
        self.data = Some(dir);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" if self.data.is_some() => ModuleReply::ok(self.headlines()),
            "launch" => ModuleReply::err("reader storage not initialised"),
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Rss::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const RSS_FIXTURE: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>Solo RP News</title>
  <item>
    <title>First &amp; foremost</title>
    <link>https://example.com/1</link>
    <pubDate>Mon, 06 Oct 2025 08:00:00 GMT</pubDate>
    <description><![CDATA[<p>Body <b>text</b> here.</p>]]></description>
  </item>
  <item>
    <title>Second story</title>
    <link>https://example.com/2</link>
    <pubDate>Tue, 07 Oct 2025 09:30:00 GMT</pubDate>
    <description>Plain &amp; simple</description>
  </item>
</channel></rss>"#;

    const ATOM_FIXTURE: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Feed</title>
  <entry>
    <title>Entry one</title>
    <link rel="alternate" href="https://example.org/a"/>
    <updated>2025-10-08T10:00:00Z</updated>
    <summary>Summary &lt;em&gt;of&lt;/em&gt; entry</summary>
  </entry>
  <entry>
    <title>Entry two</title>
    <link href="https://example.org/b"/>
    <published>2025-10-09T11:00:00Z</published>
    <content type="html">&lt;p&gt;Second&lt;/p&gt;</content>
  </entry>
</feed>"#;

    fn ready() -> (tempfile::TempDir, Rss) {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Rss::new();
        r.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-rss") }).unwrap();
        (dir, r)
    }

    fn ok(r: &mut Rss, cmd: &str) -> String {
        let out = r.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    /// Serve one HTTP response on a loopback port; returns the port.
    fn serve_once(body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn parses_rss_and_atom() {
        let items = parse_feed(RSS_FIXTURE, "fallback").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First & foremost");
        assert_eq!(items[0].source, "Solo RP News");
        assert_eq!(items[0].link, "https://example.com/1");
        assert_eq!(items[0].summary, "Body text here.");
        assert!(items[0].published.starts_with("Mon, 06 Oct"));

        let items = parse_feed(ATOM_FIXTURE, "fallback").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].source, "Atom Feed");
        assert_eq!(items[0].link, "https://example.org/a");
        assert_eq!(items[1].link, "https://example.org/b");
        assert_eq!(items[0].summary, "Summary of entry");
        assert_eq!(items[1].summary, "Second");
    }

    #[test]
    fn parse_errors_are_reported() {
        assert!(parse_feed("not xml at all", "x").is_err());
        assert!(parse_feed("<rss><channel><title>t</title></channel></rss>", "x").is_err(), "no items");
        assert!(parse_feed("html page", "x").is_err());
    }

    #[test]
    fn subscribe_unsubscribe_and_persistence() {
        let (dir, mut r) = ready();
        assert!(!r.execute("fetch").ok, "fetch with no feeds errors");
        ok(&mut r, "add https://example.com/feed.xml Example");
        assert!(!r.execute("add https://example.com/feed.xml").ok, "duplicate rejected");
        assert!(!r.execute("add ftp://example.com/x").ok, "scheme checked");
        assert!(!r.execute("add example.com/no-scheme").ok);
        assert!(!r.execute("add").ok);
        let feeds = ok(&mut r, "feeds");
        assert!(feeds.contains("https://example.com/feed.xml") && feeds.contains("Example"), "{feeds}");

        let mut second = Rss::new();
        second
            .init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-rss") })
            .unwrap();
        assert!(second.handle("run", "feeds").text.contains("example.com/feed.xml"));

        assert!(second.execute("rm example").ok, "removal succeeds");
        assert!(!second.execute("rm ghost").ok);
        assert!(!second.execute("rm").ok);
    }

    #[test]
    fn fetch_from_a_local_server_caches_headlines() {
        let (dir, mut r) = ready();
        let port = serve_once(RSS_FIXTURE);
        ok(&mut r, &format!("add http://127.0.0.1:{port}/feed.xml Test Feed"));
        let report = ok(&mut r, "fetch");
        assert!(report.contains("1/1 feed(s) updated"), "{report}");

        let heads = ok(&mut r, "");
        assert!(heads.contains("First & foremost") && heads.contains("Second story"), "{heads}");
        assert!(heads.contains("newest first"));

        let item = ok(&mut r, "read 1");
        assert!(item.contains("https://example.com/2"), "newest first: {item}");

        // The cache survives a restart — headlines are available offline.
        let mut same = Rss::new();
        same.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-rss") })
            .unwrap();
        let heads = ok(&mut same, "");
        assert!(heads.contains("Second story"), "cached headlines survive restart: {heads}");
    }

    #[test]
    fn unreachable_feed_reports_failure_and_keeps_cache() {
        let (_d, mut r) = ready();
        ok(&mut r, "add http://127.0.0.1:1/feed.xml down");
        let report = r.execute("fetch");
        assert!(!report.ok, "{}", report.text);
        assert!(report.text.contains("failed:"), "{}", report.text);
        assert!(!r.execute("read 5").ok);
        assert!(!r.execute("read x").ok);
    }

    #[test]
    fn html_is_stripped_and_text_is_truncated() {
        assert_eq!(strip_html("<p>a &amp; b</p>"), "a & b");
        assert_eq!(strip_html("no tags"), "no tags");
        assert_eq!(strip_html("multi\nline  text"), "multi line text");
        let long = "x".repeat(50);
        let cut = truncate(&long, 10);
        assert_eq!(cut.chars().count(), 10);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn headlines_show_cache_or_instructions() {
        let (_d, mut r) = ready();
        let empty = ok(&mut r, "");
        assert!(empty.contains("No headlines cached yet"), "{empty}");
    }

    #[test]
    fn uninitialised_and_unknown_actions() {
        let mut r = Rss::new();
        assert!(!r.handle("launch", "").ok);
        assert!(!r.handle("run", "feeds").ok);
        assert!(!r.handle("bogus", "").ok);
        let (_d, mut r) = ready();
        assert!(!r.execute("wat").ok);
        assert!(r.execute("help").ok);
    }
}
