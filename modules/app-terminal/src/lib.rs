//! A1 Terminal: an embedded, sandboxed command shell.
//!
//! The terminal deliberately does **not** spawn processes. Mandala modules run
//! inside the launcher process, so handing user input to `sh -c` would give
//! the command bar the full privileges of the host. Instead this module
//! implements a small, self-contained command set over a private directory
//! (`<home>/data/app-terminal`), which covers the launcher's needs (quick
//! files, timestamps, history) without a shell escape.
//!
//! Actions
//! * `launch` – banner + directory listing
//! * `run`    – payload is one command line, e.g. `ls -l` or `echo hi`
//!
//! Paths are relative to the module's data directory. Absolute paths and
//! `..` are rejected, and symlinks that point outside the sandbox are refused.

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LINE: usize = 2048;
const MAX_READ: u64 = 64 * 1024;
const MAX_WRITE: usize = 64 * 1024;
const HISTORY_LEN: usize = 32;
const LIST_LIMIT: usize = 100;

const HELP: &str = "\
TERMINAL

  help                 this text
  ls [dir]             list files
  cat <file>           print a file
  write <file> <text>  create/overwrite a file
  append <file> <text> append one line
  rm <file>            delete a file
  wc <file>            line/word/byte count
  pwd                  sandbox root
  echo <text>          print text
  date                 current UTC time
  uname                platform information
  history              command history
  clear                clear the screen

Files live in the module's private data directory. Absolute paths, `..` and
process execution are intentionally unavailable.";

#[derive(Default)]
pub struct Terminal {
    root: Option<PathBuf>,
    history: Vec<String>,
}

impl Terminal {
    pub fn new() -> Self {
        Self::default()
    }

    fn root(&self) -> Result<&Path, String> {
        self.root
            .as_deref()
            .ok_or_else(|| "terminal storage not initialised".to_string())
    }

    /// Resolve `arg` inside the sandbox, refusing traversal and escaping
    /// symlinks.
    fn resolve(root: &Path, arg: &str) -> Result<PathBuf, String> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Ok(root.to_path_buf());
        }
        if arg.starts_with('/') || arg.starts_with('\\') {
            return Err("absolute paths are not available in the terminal".into());
        }
        let mut out = root.to_path_buf();
        for part in arg.split(['/', '\\']) {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err("`..` is not available in the terminal".into());
            }
            if part.contains('\0') {
                return Err("invalid path".into());
            }
            out.push(part);
        }
        Self::ensure_inside(root, &out)?;
        Ok(out)
    }

    /// Canonicalise the deepest existing ancestor and require it to stay below
    /// `root`. This catches `ln -s /etc etc` style escapes.
    fn ensure_inside(root: &Path, path: &Path) -> Result<(), String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("sandbox unavailable: {e}"))?;
        let mut probe = path;
        loop {
            if let Ok(real) = probe.canonicalize() {
                return if real.starts_with(&root) {
                    Ok(())
                } else {
                    Err("path escapes the terminal sandbox".into())
                };
            }
            match probe.parent() {
                Some(p) => probe = p,
                None => return Err("path escapes the terminal sandbox".into()),
            }
        }
    }

    fn list(&self, dir: &str) -> Result<String, String> {
        let root = self.root()?;
        let path = Self::resolve(root, dir)?;
        let rd = std::fs::read_dir(&path).map_err(|e| format!("ls: {e}"))?;
        let mut entries: Vec<(String, String)> = Vec::new();
        for entry in rd.flatten().take(LIST_LIMIT) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let (kind, size) = match entry.metadata() {
                Ok(m) if m.is_dir() => ("dir", String::new()),
                Ok(m) => ("file", format!("{:>8}", m.len())),
                Err(_) => ("?", String::new()),
            };
            entries.push((name, format!("{kind:<4} {size}")));
        }
        if entries.is_empty() {
            return Ok(format!("{}\n(empty)", self.display(&path)));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let rows: Vec<String> = entries
            .into_iter()
            .map(|(name, meta)| format!("{meta}  {name}"))
            .collect();
        Ok(format!("{}\n{}", self.display(&path), rows.join("\n")))
    }

    fn display(&self, path: &Path) -> String {
        match self.root() {
            Ok(root) => match path.strip_prefix(root) {
                Ok(rel) if rel.as_os_str().is_empty() => "~".into(),
                Ok(rel) => format!("~/{}", rel.display()),
                Err(_) => "~".into(),
            },
            Err(_) => "~".into(),
        }
    }

    fn read_file(&self, file: &str) -> Result<String, String> {
        let root = self.root()?.to_path_buf();
        let path = Self::resolve(&root, file)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("cat: {e}"))?;
        if !meta.is_file() {
            return Err(format!("cat: {} is not a file", file));
        }
        if meta.len() > MAX_READ {
            return Err(format!("cat: file is larger than {MAX_READ} bytes"));
        }
        std::fs::read_to_string(&path).map_err(|e| format!("cat: {e}"))
    }

    fn write_file(&self, file: &str, text: &str, append: bool) -> Result<String, String> {
        if file.trim().is_empty() {
            return Err("write: missing file name".into());
        }
        if text.len() > MAX_WRITE {
            return Err(format!("write: text is larger than {MAX_WRITE} bytes"));
        }
        let root = self.root()?.to_path_buf();
        let path = Self::resolve(&root, file)?;
        if path.is_dir() {
            return Err(format!("write: {} is a directory", file));
        }
        let separator = if append && path.exists() && !ends_with_newline(&path) {
            "\n"
        } else {
            ""
        };
        let body = if append {
            format!("{separator}{text}\n")
        } else {
            text.to_string()
        };
        let result = if append {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut f| f.write_all(body.as_bytes()))
        } else {
            std::fs::write(&path, body.as_bytes())
        };
        result.map_err(|e| format!("write: {e}"))?;
        Ok(format!(
            "wrote {} bytes to {}",
            body.len(),
            self.display(&path)
        ))
    }

    fn remove(&self, file: &str) -> Result<String, String> {
        let root = self.root()?.to_path_buf();
        let path = Self::resolve(&root, file)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("rm: {e}"))?;
        if meta.is_dir() {
            return Err("rm: directories are not removable from the terminal".into());
        }
        std::fs::remove_file(&path).map_err(|e| format!("rm: {e}"))?;
        Ok(format!("removed {}", self.display(&path)))
    }

    fn count(&self, file: &str) -> Result<String, String> {
        let text = self.read_file(file)?;
        Ok(format!(
            "{:>6} {:>6} {:>8}  {}",
            text.lines().count(),
            text.split_whitespace().count(),
            text.len(),
            file
        ))
    }

    fn banner(&self) -> String {
        let listing = self.list("").unwrap_or_else(|e| e);
        format!("{HELP}\n\n{listing}")
    }

    /// Execute one command line. Errors are returned as replies, never panics.
    pub fn execute(&mut self, line: &str) -> ModuleReply {
        let line = line.trim();
        if line.is_empty() {
            return ModuleReply::ok(String::new());
        }
        if line.len() > MAX_LINE {
            return ModuleReply::err(format!("command is longer than {MAX_LINE} characters"));
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_LEN {
            self.history.remove(0);
        }
        match self.run(line) {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }

    fn run(&mut self, line: &str) -> Result<String, String> {
        let (head, rest) = split_head(line);
        let rest = rest.trim();
        match head.to_ascii_lowercase().as_str() {
            "help" | "?" => Ok(HELP.to_string()),
            "clear" => Ok(String::new()),
            "echo" => Ok(rest.to_string()),
            "pwd" => Ok(self.display(self.root()?)),
            "date" | "time" => Ok(format_utc(now_secs())),
            "uname" => Ok(format!(
                "mandala-terminal 1 ({} {}, abi {})",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::env::consts::FAMILY
            )),
            "history" => {
                if self.history.is_empty() {
                    Ok("(no history)".into())
                } else {
                    Ok(self
                        .history
                        .iter()
                        .enumerate()
                        .map(|(i, c)| format!("{:>3}  {c}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n"))
                }
            }
            "ls" | "dir" => self.list(rest),
            "cat" | "type" => {
                if rest.is_empty() {
                    return Err("cat: missing file name".into());
                }
                self.read_file(rest)
            }
            "write" => {
                let (file, text) = split_head(rest);
                if file.is_empty() {
                    return Err("write: missing file name".into());
                }
                self.write_file(file, text.trim_start(), false)
            }
            "append" => {
                let (file, text) = split_head(rest);
                if file.is_empty() {
                    return Err("append: missing file name".into());
                }
                self.write_file(file, text.trim_start(), true)
            }
            "rm" | "del" => {
                if rest.is_empty() {
                    return Err("rm: missing file name".into());
                }
                self.remove(rest)
            }
            "wc" => {
                if rest.is_empty() {
                    return Err("wc: missing file name".into());
                }
                self.count(rest)
            }
            other => Err(format!("unknown command `{other}` (try `help`)")),
        }
    }
}

/// True when the file's last byte is `\n` (empty files count as "no").
fn ends_with_newline(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    if len == 0 {
        return false;
    }
    if f.seek(SeekFrom::Start(len - 1)).is_err() {
        return false;
    }
    let mut b = [0u8; 1];
    matches!(f.read(&mut b), Ok(1)) && b[0] == b'\n'
}

/// Split the first whitespace-delimited word from the remainder.
fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC (Howard Hinnant's civil-from-days algorithm).
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

impl Module for Terminal {
    fn id(&self) -> &'static str {
        "app-terminal"
    }
    fn name(&self) -> &'static str {
        "Terminal"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.root = Some(dir);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => {
                if self.root.is_none() {
                    return ModuleReply::err("terminal storage not initialised");
                }
                ModuleReply::ok(self.banner())
            }
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Terminal::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Terminal) {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Terminal::new();
        t.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-terminal"),
        })
        .unwrap();
        (dir, t)
    }

    fn run(t: &mut Terminal, line: &str) -> String {
        let r = t.handle("run", line);
        assert!(r.ok, "{line}: {}", r.text);
        r.text
    }

    #[test]
    fn banner_and_help() {
        let (_d, mut t) = ready();
        assert!(t.handle("launch", "").text.contains("TERMINAL"));
        assert!(run(&mut t, "help").contains("write <file> <text>"));
    }

    #[test]
    fn write_read_append_wc_rm() {
        let (_d, mut t) = ready();
        run(&mut t, "write hello.txt hi there");
        assert_eq!(run(&mut t, "cat hello.txt"), "hi there");
        run(&mut t, "append hello.txt second line");
        assert_eq!(run(&mut t, "cat hello.txt"), "hi there\nsecond line\n");
        let wc = run(&mut t, "wc hello.txt");
        assert!(wc.contains("2") && wc.contains("hello.txt"), "{wc}");
        assert!(run(&mut t, "ls").contains("hello.txt"));
        run(&mut t, "rm hello.txt");
        assert!(!t.handle("run", "cat hello.txt").ok);
    }

    #[test]
    fn sandbox_blocks_traversal_absolute_paths_and_symlinks() {
        let (dir, mut t) = ready();
        for bad in [
            "cat ../secret",
            "cat /etc/passwd",
            "cat a/../../b",
            "write ../x y",
            "ls ..",
        ] {
            assert!(!t.handle("run", bad).ok, "{bad} should be rejected");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.path().join("etc")).unwrap();
            let r = t.handle("run", "cat etc/passwd");
            assert!(!r.ok, "symlink escape must be rejected: {r:?}");
        }
    }

    #[test]
    fn unknown_and_empty_commands() {
        let (_d, mut t) = ready();
        assert!(!t.handle("run", "sudo reboot").ok);
        assert_eq!(t.handle("run", "   ").text, "");
        assert!(!t.handle("explode", "").ok);
    }

    #[test]
    fn history_is_bounded_and_numbered() {
        let (_d, mut t) = ready();
        for i in 0..(HISTORY_LEN + 5) {
            run(&mut t, &format!("echo {i}"));
        }
        let h = run(&mut t, "history");
        assert_eq!(h.lines().count(), HISTORY_LEN);
        assert!(h.contains(&format!("echo {}", HISTORY_LEN + 4)));
        assert!(!h.contains("echo 0\n"));
    }

    #[test]
    fn oversized_input_is_rejected() {
        let (_d, mut t) = ready();
        assert!(!t.handle("run", &"x".repeat(MAX_LINE + 1)).ok);
        run(&mut t, "write big.txt start");
        let huge = format!("write big.txt {}", "y".repeat(MAX_WRITE + 1));
        assert!(!t.handle("run", &huge).ok);
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_709_210_096), "2024-02-29 12:34:56 UTC");
    }

    #[test]
    fn path_components_are_not_smuggled() {
        // Use a real temporary directory so the test also works on Windows,
        // where /tmp is not a conventional existing path.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        assert!(Terminal::resolve(root, "a\\..\\b").is_err());
        assert!(Terminal::resolve(root, "./a/b/../c").is_err());
        let p = Terminal::resolve(root, "a/./b/").unwrap();
        assert_eq!(p, root.join("a/b"));
    }

    #[test]
    fn uninitialised_terminal_errors() {
        let mut t = Terminal::new();
        assert!(!t.handle("launch", "").ok);
        assert!(!t.handle("run", "ls").ok);
    }
}
