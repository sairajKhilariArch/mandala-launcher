//! B5 Media Gallery: a local, list-first image/video browser.
//!
//! Scope, stated plainly: this module **enumerates and describes** media —
//! name, type, size, and image dimensions parsed from PNG/JPEG/GIF/BMP
//! headers. It does not decode or render pixels (no image codec dependency),
//! and it never uploads anything.
//!
//! By default it scans its own document directory. `set <dir>` points it at
//! e.g. `~/Pictures` (absolute path, must exist), which is remembered.
//!
//! Actions
//! * `launch` – listing
//! * `run`    – `set <dir>` · `list [fragment]` · `info <name>` · `root` ·
//!   `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 500;
const IMAGE_EXT: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "heic"];
const VIDEO_EXT: [&str; 5] = ["mp4", "mkv", "mov", "webm", "avi"];

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
struct Config {
    #[serde(default)]
    root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: &'static str,
    pub bytes: u64,
    pub dims: Option<(u32, u32)>,
}

pub struct Gallery {
    root: PathBuf,
    /// Directory actually scanned (module dir until `set` is used).
    current: PathBuf,
    truncated: bool,
}

impl Gallery {
    pub fn new() -> Self {
        Self {
            root: PathBuf::new(),
            current: PathBuf::new(),
            truncated: false,
        }
    }

    fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    fn load_config(file: &Path) -> Option<Config> {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    }

    /// `set <dir>`: absolute, existing directory.
    fn set_root(&mut self, dir: &str) -> Result<String, String> {
        let dir = dir.trim();
        if dir.is_empty() {
            return Err("usage: set <absolute directory>".into());
        }
        let path = PathBuf::from(dir);
        if !path.is_absolute() {
            return Err(
                "use an absolute path, e.g. set ~/Pictures (expanded by your shell)".into(),
            );
        }
        let meta = std::fs::metadata(&path).map_err(|e| format!("set: {e}"))?;
        if !meta.is_dir() {
            return Err(format!("set: {path:?} is not a directory"));
        }
        self.current = path.clone();
        let cfg = Config {
            root: path.to_string_lossy().into_owned(),
        };
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        write_private(&self.config_file(), json.as_bytes()).map_err(|e| format!("set: {e}"))?;
        Ok(format!("scanning {}", path.display()))
    }

    fn scan(&mut self, fragment: &str) -> Vec<Entry> {
        let needle = fragment.trim().to_ascii_lowercase();
        let mut entries: Vec<Entry> = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.current) else {
            return entries;
        };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        self.truncated = false;
        for path in paths {
            let meta = match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => m,
                _ => continue,
            };
            let name = match path.file_name().map(|n| n.to_string_lossy().into_owned()) {
                Some(n) => n,
                None => continue,
            };
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let kind = classify(&ext);
            if kind.is_none() {
                continue;
            }
            if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            if entries.len() >= MAX_ENTRIES {
                self.truncated = true;
                break;
            }
            let bytes = meta.len();
            let dims = if kind == Some("image") {
                probe_dimensions(&path)
            } else {
                None
            };
            entries.push(Entry {
                name,
                kind: kind.unwrap_or("file"),
                bytes,
                dims,
            });
        }
        entries
    }

    fn listing(&mut self, fragment: &str) -> String {
        let entries = self.scan(fragment);
        let mut out = format!("GALLERY — {}\n", self.current.display());
        if let Some(c) = Gallery::load_config(&self.config_file()) {
            if !c.root.is_empty() && c.root != self.current.to_string_lossy() {
                out.push_str(&format!("(remembered root: {})\n", c.root));
            }
        }
        out.push('\n');
        if entries.is_empty() {
            let body = if fragment.is_empty() {
                "No images or videos here.\n  set <dir>       point at a directory, e.g. your Pictures folder\n  list <fragment> filter by name".to_string()
            } else {
                format!("No media matching `{fragment}`.")
            };
            out.push_str(&body);
            return out;
        }
        for e in &entries {
            let dims = match e.dims {
                Some((w, h)) => format!("{w}x{h}"),
                None => "—".into(),
            };
            out.push_str(&format!(
                "{:<9} {:>10}  {:>10}  {}\n",
                e.kind,
                human(e.bytes),
                dims,
                e.name
            ));
        }
        out.push_str(&format!("\n{} file(s)", entries.len()));
        if self.truncated {
            out.push_str(&format!(" (first {MAX_ENTRIES} only)"));
        }
        out
    }

    fn info(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("usage: info <file name>".into());
        }
        let hits = self.scan(name);
        match hits.len() {
            0 => Err(format!("no media matching `{name}`")),
            1 => {
                let e = &hits[0];
                let dims = match e.dims {
                    Some((w, h)) => format!("{w} x {h} px"),
                    None => "dimensions not parsed for this format".into(),
                };
                Ok(format!("{}\n  type      {}\n  size      {} ({} bytes)\n  dimensions {}\n  path      {}",
                    e.name, e.kind, human(e.bytes), e.bytes, dims, self.current.display()))
            }
            _ => {
                let list: Vec<String> = hits.iter().map(|e| format!("  {}", e.name)).collect();
                Err(format!(
                    "`{name}` matches several files:\n{}",
                    list.join("\n")
                ))
            }
        }
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.root.as_os_str().is_empty() {
            return ModuleReply::err("gallery storage not initialised");
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.listing(""));
        }
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "list" | "ls" | "show" => Ok(self.listing(rest)),
            "info" | "stat" => self.info(rest),
            "set" => self.set_root(rest),
            "root" => Ok(format!("{}", self.current.display())),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Gallery {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "GALLERY

  set <dir>         scan an absolute directory (remembered)
  root              show the directory being scanned
  list [fragment]   media in the directory, filtered by name
  info <name>       size and pixel dimensions of one file

Local files only; nothing is uploaded. Images are identified from their
header bytes, so sizes are exact and dimensions cover PNG/JPEG/GIF/BMP.";

/// `Some("image" | "video")` for media extensions.
fn classify(ext: &str) -> Option<&'static str> {
    if IMAGE_EXT.contains(&ext) {
        Some("image")
    } else if VIDEO_EXT.contains(&ext) {
        Some("video")
    } else {
        None
    }
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Read just enough bytes to recover the pixel dimensions of common formats.
pub fn probe_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut buf = [0u8; 64];
    let mut f = std::fs::File::open(path).ok()?;
    use std::io::Read;
    let n = f.read(&mut buf).ok()?;
    let b = &buf[..n];
    png_dims(b)
        .or_else(|| jpeg_dims(b, path))
        .or_else(|| gif_dims(b))
        .or_else(|| bmp_dims(b))
}

fn png_dims(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// JPEG: walk segments until a Start-Of-Frame marker carries the size.
/// Needs the file itself, since the header alone rarely contains SOF.
fn jpeg_dims(head: &[u8], path: &Path) -> Option<(u32, u32)> {
    if head.len() < 3 || head[0] != 0xff || head[1] != 0xd8 {
        return None;
    }
    let mut bytes = Vec::with_capacity(8 * 1024);
    bytes.extend_from_slice(head);
    let mut f = std::fs::File::open(path).ok()?;
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(head.len() as u64)).ok()?;
    let mut rest = [0u8; 64 * 1024];
    let n = f.read(&mut rest).ok()?;
    bytes.extend_from_slice(&rest[..n]);

    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if (0xd0..=0xd9).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let is_sof = matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        );
        if is_sof && i + 9 <= bytes.len() {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

fn gif_dims(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 10 {
        return None;
    }
    let magic = &b[..6];
    if magic != b"GIF87a" && magic != b"GIF89a" {
        return None;
    }
    let w = u16::from_le_bytes([b[6], b[7]]) as u32;
    let h = u16::from_le_bytes([b[8], b[9]]) as u32;
    (w > 0 && h > 0).then_some((w, h))
}

fn bmp_dims(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 26 || &b[..2] != b"BM" {
        return None;
    }
    let w = i32::from_le_bytes(b[18..22].try_into().ok()?) as i64;
    let h = i32::from_le_bytes(b[22..26].try_into().ok()?) as i64;
    (w > 0 && h != 0).then_some((w as u32, h.unsigned_abs() as u32))
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
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

impl Module for Gallery {
    fn id(&self) -> &'static str {
        "app-gallery"
    }
    fn name(&self) -> &'static str {
        "Gallery"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut current = dir.clone();
        if let Some(cfg) = Gallery::load_config(&dir.join("config.json")) {
            let saved = PathBuf::from(&cfg.root);
            if saved.is_dir() {
                current = saved;
            }
        }
        self.root = dir;
        self.current = current;
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" if !self.root.as_os_str().is_empty() => ModuleReply::ok(self.listing("")),
            "launch" => ModuleReply::err("gallery storage not initialised"),
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Gallery::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Gallery) {
        let dir = tempfile::tempdir().unwrap();
        let mut g = Gallery::new();
        g.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-gallery"),
        })
        .unwrap();
        (dir, g)
    }

    fn ok(g: &mut Gallery, cmd: &str) -> String {
        let out = g.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]); // depth/colour/etc.
        b
    }

    #[test]
    fn dimension_parsing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.png");
        std::fs::write(&p, png_bytes(800, 600)).unwrap();
        assert_eq!(probe_dimensions(&p), Some((800, 600)));

        let p = d.path().join("a.gif");
        std::fs::write(&p, b"GIF89a\x40\x01\xf0\x00").unwrap();
        assert_eq!(probe_dimensions(&p), Some((320, 240)));

        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0u8; 16]);
        bmp.extend_from_slice(&500i32.to_le_bytes()); // width @18
        bmp.extend_from_slice(&(-300i32).to_le_bytes()); // top-down height @22
        let p = d.path().join("a.bmp");
        std::fs::write(&p, bmp).unwrap();
        assert_eq!(probe_dimensions(&p), Some((500, 300)));

        let p = d.path().join("a.jpg");
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]; // APP0 header
        jpg.extend_from_slice(&[0u8; 14]);
        jpg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08, 0x02, 0x58, 0x03, 0x20]); // SOF0: 600x800
        std::fs::write(&p, jpg).unwrap();
        assert_eq!(probe_dimensions(&p), Some((800, 600)));

        let p = d.path().join("nothing.bin");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(probe_dimensions(&p), None);
    }

    #[test]
    fn default_directory_lists_media_and_filters() {
        let (dir, mut g) = ready();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(media.join("one.png"), png_bytes(64, 64)).unwrap();
        std::fs::write(media.join("two.mp4"), vec![0u8; 2048]).unwrap();
        std::fs::write(media.join("notes.txt"), b"not media").unwrap();

        let list = ok(&mut g, &format!("set {}", media.display()));
        assert!(list.contains("scanning"), "{list}");
        let list = ok(&mut g, "list");
        assert!(
            list.contains("one.png") && list.contains("two.mp4"),
            "{list}"
        );
        assert!(!list.contains("notes.txt"), "non-media filtered out");
        assert!(list.contains("64x64"), "{list}");
        assert!(list.contains("2.0 KiB"), "{list}");
        assert!(list.contains("2 file(s)"));

        let filtered = ok(&mut g, "list one");
        assert!(
            filtered.contains("one.png") && !filtered.contains("two.mp4"),
            "{filtered}"
        );

        let info = ok(&mut g, "info one.png");
        assert!(
            info.contains("image") && info.contains("64 x 64 px"),
            "{info}"
        );
    }

    #[test]
    fn set_root_validation() {
        let (dir, mut g) = ready();
        assert!(!g.execute("set relative/path").ok, "absolute only");
        assert!(!g.execute("set /definitely/not/here").ok, "must exist");
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        assert!(
            !g.execute(&format!("set {}", file.display())).ok,
            "must be a directory"
        );
        assert!(!g.execute("set").ok);
        assert!(g.execute("root").ok);
    }

    #[test]
    fn root_is_remembered_across_instances() {
        let (dir, mut g) = ready();
        let media = tempfile::tempdir().unwrap();
        ok(&mut g, &format!("set {}", media.path().display()));
        let mut second = Gallery::new();
        second
            .init(&ModuleContext {
                data_dir: dir.path().to_string_lossy().into(),
                host: Host::detached("app-gallery"),
            })
            .unwrap();
        assert_eq!(second.current, media.path());
    }

    #[test]
    fn empty_results_and_bad_input() {
        let (_d, mut g) = ready();
        assert!(g.handle("launch", "").text.contains("No images"));
        assert!(!g.execute("info ghost.png").ok);
        assert!(!g.execute("info").ok);
        assert!(!g.execute("wat").ok);
        assert!(!g.handle("bogus", "").ok);
        assert!(g.execute("help").ok);
    }

    #[test]
    fn uninitialised_gallery() {
        let mut g = Gallery::new();
        assert!(!g.handle("launch", "").ok);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2.0 KiB");
        assert_eq!(human(1024 * 1024 * 3), "3.0 MiB");
    }
}
