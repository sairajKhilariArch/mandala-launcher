//! B4 Document Reader: page through plain-text and markdown files offline.
//!
//! Scope, stated plainly: this reader parses **text** (`.txt`, `.md`, `.rst`,
//! `.log`, source files, man pages, …). PDF is not decoded — `open x.pdf`
//! says so instead of pretending.
//!
//! Paths
//! * relative paths are resolved inside the module's private document
//!   directory (`<home>/data/app-reader`) and may not escape it;
//! * an absolute path (or `~/…`) is read as given — that is the user explicitly
//!   naming a file.
//!
//! Actions
//! * `launch` – list of bundled documents
//! * `run`    – `open <path>` · `next` · `prev` · `list` · `find <name>` ·
//!   `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 256 * 1024;
const PAGE_LINES: usize = 60;

pub struct Reader {
    docs: Option<PathBuf>,
    /// Currently open document and the first line of the visible page.
    current: Option<(PathBuf, usize)>,
}

impl Reader {
    pub fn new() -> Self {
        Self {
            docs: None,
            current: None,
        }
    }

    fn docs_dir(&self) -> Result<PathBuf, String> {
        self.docs
            .clone()
            .ok_or_else(|| "reader storage not initialised".to_string())
    }

    /// Resolve a relative name inside the document directory, refusing
    /// traversal; pass through absolute / home paths unchanged.
    fn resolve(docs: &Path, arg: &str) -> Result<PathBuf, String> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Err("missing file name".into());
        }
        if arg.starts_with('/') {
            return Ok(PathBuf::from(arg));
        }
        if let Some(rest) = arg.strip_prefix("~/") {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| "no HOME set".to_string())?;
            return Ok(home.join(rest));
        }
        if arg.starts_with('~') {
            return Err("`~name` is not supported; use `~/…` or a plain name".into());
        }
        let mut out = docs.to_path_buf();
        for part in arg.split(['/', '\\']) {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(
                    "relative paths may not leave the document directory (use an absolute path)"
                        .into(),
                );
            }
            out.push(part);
        }
        Ok(out)
    }

    fn read_lines(path: &Path) -> Result<Vec<String>, String> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext == "pdf" {
            return Err(
                "PDF is not decoded by this reader — export the document to .txt/.md first".into(),
            );
        }
        if ext == "epub" {
            return Err("EPUB is not decoded by this reader — export to .txt/.md first".into());
        }
        let meta = std::fs::metadata(path).map_err(|e| format!("open: {e}"))?;
        if !meta.is_file() {
            return Err(format!("open: {} is not a regular file", path.display()));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("open: file is larger than {MAX_FILE_BYTES} bytes (paged reading is limited to small text files)"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                format!("open: {} is not valid UTF-8 text", path.display())
            } else {
                format!("open: {e}")
            }
        })?;
        Ok(text.lines().map(|l| l.to_string()).collect())
    }

    fn open(&mut self, arg: &str) -> Result<String, String> {
        let docs = self.docs_dir()?;
        let path = Self::resolve(&docs, arg)?;
        let lines = Self::read_lines(&path)?;
        self.current = Some((path.clone(), 0));
        Ok(render_page(&path, &lines, 0))
    }

    fn page(&self, delta: isize) -> Result<String, String> {
        let (path, offset) = self
            .current
            .as_ref()
            .ok_or_else(|| "nothing is open (try `open <name>`)".to_string())?;
        let lines = Self::read_lines(path)?;
        let max = lines.len().saturating_sub(1);
        let next = if delta > 0 {
            offset.saturating_add(PAGE_LINES)
        } else {
            offset.saturating_sub(PAGE_LINES)
        };
        let next = next.min(max);
        Ok(render_page(path, &lines, next))
    }

    fn listing(&self) -> String {
        let dir = match self.docs_dir() {
            Ok(d) => d,
            Err(e) => return format!("READER\n\n{e}"),
        };
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let body = if names.is_empty() {
            "  (no documents yet)\n  put .txt/.md files in the module's document directory,\n  or open any file by absolute path".to_string()
        } else {
            names
                .iter()
                .map(|n| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!("READER\n\n{body}\n\nopen <name>   page through a document\nnext / prev   move one page")
    }

    fn find(&self, needle: &str) -> Result<String, String> {
        let dir = self.docs_dir()?;
        let needle = needle.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Err("find needs a name fragment".into());
        }
        let hits: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.to_ascii_lowercase().contains(&needle))
                    .collect()
            })
            .unwrap_or_default();
        match hits.len() {
            0 => Err(format!("no document matching `{needle}`")),
            _ => Ok(hits
                .iter()
                .map(|n| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")),
        }
    }

    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.docs.is_none() {
            return ModuleReply::err("reader storage not initialised");
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
            "open" | "read" | "page" => self.open(rest),
            "next" | "n" => self.page(PAGE_LINES as isize),
            "prev" | "p" => self.page(-(PAGE_LINES as isize)),
            "find" | "search" => self.find(rest),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

fn render_page(path: &Path, lines: &[String], start: usize) -> String {
    if lines.is_empty() {
        return format!("{}\n(empty file)", path.display());
    }
    let start = start.min(lines.len().saturating_sub(1));
    let end = (start + PAGE_LINES).min(lines.len());
    let more = if end < lines.len() { " … `next`" } else { "" };
    format!(
        "{} — lines {}-{} of {}{more}\n\n{}",
        path.display(),
        start + 1,
        end,
        lines.len(),
        lines[start..end].join("\n")
    )
}

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

const HELP: &str = "READER

  open <name>      open a document (relative names stay in the doc directory)
  open /abs/path   open any file you name explicitly
  next / prev      page forward / back
  list             documents in the doc directory
  find <fragment>  match document names

Text formats only (.txt, .md, .log, source files). PDF/EPUB are not decoded.";

impl Module for Reader {
    fn id(&self) -> &'static str {
        "app-reader"
    }
    fn name(&self) -> &'static str {
        "Doc Reader"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir).join("docs");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.docs = Some(dir);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => {
                if self.docs.is_none() {
                    return ModuleReply::err("reader storage not initialised");
                }
                ModuleReply::ok(self.listing())
            }
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Reader::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Reader) {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Reader::new();
        r.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-reader"),
        })
        .unwrap();
        (dir, r)
    }

    fn ok(r: &mut Reader, cmd: &str) -> String {
        let out = r.execute(cmd);
        assert!(out.ok, "{cmd}: {}", out.text);
        out.text
    }

    fn doc(r: &Reader, name: &str, body: &str) -> PathBuf {
        let p = r.docs_dir().unwrap().join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn list_and_open_local_document() {
        let (_d, mut r) = ready();
        assert!(r.handle("launch", "").text.contains("no documents yet"));
        doc(&r, "guide.md", "# Guide\nline two\nline three");
        let list = ok(&mut r, "list");
        assert!(list.contains("guide.md"), "{list}");
        let page = ok(&mut r, "open guide.md");
        assert!(
            page.contains("# Guide") && page.contains("line three"),
            "{page}"
        );
        assert!(page.contains("of 3"));
    }

    #[test]
    fn pagination_walks_the_document() {
        let (_d, mut r) = ready();
        let body: String = (0..120).map(|i| format!("line {i}\n")).collect();
        doc(&r, "long.txt", &body);
        let first = ok(&mut r, "open long.txt");
        assert!(
            first.contains("lines 1-60 of 120") && first.contains("`next`"),
            "{first}"
        );
        let second = ok(&mut r, "next");
        assert!(second.contains("lines 61-120 of 120"), "{second}");
        assert!(!second.contains("`next`"));
        let back = ok(&mut r, "prev");
        assert!(back.contains("lines 1-60 of 120"), "{back}");
    }

    #[test]
    fn traversal_is_refused_but_absolute_paths_work() {
        let (_d, mut r) = ready();
        assert!(
            !r.execute("open ../secret.txt").ok,
            "relative traversal refused"
        );
        assert!(
            !r.execute("open a/../../b.txt").ok,
            "nested traversal refused"
        );

        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("notes.txt");
        std::fs::write(&target, "outside content").unwrap();
        let page = ok(&mut r, &format!("open {}", target.display()));
        assert!(
            page.contains("outside content"),
            "explicit absolute path is allowed: {page}"
        );
    }

    #[test]
    fn paging_without_an_open_document_errors() {
        let (_d, mut r) = ready();
        let out = r.execute("next");
        assert!(
            !out.ok && out.text.contains("nothing is open"),
            "{}",
            out.text
        );
    }

    #[test]
    fn missing_file_and_pdf_are_reported() {
        let (_d, mut r) = ready();
        assert!(!r.execute("open nope.txt").ok);
        doc(&r, "manual.pdf", "%PDF-1.4 not really");
        let out = r.execute("open manual.pdf");
        assert!(!out.ok && out.text.contains("PDF"), "{}", out.text);
    }

    #[test]
    fn oversized_and_binary_files_are_rejected() {
        let (_d, mut r) = ready();
        doc(&r, "big.txt", &"a".repeat(MAX_FILE_BYTES as usize + 1));
        assert!(!r.execute("open big.txt").ok);
        let p = r.docs_dir().unwrap().join("binary.txt");
        std::fs::write(&p, [0xff, 0xd8, 0xff, 0x00, 0x9c]).unwrap();
        assert!(
            !r.execute("open binary.txt").ok,
            "non-UTF-8 is reported, not crashed"
        );
    }

    #[test]
    fn find_matches_names() {
        let (_d, mut r) = ready();
        doc(&r, "api-reference.md", "x");
        doc(&r, "notes.txt", "y");
        assert!(ok(&mut r, "find reference").contains("api-reference.md"));
        assert!(!r.execute("find zzz").ok);
        assert!(!r.execute("find").ok);
    }

    #[test]
    fn uninitialised_and_unknown_actions() {
        let mut r = Reader::new();
        assert!(!r.handle("launch", "").ok);
        assert!(!r.handle("run", "list").ok);
        assert!(!r.handle("bogus", "").ok);
        let (_d, mut r) = ready();
        assert!(!r.execute("wat").ok);
        assert!(r.execute("help").ok);
    }
}
