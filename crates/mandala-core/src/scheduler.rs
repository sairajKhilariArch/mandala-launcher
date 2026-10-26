//! Host-side scheduler: one-shot alarms that survive restarts.
//!
//! An alarm is a [`AlarmRequest`]: at `due_ms` (UNIX epoch ms) the host
//! dispatches one envelope through the IPC broker. The dispatcher always
//! runs as the trusted host sender, so alarms may target the shell itself
//! (a visible notice) or any installed module (cross-app triggers: a task
//! application schedules "ring me at 18:30" against the alarm module).
//!
//! State lives in `<root>/scheduler.json`; requests made while the host was
//! offline fire once at boot. The polling thread sleeps only until the
//! earliest request, clamped, so an idle shell stays quiet.

use crate::ipc_bus::{MicrokernelIpcBroker, HOST};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AlarmRequest {
    /// Unique label; re-scheduling with the same id replaces the entry.
    pub id: String,
    /// UNIX epoch milliseconds of when the alarm fires.
    pub due_ms: u64,
    /// Broker participant the alarm is dispatched to (`host` = the shell).
    pub target: String,
    /// Action and payload of the outbound envelope.
    pub action: String,
    pub payload: String,
}

impl AlarmRequest {
    /// A shell-visible alarm: fires as an unsolicited host notice.
    pub fn spotlight(id: impl Into<String>, due_ms: u64, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            due_ms,
            target: HOST.to_string(),
            action: "alarm".into(),
            // Reply-shaped so the shell's unsolicited-message branch shows the
            // text verbatim (`{"ok":true,"text":…}`).
            payload: serde_json::json!({ "ok": true, "text": text.into() }).to_string(),
        }
    }
}

/// All pending alarms, kept sorted by due time.
pub struct Scheduler {
    request: Vec<AlarmRequest>,
    path: PathBuf,
}

impl Scheduler {
    /// Load the persisted file; a missing or broken file starts empty (a
    /// user-writable home can lose the file — that is not fatal).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let request = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<AlarmRequest>>(&s).ok())
            .unwrap_or_default();
        let mut s = Self { request, path };
        s.request.sort_by_key(|r| r.due_ms);
        s
    }

    /// Pending alarms, earliest first.
    pub fn list(&self) -> &[AlarmRequest] {
        &self.request
    }

    /// Add or replace (by id). Returning when the file write fails lets the
    /// caller report a persistence problem.
    pub fn add(&mut self, req: AlarmRequest) -> Result<()> {
        self.request.retain(|r| r.id != req.id);
        self.request.push(req);
        self.request.sort_by_key(|r| r.due_ms);
        self.persist()
    }

    /// Remove by id; `true` if it existed.
    pub fn cancel(&mut self, id: &str) -> Result<bool> {
        let n = self.request.len();
        self.request.retain(|r| r.id != id);
        let removed = self.request.len() != n;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Pop every request already due at `now_ms` (and persist the removal).
    pub fn pop_due(&mut self, now_ms: u64) -> Vec<AlarmRequest> {
        let due: Vec<_> = self
            .request
            .iter()
            .filter(|r| r.due_ms <= now_ms)
            .cloned()
            .collect();
        self.request.retain(|r| r.due_ms > now_ms);
        if !due.is_empty() {
            let _ = self.persist();
        }
        due
    }

    fn persist(&self) -> Result<()> {
        let p = self.path.clone();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| crate::MandalaError::io(&p, e))?;
        }
        let buf = serde_json::to_string(&self.request)
            .map_err(|e| crate::MandalaError::Schedule(e.to_string()))?;
        std::fs::write(&p, buf).map_err(|e| crate::MandalaError::io(&p, e))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Background dispatcher. Sleeps until the earliest pending request, never
/// longer than 15 s, fires everything due and repeats. Each wake dispatches
/// through the broker as the host (always permitted by the ACL).
pub fn spawn_runner(sched: Arc<Mutex<Scheduler>>, broker: Arc<MicrokernelIpcBroker>) {
    std::thread::Builder::new()
        .name("mandala-scheduler".into())
        .spawn(move || loop {
            let (now, jobs, next_due) = {
                let mut s = sched.lock().unwrap_or_else(|e| e.into_inner());
                let now = now_ms();
                let jobs = s.pop_due(now);
                let next = s.list().first().map(|r| r.due_ms);
                (now, jobs, next)
            };
            for job in jobs {
                let env = broker.envelope(HOST, &job.target, &job.action, &job.payload);
                if let Err(e) = broker.try_dispatch(env) {
                    log::error!("alarm {} undelivered: {e}", job.id);
                } else {
                    log::info!("alarm {} fired to {}", job.id, job.target);
                }
            }
            let sleep = match next_due {
                Some(d) => (d.saturating_sub(now)).clamp(200, 15_000),
                None => 15_000,
            };
            std::thread::sleep(Duration::from_millis(sleep));
        })
        .expect("spawning the scheduler thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sched(dir: &Path) -> Scheduler {
        Scheduler::load(dir.join("scheduler.json"))
    }

    #[test]
    fn add_replace_cancel_and_list_are_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = sched(dir.path());
        s.add(AlarmRequest::spotlight("b", 2_000, "b")).unwrap();
        s.add(AlarmRequest::spotlight("a", 1_000, "a")).unwrap();
        // Replace by id.
        s.add(AlarmRequest::spotlight("b", 3_000, "b2")).unwrap();
        let ids: Vec<&str> = s.list().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "list is sorted by due time");
        assert_eq!(s.list()[1].payload, r#"{"ok":true,"text":"b2"}"#);
        assert!(s.cancel("a").unwrap());
        assert!(!s.cancel("a").unwrap());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn persistence_survives_reload_and_offline_due_fires_once() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = sched(dir.path());
            s.add(AlarmRequest::spotlight("late", 1_000, "dinner"))
                .unwrap();
            s.add(AlarmRequest::spotlight("future", 2_000_000, "later"))
                .unwrap();
        }
        let mut s = sched(dir.path());
        assert_eq!(s.list().len(), 2, "state survives a restart");
        let fired = s.pop_due(1_500);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, "late");
        assert_eq!(s.list().len(), 1);
        // Popping again after `now` is empty: a due alarm never fires twice.
        assert!(s.pop_due(1_500).is_empty());
    }

    #[test]
    fn broken_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("scheduler.json"), "{ nope").unwrap();
        assert!(sched(dir.path()).list().is_empty());
    }

    #[tokio::test]
    async fn runner_fires_due_alarm_into_the_broker() {
        let dir = tempfile::tempdir().unwrap();
        let (broker, mut rx) =
            crate::ipc_bus::MicrokernelIpcBroker::new(8, crate::ipc_bus::AccessControlList::new());
        let broker = Arc::new(broker);
        let mut s = Scheduler::load(dir.path().join("scheduler.json"));
        s.add(AlarmRequest::spotlight("now", now_ms() - 1, "ring"))
            .unwrap();
        spawn_runner(Arc::new(Mutex::new(s)), broker);
        let got = tokio::time::timeout(Duration::from_secs(40), rx.recv())
            .await
            .expect("alarm must fire shortly after boot")
            .unwrap();
        assert_eq!(got.target_module, HOST);
        assert_eq!(got.payload_action, "alarm");
        assert!(got.data_json.contains("ring"), "{}", got.data_json);
    }
}
