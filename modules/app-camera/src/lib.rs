//! C5 Camera Capture: the destination half of the camera pipeline.
//!
//! Division of labour, stated plainly: a `cdylib` module cannot open a camera
//! — Android requires an `Intent` fired from the Activity, and desktop would
//! need V4L2/ffmpeg plumbing the launcher deliberately does not link. So:
//!
//! * the **shell** owns capture. `camera` in the command bar becomes
//!   `Effect::Camera`, which on Android fires `MediaStore.ACTION_IMAGE_CAPTURE`
//!   with `EXTRA_OUTPUT` pointed at this module's captures directory (so the
//!   default camera app — Open Camera or any other — writes straight to disk,
//!   exactly as the roadmap asks). On desktop it reports "Android only".
//! * this **module** owns the destination: it creates
//!   `<data>/app-camera/captures`, lists what landed there, describes files
//!   and deletes mistakes. B5 Gallery scans the same directory, so a capture
//!   shows up in the gallery immediately.
//!
//! Actions
//! * `launch` – captures directory + the most recent files
//! * `run`    – `list [fragment]` · `path` · `info <name>` · `rm <name>` ·
//!   `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use std::path::{Path, PathBuf};

/// Subdirectory of the module data dir that captures land in. The shell
/// computes the same path for `EXTRA_OUTPUT`; keep the two in sync.
pub const CAPTURES_DIR: &str = "captures";
const MAX_LIST: usize = 50;

pub struct Camera {
    captures: PathBuf,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            captures: PathBuf::new(),
        }
    }

    fn ensure_ready(&self) -> Result<&Path, ModuleReply> {
        if self.captures.as_os_str().is_empty() {
            return Err(ModuleReply::err("camera storage not initialised"));
        }
        Ok(&self.captures)
    }

    fn listing(&self, fragment: &str) -> String {
        let needle = fragment.trim().to_ascii_lowercase();
        let mut files: Vec<(String, u64)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.captures) {
            for entry in rd.flatten() {
                let path = entry.path();
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if !is_media(&name) {
                    continue;
                }
                if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                files.push((name, meta.len()));
            }
        }
        files.sort_by(|a, b| b.0.cmp(&a.0)); // newest-looking names first
        let total = files.len();
        let mut out = format!("CAPTURES — {}\n\n", self.captures.display());
        if files.is_empty() {
            out.push_str(
                "Nothing captured yet.\n\n  camera           fire the camera (command bar)\n  list <fragment>  filter by name\n\nCaptures are written straight to this directory and appear in\nGallery (B5) immediately. On desktop the capture intent is\nAndroid-only; files can still be copied here manually.",
            );
            return out;
        }
        for (name, bytes) in files.iter().take(MAX_LIST) {
            out.push_str(&format!("{:>10}  {}\n", human(*bytes), name));
        }
        if total > MAX_LIST {
            out.push_str(&format!(
                "… and {} more (use `list <fragment>`)\n",
                total - MAX_LIST
            ));
        }
        out.push_str(&format!("\n{total} file(s)"));
        out
    }

    fn info(&self, name: &str) -> Result<String, String> {
        let path = self.resolve(name)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("info: {e}"))?;
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let kind = if matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic"
        ) {
            "image"
        } else if matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "webm" | "avi") {
            "video"
        } else {
            "file"
        };
        let mut dims = String::new();
        if kind == "image" {
            if let Some((w, h)) = probe_dimensions(&path) {
                dims = format!("   {w}x{h}");
            }
        }
        Ok(format!(
            "{}\n  kind    {kind}{dims}\n  size    {}\n  path    {}",
            name,
            human(meta.len()),
            path.display()
        ))
    }

    fn remove(&self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("usage: rm <file name>".into());
        }
        let path = self.resolve(name)?;
        std::fs::remove_file(&path).map_err(|e| format!("rm: {e}"))?;
        Ok(format!("deleted {}", path.display()))
    }

    /// Resolve a file name inside the captures dir, rejecting any path that
    /// would escape it (`..`, separators, absolute paths).
    fn resolve(&self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("file name required".into());
        }
        if name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || Path::new(name).is_absolute()
        {
            return Err("plain file names only (no paths)".into());
        }
        let path = self.captures.join(name);
        if !path.starts_with(&self.captures) {
            return Err("refusing to leave the captures directory".into());
        }
        Ok(path)
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if let Err(e) = self.ensure_ready() {
            return e;
        }
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(self.listing(""));
        }
        let (head, rest) = match line.find(char::is_whitespace) {
            Some(i) => (&line[..i], line[i..].trim()),
            None => (line, ""),
        };
        let outcome: Result<String, String> = match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "path" => Ok(self.captures.display().to_string()),
            "list" | "ls" => Ok(self.listing(rest)),
            "info" => self.info(rest),
            "rm" | "delete" => self.remove(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "CAMERA

  (command bar)  camera        fire the camera app; output lands here
  list [name]    recent captures, newest first
  info <name>    size and dimensions of one capture
  rm <name>      delete a capture
  path           print the captures directory

Capture itself is a platform action: the launcher fires the Android camera
intent (works with Open Camera or any default camera app) and points it at
this directory. Desktop builds report that capture is Android-only; the
directory and review tools still work everywhere.";

/// Only images and videos are listed; stray files in the captures directory
/// (`.nomedia`, partial writes) are ignored.
fn is_media(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "webp"
            | "heic"
            | "mp4"
            | "mkv"
            | "mov"
            | "webm"
            | "avi"
    )
}

fn human(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (1 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / (1 << 10) as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Read image dimensions from the file header (PNG/GIF/BMP/JPEG SOF).
fn probe_dimensions(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 32];
    let n = f.read(&mut head).ok()?;
    // GIF needs only the 10-byte logical screen descriptor; the rest need 24+.
    if head.starts_with(b"GIF8") {
        if n < 10 {
            return None;
        }
        return Some((
            u32::from(u16::from_le_bytes([head[6], head[7]])),
            u32::from(u16::from_le_bytes([head[8], head[9]])),
        ));
    }
    if n < 24 {
        return None;
    }
    // PNG: IHDR at fixed offset 16.
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some((
            u32::from_be_bytes([head[16], head[17], head[18], head[19]]),
            u32::from_be_bytes([head[20], head[21], head[22], head[23]]),
        ));
    }
    // BMP: DIB header, width/height are signed 32-bit LE at 18/22.
    if head.starts_with(b"BM") {
        let w = i32::from_le_bytes([head[18], head[19], head[20], head[21]]);
        let h = i32::from_le_bytes([head[22], head[23], head[24], head[25]]).abs();
        return Some((w as u32, h as u32));
    }
    // JPEG: scan for a SOF marker and read the frame header.
    if head.starts_with(&[0xFF, 0xD8]) {
        let mut file = std::fs::File::open(path).ok()?;
        let mut buf = Vec::with_capacity(8192);
        file.by_ref().take(64 * 1024).read_to_end(&mut buf).ok()?;
        let mut i = 2;
        while i + 9 < buf.len() {
            if buf[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = buf[i + 1];
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let h = u16::from_be_bytes([buf[i + 5], buf[i + 6]]);
                let w = u16::from_be_bytes([buf[i + 7], buf[i + 8]]);
                return Some((u32::from(w), u32::from(h)));
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
            i += 2 + len;
        }
    }
    None
}

impl Module for Camera {
    fn id(&self) -> &'static str {
        "app-camera"
    }
    fn name(&self) -> &'static str {
        "Camera"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir).join(CAPTURES_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.captures = dir;
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" | "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Camera::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Camera) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Camera::new();
        c.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-camera"),
        })
        .unwrap();
        (dir, c)
    }

    fn ok(c: &mut Camera, cmd: &str) -> String {
        let out = c.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    /// A 1x1 PNG (the canonical smallest valid PNG).
    const PNG_1X1: [u8; 67] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn empty_state_explains_the_pipeline() {
        let (_d, mut c) = ready();
        let out = ok(&mut c, "");
        assert!(
            out.contains("Nothing captured yet") && out.contains("camera"),
            "{out}"
        );
        assert!(ok(&mut c, "path").ends_with("captures"));
        assert!(ok(&mut c, "help").contains("Open Camera"));
    }

    #[test]
    fn listings_info_and_removal() {
        let (d, mut c) = ready();
        let cap = d.path().join("captures");
        std::fs::write(cap.join("IMG_0002.jpg"), vec![0u8; 2048]).unwrap();
        std::fs::write(cap.join("IMG_0001.png"), PNG_1X1).unwrap();
        std::fs::write(cap.join("notes.txt"), b"not media").unwrap();

        let out = ok(&mut c, "list");
        assert!(
            out.contains("IMG_0002.jpg") && out.contains("2.0 KiB"),
            "{out}"
        );
        assert!(out.contains("2 file(s)"), "non-media excluded: {out}");
        assert!(!out.contains("notes.txt"), "{out}");

        let filtered = ok(&mut c, "list img_0001");
        assert!(
            filtered.contains("IMG_0001.png") && !filtered.contains("IMG_0002"),
            "{filtered}"
        );

        let info = ok(&mut c, "info IMG_0001.png");
        assert!(info.contains("image") && info.contains("1x1"), "{info}");
        assert!(!c.execute("info missing.jpg").ok);

        assert!(!c.execute("rm").ok);
        assert!(ok(&mut c, "rm IMG_0002.jpg").contains("deleted"));
        assert!(!ok(&mut c, "list").contains("IMG_0002.jpg"));
    }

    #[test]
    fn path_escapes_are_rejected() {
        let (_d, mut c) = ready();
        for bad in ["../secret", "/etc/passwd", "a/b.png", "..\\x"] {
            let out = c.execute(&format!("info {bad}"));
            assert!(!out.ok, "{bad} must be rejected");
            let out = c.execute(&format!("rm {bad}"));
            assert!(!out.ok, "{bad} must be rejected");
        }
        assert!(!c.execute("explode").ok);
    }

    #[test]
    fn dimension_probes() {
        let d = tempfile::tempdir().unwrap();
        let png = d.path().join("a.png");
        std::fs::write(&png, PNG_1X1).unwrap();
        assert_eq!(probe_dimensions(&png), Some((1, 1)));

        let gif = d.path().join("a.gif");
        std::fs::write(&gif, b"GIF89a\x04\x00\x03\x00\x00").unwrap();
        assert_eq!(probe_dimensions(&gif), Some((4, 3)));

        let bmp = d.path().join("a.bmp");
        let mut bytes = vec![b'B', b'M'];
        bytes.extend_from_slice(&[0u8; 16]); // to offset 18: BM(2) + 16 pad
        bytes.extend_from_slice(&2i32.to_le_bytes()); // width
        bytes.extend_from_slice(&(-3i32).to_le_bytes()); // height (negative = upright)
        std::fs::write(&bmp, &bytes).unwrap();
        assert_eq!(probe_dimensions(&bmp), Some((2, 3)));

        assert_eq!(
            probe_dimensions(d.path().join("missing.png").as_path()),
            None
        );
    }

    #[test]
    fn uninitialised_module_errors() {
        let mut c = Camera::new();
        assert!(!c.handle("launch", "").ok);
        assert!(!c.handle("run", "list").ok);
        assert!(!c.handle("bogus", "").ok);
    }
}
