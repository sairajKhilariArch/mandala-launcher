//! B3 Task & Habit Tracker: a plain-text todo list plus streak-based habits.
//!
//! Everything lives in one JSON file in the module's private data directory
//! (mode 0600 on Unix). No network, no accounts, no sync.
//!
//! Actions
//! * `launch` – list (same as `run list`)
//! * `run`    – one command line:
//!   `add <text>` · `habit <text>` · `done <n>` · `undo <n>` · `rm <n>` ·
//!   `clean` (drop completed tasks) · `list` · `help`

use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TEXT: usize = 160;
const MAX_ITEMS: usize = 200;
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: u32,
    pub text: String,
    #[serde(default)]
    pub habit: bool,
    #[serde(default)]
    pub done: bool,
    /// Consecutive days the habit was completed.
    #[serde(default)]
    pub streak: u32,
    #[serde(default)]
    pub best: u32,
    #[serde(default)]
    pub done_day: Option<u64>,
}

pub struct Tasks {
    file: Option<PathBuf>,
    items: Vec<Task>,
    /// Days since the Unix epoch; injectable so streaks are testable.
    today: u64,
    warning: Option<String>,
}

impl Tasks {
    pub fn new() -> Self {
        Self {
            file: None,
            items: Vec::new(),
            today: today_of_now(),
            warning: None,
        }
    }

    /// Test seam: pretend it is another day.
    pub fn set_day(&mut self, day: u64) {
        self.today = day;
    }

    fn path(&self) -> Result<&Path, String> {
        self.file
            .as_deref()
            .ok_or_else(|| "tasks storage not initialised".to_string())
    }

    fn load(file: &Path) -> (Vec<Task>, Option<String>) {
        match std::fs::read_to_string(file) {
            Ok(text) if text.trim().is_empty() => (Vec::new(), None),
            Ok(text) => match serde_json::from_str::<Vec<Task>>(&text) {
                Ok(v) => (v, None),
                Err(e) => (
                    Vec::new(),
                    Some(format!("tasks file was unreadable ({e}); starting empty")),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(e) => (Vec::new(), Some(format!("could not read tasks: {e}"))),
        }
    }

    fn save(&self) -> Result<(), String> {
        let path = self.path()?;
        let json = serde_json::to_string_pretty(&self.items).map_err(|e| e.to_string())?;
        write_private(path, json.as_bytes()).map_err(|e| format!("could not write tasks: {e}"))
    }

    fn index_of(&self, token: &str) -> Result<usize, String> {
        let n: usize = token
            .trim()
            .trim_end_matches('.')
            .parse()
            .map_err(|_| format!("`{token}` is not a list number"))?;
        if n == 0 || n > self.items.len() {
            return Err(format!("no item {n} (list has {})", self.items.len()));
        }
        Ok(n - 1)
    }

    fn add(&mut self, text: &str, habit: bool) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("nothing to add".into());
        }
        if text.chars().count() > MAX_TEXT {
            return Err(format!("text is longer than {MAX_TEXT} characters"));
        }
        if self.items.len() >= MAX_ITEMS {
            return Err(format!("list is full ({MAX_ITEMS} items)"));
        }
        let id = self
            .items
            .iter()
            .map(|t| t.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.items.push(Task {
            id,
            text: text.to_string(),
            habit,
            done: false,
            streak: 0,
            best: 0,
            done_day: None,
        });
        self.save()?;
        Ok(format!(
            "{} `{text}`",
            if habit { "habit added:" } else { "added:" }
        ))
    }

    /// Mark item `n` complete. Habits advance/reset their streak by day.
    fn complete(&mut self, token: &str) -> Result<String, String> {
        let i = self.index_of(token)?;
        let today = self.today;
        let task = &mut self.items[i];
        if task.done && !task.habit {
            return Ok(format!("`{}` is already done", task.text));
        }
        if task.habit && task.done_day == Some(today) {
            return Ok(format!("`{}` is already done today", task.text));
        }
        if task.habit {
            task.streak = if task.done_day == Some(today.saturating_sub(1)) {
                task.streak.saturating_add(1)
            } else {
                1
            };
            task.best = task.best.max(task.streak);
            task.done_day = Some(today);
            task.done = true;
            let (text, streak, best) = (task.text.clone(), task.streak, task.best);
            self.save()?;
            return Ok(format!("`{text}` — streak {streak} (best {best})"));
        }
        task.done = true;
        task.done_day = Some(today);
        let text = task.text.clone();
        self.save()?;
        Ok(format!("done: `{text}`"))
    }

    fn reopen(&mut self, token: &str) -> Result<String, String> {
        let i = self.index_of(token)?;
        let task = &mut self.items[i];
        task.done = false;
        task.done_day = None;
        task.streak = 0;
        let text = task.text.clone();
        self.save()?;
        Ok(format!("reopened `{text}`"))
    }

    fn remove(&mut self, token: &str) -> Result<String, String> {
        let i = self.index_of(token)?;
        let removed = self.items.remove(i);
        self.save()?;
        Ok(format!("removed `{}`", removed.text))
    }

    fn clean(&mut self) -> Result<String, String> {
        let before = self.items.len();
        // Habits are kept: their history is the streak.
        self.items.retain(|t| !(t.done && !t.habit));
        let removed = before - self.items.len();
        self.save()?;
        Ok(format!("removed {removed} completed task(s)"))
    }

    pub fn listing(&self) -> String {
        let mut out = String::from("TASKS & HABITS\n\n");
        if let Some(w) = &self.warning {
            out.push_str(&format!("! {w}\n\n"));
        }
        if self.items.is_empty() {
            out.push_str("Nothing yet.\n\nadd <text>          a task\nhabit <text>        a daily habit\ndone <n>            mark item n done");
            return out;
        }
        for (i, t) in self.items.iter().enumerate() {
            // A habit that was completed on an earlier day is due again.
            let done = if t.habit {
                t.done && t.done_day == Some(self.today)
            } else {
                t.done
            };
            let mark = if done { "[x]" } else { "[ ]" };
            let habit = if t.habit {
                format!("  streak {}", t.streak)
            } else {
                String::new()
            };
            let best = if t.habit {
                format!(" (best {})", t.best)
            } else {
                String::new()
            };
            out.push_str(&format!("{:>3}. {mark} {}{habit}{best}\n", i + 1, t.text));
        }
        let tasks = self.items.iter().filter(|t| !t.habit).count();
        let habits = self.items.len() - tasks;
        out.push_str(&format!("\n{tasks} task(s), {habits} habit(s)"));
        out
    }

    /// Execute one command line.
    pub fn execute(&mut self, line: &str) -> ModuleReply {
        if self.file.is_none() {
            return ModuleReply::err("tasks storage not initialised");
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
            "add" | "a" | "new" => self.add(rest, false),
            "habit" | "h" => self.add(rest, true),
            "done" | "x" | "check" => self.complete(rest),
            "undo" | "reopen" => self.reopen(rest),
            "rm" | "remove" | "del" => self.remove(rest),
            "clean" | "purge" => self.clean(),
            other => Err(format!("unknown command `{other}` (try `help`)")),
        };
        match outcome {
            Ok(text) => ModuleReply::ok(text),
            Err(e) => ModuleReply::err(e),
        }
    }
}

impl Default for Tasks {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "TASKS & HABITS

  add <text>        add a task
  habit <text>      add a daily habit (keeps a streak)
  done <n>          mark list item n done
  undo <n>          reopen item n
  rm <n>            delete item n
  clean             drop completed tasks (habits are kept)
  list              show everything

Stored locally in the module's private directory; nothing is synced or shared.";

fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn today_of_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / SECONDS_PER_DAY)
        .unwrap_or(0)
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

impl Module for Tasks {
    fn id(&self) -> &'static str {
        "app-tasks"
    }
    fn name(&self) -> &'static str {
        "Tasks & Habits"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        let dir = PathBuf::from(&ctx.data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("tasks.json");
        let (items, warning) = Tasks::load(&file);
        self.items = items;
        self.warning = warning;
        self.file = Some(file);
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => {
                if self.file.is_none() {
                    return ModuleReply::err("tasks storage not initialised");
                }
                ModuleReply::ok(self.listing())
            }
            "run" => self.execute(payload),
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Tasks::new);

#[cfg(test)]
mod tests {
    use super::*;
    use mandala_module_api::Host;

    fn ready() -> (tempfile::TempDir, Tasks) {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Tasks::new();
        t.init(&ModuleContext {
            data_dir: dir.path().to_string_lossy().into(),
            host: Host::detached("app-tasks"),
        })
        .unwrap();
        t.set_day(1_000);
        (dir, t)
    }

    fn ok(t: &mut Tasks, cmd: &str) -> String {
        let r = t.execute(cmd);
        assert!(r.ok, "{cmd}: {}", r.text);
        r.text
    }

    #[test]
    fn add_and_list_tasks() {
        let (_d, mut t) = ready();
        assert!(t.handle("launch", "").text.contains("Nothing yet"));
        ok(&mut t, "add buy milk");
        ok(&mut t, "add walk the dog");
        let list = ok(&mut t, "list");
        assert!(
            list.contains("[ ] buy milk") && list.contains("[ ] walk the dog"),
            "{list}"
        );
        assert!(list.contains("2 task(s), 0 habit(s)"));
    }

    #[test]
    fn complete_undo_and_remove_by_number() {
        let (_d, mut t) = ready();
        ok(&mut t, "add alpha");
        ok(&mut t, "add beta");
        ok(&mut t, "done 1");
        assert!(ok(&mut t, "list").contains("[x] alpha"));
        assert!(!t.execute("done 9").ok, "index out of range");
        ok(&mut t, "undo 1");
        assert!(ok(&mut t, "list").contains("[ ] alpha"));
        ok(&mut t, "rm 2");
        assert!(!ok(&mut t, "list").contains("beta"));
    }

    #[test]
    fn habit_streaks_follow_the_calendar() {
        let (_d, mut t) = ready();
        ok(&mut t, "habit meditation");
        ok(&mut t, "done 1");
        assert!(ok(&mut t, "list").contains("streak 1 (best 1)"));
        // Same day: no change.
        assert!(t.execute("done 1").text.contains("already done today"));
        // Next day: streak grows.
        t.set_day(1_001);
        ok(&mut t, "done 1");
        assert!(ok(&mut t, "list").contains("streak 2 (best 2)"));
        // A gap resets it but keeps the best.
        t.set_day(1_010);
        assert!(
            ok(&mut t, "list").contains("[ ] meditation"),
            "habit is due again on a new day"
        );
        ok(&mut t, "done 1");
        let list = ok(&mut t, "list");
        assert!(list.contains("streak 1 (best 2)"), "{list}");
    }

    #[test]
    fn clean_keeps_habits_and_drops_finished_tasks() {
        let (_d, mut t) = ready();
        ok(&mut t, "add done task");
        ok(&mut t, "habit water plants");
        ok(&mut t, "done 1");
        ok(&mut t, "done 2");
        let out = ok(&mut t, "clean");
        assert!(out.contains("removed 1"));
        let list = ok(&mut t, "list");
        assert!(!list.contains("done task"), "{list}");
        assert!(list.contains("water plants"), "{list}");
    }

    #[test]
    fn persists_across_instances() {
        let (dir, mut t) = ready();
        ok(&mut t, "add survive reload");
        let mut second = Tasks::new();
        second
            .init(&ModuleContext {
                data_dir: dir.path().to_string_lossy().into(),
                host: Host::detached("app-tasks"),
            })
            .unwrap();
        assert!(second.handle("launch", "").text.contains("survive reload"));
    }

    #[test]
    fn limits_and_bad_input() {
        let (_d, mut t) = ready();
        assert!(!t.execute("add").ok, "empty text rejected");
        assert!(!t.execute(&format!("add {}", "x".repeat(MAX_TEXT + 1))).ok);
        assert!(!t.execute("done zzz").ok);
        assert!(!t.execute("done 3").ok, "no items yet");
        assert!(!t.execute("wat").ok);

        for i in 0..MAX_ITEMS {
            assert!(t.execute(&format!("add item-{i}")).ok);
        }
        assert!(!t.execute("add overflow").ok, "list full");
        assert!(
            !t.execute(&format!("habit x{}", "y".repeat(MAX_TEXT))).ok,
            "still full"
        );
        assert!(
            t.execute("list 3").ok,
            "extra arguments on list are ignored"
        );
        assert!(t.execute("done 3").ok, "numbered completion still works");
    }

    #[test]
    fn uninitialised_and_unknown_action() {
        let mut t = Tasks::new();
        assert!(!t.handle("launch", "").ok);
        assert!(!t.handle("run", "list").ok);
        assert!(!t.handle("bogus", "").ok);
    }

    #[test]
    fn storage_file_is_private() {
        let (dir, mut t) = ready();
        ok(&mut t, "add secret-ish");
        let file = dir.path().join("tasks.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("secret-ish"));
    }
}
