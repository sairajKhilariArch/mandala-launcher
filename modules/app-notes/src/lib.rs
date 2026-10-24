//! A2 Notes / Micro-Capture: an append-only plain-text buffer.
//!
//! Actions
//! * `launch`  – show the most recent notes
//! * `append`  – payload is appended as a timestamped line
//! * `clear`   – truncate the file

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_NOTE_BYTES: usize = 4096;
const SHOW_LAST: usize = 12;

#[derive(Default)]
pub struct Notes {
    file: Option<PathBuf>,
}

impl Notes {
    pub fn new() -> Self {
        Self::default()
    }

    fn path(&self) -> Result<&PathBuf, ModuleReply> {
        self.file.as_ref().ok_or_else(|| ModuleReply::err("notes storage not initialised"))
    }

    fn recent(&self) -> ModuleReply {
        let path = match self.path() {
            Ok(p) => p,
            Err(e) => return e,
        };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return ModuleReply::ok("Notes are empty.\nType `note <text>` in the command bar to capture a thought.");
        }
        let start = lines.len().saturating_sub(SHOW_LAST);
        ModuleReply::ok(format!("NOTES ({} total)\n\n{}", lines.len(), lines[start..].join("\n")))
    }

    fn append(&self, payload: &str) -> ModuleReply {
        let path = match self.path() {
            Ok(p) => p,
            Err(e) => return e,
        };
        let note = payload.trim();
        if note.is_empty() {
            return ModuleReply::err("nothing to save");
        }
        if note.len() > MAX_NOTE_BYTES {
            return ModuleReply::err(format!("note too long ({} > {MAX_NOTE_BYTES} bytes)", note.len()));
        }
        // One note == one line.
        let single_line = note.replace(['\r', '\n'], " ");
        let line = format!("{}  {}\n", timestamp(), single_line);
        let result = OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| f.write_all(line.as_bytes()));
        match result {
            Ok(()) => ModuleReply::ok(format!("Saved.\n{}", line.trim_end())),
            Err(e) => ModuleReply::err(format!("could not write notes: {e}")),
        }
    }

    fn clear(&self) -> ModuleReply {
        let path = match self.path() {
            Ok(p) => p,
            Err(e) => return e,
        };
        match std::fs::write(path, b"") {
            Ok(()) => ModuleReply::ok("Notes cleared."),
            Err(e) => ModuleReply::err(format!("could not clear notes: {e}")),
        }
    }
}

impl Module for Notes {
    fn id(&self) -> &'static str {
        "app-notes"
    }
    fn name(&self) -> &'static str {
        "Notes"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.file = Some(dir.join("notes.txt"));
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => self.recent(),
            "append" => self.append(payload),
            "clear" => self.clear(),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

/// `YYYY-MM-DD HH:MM` in UTC, without pulling in a date library.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format_utc(secs)
}

pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days algorithm.
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
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

declare_module!(Notes::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Notes) {
        let dir = tempfile::tempdir().unwrap();
        let mut n = Notes::new();
        n.init(&ModuleContext { data_dir: dir.path().to_string_lossy().into(), host: Host::detached("app-notes") }).unwrap();
        (dir, n)
    }

    #[test]
    fn empty_then_append_then_show() {
        let (_d, mut n) = ready();
        assert!(n.handle("launch", "").text.contains("empty"));
        assert!(n.handle("append", "buy milk").ok);
        let shown = n.handle("launch", "");
        assert!(shown.ok && shown.text.contains("buy milk") && shown.text.contains("1 total"));
    }

    #[test]
    fn multiline_notes_are_flattened_and_blank_rejected() {
        let (_d, mut n) = ready();
        assert!(!n.handle("append", "   ").ok);
        n.handle("append", "a\nb\r\nc");
        let shown = n.handle("launch", "").text;
        assert!(shown.contains("a b  c") || shown.contains("a b c"), "{shown}");
        assert!(shown.contains("1 total"));
    }

    #[test]
    fn oversized_note_rejected() {
        let (_d, mut n) = ready();
        assert!(!n.handle("append", &"x".repeat(MAX_NOTE_BYTES + 1)).ok);
    }

    #[test]
    fn only_last_notes_are_shown_and_clear_works() {
        let (_d, mut n) = ready();
        for i in 0..20 {
            n.handle("append", &format!("note-{i}"));
        }
        let shown = n.handle("launch", "").text;
        assert!(shown.contains("20 total") && shown.contains("note-19") && !shown.contains("note-3\n"));
        assert!(n.handle("clear", "").ok);
        assert!(n.handle("launch", "").text.contains("empty"));
    }

    #[test]
    fn uninitialised_and_unknown_actions_error() {
        let mut n = Notes::new();
        assert!(!n.handle("append", "x").ok);
        let (_d, mut n) = ready();
        assert!(!n.handle("explode", "").ok);
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_709_210_096), "2024-02-29 12:34"); // leap day
        assert_eq!(format_utc(4_102_444_800), "2100-01-01 00:00");
    }
}
