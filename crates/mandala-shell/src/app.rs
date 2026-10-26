//! Application controller: owns the UDF state and connects
//! Slint callbacks -> reducer -> effects -> IPC broker -> modules -> UI.

use crate::store::Store;
use mandala_core::gesture::{Gesture, GestureConfig, GestureTracker};
use mandala_core::ipc_bus::{IpcEnvelope, MicrokernelIpcBroker, HOST};
use mandala_core::loader::{format_monitor, ModuleHost};
use mandala_core::scheduler::{now_ms, AlarmRequest, Scheduler};
use mandala_core::slots;
use mandala_core::state::{reduce, Action, AppState, Effect};
use mandala_core::theme::ThemeManager;
use mandala_module_api::ModuleReply;
use mandala_ui::MainWindow;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct App {
    state: Mutex<AppState>,
    gestures: Mutex<GestureTracker>,
    themes: Mutex<ThemeManager>,
    /// The module id whose theme is currently on the window (None = home),
    /// so the theme changes exactly when the visible context does.
    active_theme_for: Mutex<Option<String>>,
    broker: Arc<MicrokernelIpcBroker>,
    host: Arc<ModuleHost>,
    store: Store,
    scheduler: Arc<Mutex<Scheduler>>,
    rt: tokio::runtime::Handle,
    ui: Mutex<slint::Weak<MainWindow>>,
    started: Instant,
}

impl App {
    pub fn new(
        broker: Arc<MicrokernelIpcBroker>,
        host: Arc<ModuleHost>,
        themes: ThemeManager,
        scheduler: Arc<Mutex<Scheduler>>,
        rt: tokio::runtime::Handle,
        ui: slint::Weak<MainWindow>,
        installed: Vec<String>,
    ) -> Arc<Self> {
        let state = AppState {
            theme_id: themes.current().id.clone(),
            installed,
            ..AppState::default()
        };
        let store = Store::new(&host.paths().root);
        log::info!("module store base URL: {}", store.base());
        Arc::new(Self {
            state: Mutex::new(state),
            gestures: Mutex::new(GestureTracker::new(GestureConfig::default())),
            themes: Mutex::new(themes),
            active_theme_for: Mutex::new(None),
            broker,
            host,
            store,
            scheduler,
            rt,
            ui: Mutex::new(ui),
            started: Instant::now(),
        })
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Thread-safe entry point: reduce, repaint, run effects in order.
    pub fn dispatch(self: &Arc<Self>, action: Action) {
        let effects = reduce(&mut lock(&self.state), action);
        self.repaint();
        // Theme follows the visible context: an app with its own theme gets
        // it on open; back to home restores the selected theme.
        let active = lock(&self.state).active_module.clone();
        self.sync_theme(active);
        if !effects.is_empty() {
            let this = self.clone();
            // Module calls may block (file I/O, dlopen): keep them off the UI thread.
            self.rt.spawn_blocking(move || {
                for fx in effects {
                    this.run_effect(fx);
                }
            });
        }
    }

    fn repaint(self: &Arc<Self>) {
        let this = self.clone();
        let weak = lock(&self.ui).clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let snapshot = lock(&this.state).clone();
            mandala_ui::render(&ui, &snapshot);
        });
    }

    fn run_effect(self: &Arc<Self>, fx: Effect) {
        match fx {
            Effect::CallModule {
                module,
                action,
                payload,
            } => {
                let env = self.broker.envelope(HOST, &module, &action, &payload);
                if let Err(e) = self.broker.try_dispatch(env) {
                    self.dispatch(Action::HostError {
                        module,
                        text: e.to_string(),
                    });
                }
            }
            Effect::RefreshMonitor => {
                let text = format_monitor(&self.host.status(slots::module_ids()));
                self.dispatch(Action::MonitorUpdated(text));
            }
            Effect::UnloadAll => {
                let n = self.host.unload_all();
                log::info!("unloaded {n} module(s)");
            }
            Effect::RestartAll => {
                let n = self.host.restart_all();
                log::info!("restart: dropped {n} module(s), cleared quarantine verdicts");
            }
            Effect::SetWallpaper(path) => {
                #[cfg(target_os = "android")]
                let message = match crate::android::set_wallpaper(std::path::Path::new(&path)) {
                    Ok(()) => "Wallpaper updated.".to_string(),
                    Err(e) => format!("wallpaper: {e}"),
                };
                #[cfg(not(target_os = "android"))]
                let message = {
                    let _ = path;
                    "wallpaper: Android only — on desktop the OS owns the background.".to_string()
                };
                self.dispatch(Action::Notice(message));
            }
            Effect::RequestOverlay => {
                #[cfg(target_os = "android")]
                let message = match crate::android::request_overlay_permission() {
                    Ok(true) => "Overlay permission already granted.".to_string(),
                    Ok(false) => "Overlay permission requested.".to_string(),
                    Err(e) => format!("overlay: {e}"),
                };
                #[cfg(not(target_os = "android"))]
                let message =
                    "overlay: Android only — desktop window managers do not need it.".to_string();
                self.dispatch(Action::Notice(message));
            }
            Effect::Camera => {
                #[cfg(target_os = "android")]
                let message = {
                    let dir = self.host.paths().data_dir("app-camera").join("captures");
                    let file = dir.join(format!(
                        "IMG_{}.jpg",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    ));
                    let _ = std::fs::create_dir_all(&dir);
                    match crate::android::capture(&file) {
                        Ok(()) => format!(
                            "camera: opened — the photo is written to {}",
                            file.display()
                        ),
                        Err(e) => format!("camera: {e}"),
                    }
                };
                #[cfg(not(target_os = "android"))]
                let message = "camera: Android only — this build has no device camera.\n\nThe Camera slot still lists and inspects files you copy into\n<home>/data/app-camera/captures.".to_string();
                self.dispatch(Action::Notice(message));
            }
            Effect::SetTheme(key) => {
                let picked = lock(&self.themes).select(&key).cloned();
                match picked {
                    Ok(theme) => {
                        let id = theme.id.clone();
                        let weak = lock(&self.ui).clone();
                        let _ = weak
                            .upgrade_in_event_loop(move |ui| mandala_ui::apply_theme(&ui, &theme));
                        self.dispatch(Action::ThemeApplied { id });
                    }
                    Err(e) => self.dispatch(Action::Notice(e.to_string())),
                }
            }
            Effect::FetchCatalog => match self.store.fetch(&self.host) {
                Ok(out) => self.dispatch(Action::StoreCatalog {
                    rows: out.rows,
                    note: out.note,
                }),
                Err(e) => self.dispatch(Action::StoreCatalogError(e)),
            },
            Effect::InstallModule { id } => {
                let (ok, msg) = self.store.install(&self.host, &id);
                // Rows are recomputed from disk so the reducer never has to
                // duplicate install bookkeeping.
                let rows = self.store.rows(&self.host);
                // Panels are app holders: refresh the installed set so a
                // newly installed module appears without a restart.
                let installed = crate::installed_module_ids(self.host.paths());
                self.dispatch(Action::InstalledChanged(installed));
                self.dispatch(Action::StoreInstallDone { ok, msg, rows });
            }
            Effect::ExitPlatform => {
                // The only path that closes Mandala itself (the Exit button):
                // on Android finish the activity (back to the system launcher),
                // on desktop quit the event loop. Safe from any thread.
                #[cfg(target_os = "android")]
                if let Err(e) = crate::android::finish_activity() {
                    log::error!("exit: {e}");
                }
                #[cfg(not(target_os = "android"))]
                let _ = slint::quit_event_loop();
            }
            Effect::ScheduleAlarm { when, text, target } => {
                let Some(due) = Self::due_ms_from(&when) else {
                    self.notice(format!("alarm: bad time `{when}`"));
                    return;
                };
                let req = AlarmRequest {
                    id: format!("u-{}-{due}", now_ms()),
                    due_ms: due,
                    target: target.clone().unwrap_or_else(|| HOST.to_string()),
                    action: "alarm".into(),
                    payload: serde_json::json!({ "ok": true, "text": text }).to_string(),
                };
                let tomorrow = Self::due_is_tomorrow(due);
                let via = target.map(|t| format!(" via {t}")).unwrap_or_default();
                match lock(&self.scheduler).add(req) {
                    Ok(()) => self.notice(format!(
                        "alarm set: {when} {text}{via}{}",
                        if tomorrow { " (tomorrow)" } else { "" }
                    )),
                    Err(e) => self.notice(format!("alarm: {e}")),
                }
            }
            Effect::ListAlarms => {
                self.notice(self.alarm_list_text());
            }
            Effect::CancelAlarm { index } => {
                let id = lock(&self.scheduler)
                    .list()
                    .get(index)
                    .map(|r| r.id.clone());
                let out = match id {
                    Some(id) => lock(&self.scheduler).cancel(&id),
                    None => Ok(false),
                };
                self.notice(match out {
                    Ok(true) => format!("alarm #{index} canceled"),
                    Ok(false) => "alarm: nothing to cancel at that position".to_string(),
                    Err(e) => format!("alarm: {e}"),
                });
            }
        }
    }

    fn notice(self: &Arc<Self>, text: impl Into<String>) {
        self.dispatch(Action::Notice(text.into()));
    }

    /// `HH:MM` -> epoch ms of the next occurrence in the device's local zone
    /// (today if still ahead, else tomorrow). `None` on a malformed time.
    /// shortcut: local zone resolution may fall back to UTC where the OS
    /// timezone cannot be read (Android without TZ env); upgrade if alarms
    /// drift once real users depend on wall-clock exactness.
    fn due_ms_from(when: &str) -> Option<u64> {
        use chrono::TimeZone;
        let (h, m) = when.split_once(':')?;
        let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
        if h > 23 || m > 59 {
            return None;
        }
        let now = chrono::Local::now();
        let next = |day_off: i64| {
            let naive =
                (now.date_naive() + chrono::Duration::days(day_off)).and_hms_opt(h, m, 0)?;
            now.timezone()
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.timestamp_millis() as u64)
        };
        let today = next(0)?;
        if today > now_ms() {
            Some(today)
        } else {
            next(1)
        }
    }

    /// Epoch ms -> `HH:MM` in the local zone (empty string if unresolvable).
    fn alarm_hhmm(due_ms: u64) -> String {
        use chrono::TimeZone;
        match chrono::Local
            .timestamp_opt((due_ms / 1000) as i64, 0)
            .single()
        {
            Some(t) => t.format("%H:%M").to_string(),
            None => String::new(),
        }
    }

    /// True when the due time is not today (the `(tomorrow)` suffix).
    fn due_is_tomorrow(due_ms: u64) -> bool {
        use chrono::TimeZone;
        match chrono::Local
            .timestamp_opt((due_ms / 1000) as i64, 0)
            .single()
        {
            Some(t) => t.date_naive() != chrono::Local::now().date_naive(),
            None => false,
        }
    }

    /// Readable list of pending alarms for the home canvas.
    fn alarm_list_text(&self) -> String {
        let sched = lock(&self.scheduler);
        let list = sched.list();
        if list.is_empty() {
            return "no alarms scheduled".into();
        }
        list.iter()
            .enumerate()
            .map(|(n, r)| {
                let time = Self::alarm_hhmm(r.due_ms);
                let via = if r.target == HOST {
                    String::new()
                } else {
                    format!(" via {}", r.target)
                };
                let text = serde_json::from_str::<serde_json::Value>(&r.payload)
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                    .unwrap_or_default();
                format!("{n}: {time}{via} {text}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Apply the current theme to a freshly created window (UI thread).
    pub fn apply_current_theme(&self, ui: &MainWindow) {
        mandala_ui::apply_theme(ui, lock(&self.themes).current());
    }

    /// Switch the window theme exactly when the visible module context
    /// changes: app theme for that app, the user-selected theme otherwise.
    fn sync_theme(self: &Arc<Self>, active: Option<String>) {
        {
            let mut last = lock(&self.active_theme_for);
            if *last == active {
                return;
            }
            *last = active.clone();
        }
        let theme = {
            let tm = lock(&self.themes);
            active
                .as_deref()
                .and_then(|id| tm.for_app(id).cloned())
                .unwrap_or_else(|| tm.current().clone())
        };
        let weak = lock(&self.ui).clone();
        let id = theme.id.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| mandala_ui::apply_theme(&ui, &theme));
        self.dispatch(Action::ThemeApplied { id });
    }

    // ---- Slint callbacks -------------------------------------------------

    pub fn on_pointer_down(&self, x: f32, y: f32) {
        let now = self.now_ms();
        lock(&self.gestures).pointer_down(x, y, now);
    }

    pub fn on_pointer_up(self: &Arc<Self>, x: f32, y: f32, w: f32, h: f32) {
        let now = self.now_ms();
        let g = lock(&self.gestures).pointer_up(x, y, w, h, now);
        if !matches!(g, Gesture::None | Gesture::Tap) {
            self.dispatch(Action::Gesture(g));
        }
    }

    // ---- Router callback (messages addressed to the host) ----------------

    pub fn on_host_envelope(self: &Arc<Self>, env: IpcEnvelope) {
        let module = env.sender_module.clone();
        // Module-facing scheduler API: any signed module may schedule or
        // cancel alarms to the shell (host), or to a module the ACL already
        // lets it message — cross-app triggers without new wiring
        // (`{"id":..,"when":"HH:MM"|"due_ms":..,"text":..,"target":app-x?}`).
        match env.payload_action.as_str() {
            "schedule" => {
                return self.schedule_from_module(&module, &env.data_json);
            }
            "unschedule" => {
                return self.unschedule_from_module(&module, &env.data_json);
            }
            _ => {}
        }
        let reply = ModuleReply::from_json(&env.data_json);
        let action = match env.payload_action.as_str() {
            "reply" => Action::ModuleReply {
                module,
                ok: reply.ok,
                text: reply.text,
            },
            "error" => Action::HostError {
                module,
                text: reply.text,
            },
            // Unsolicited message from a module (e.g. a notification).
            _ => Action::Notice(format!(
                "[{module}] {}",
                if reply.text.is_empty() {
                    env.data_json
                } else {
                    reply.text
                }
            )),
        };
        self.dispatch(action);
    }

    /// Alarm from a module. Payload (JSON):
    /// `{"id": str, "when": "HH:MM" } or {"id": str, "due_ms": u64}`,
    /// optional `"text"`, optional `"target"` (module id — only allowed if
    /// the ACL already grants module -> target).
    fn schedule_from_module(self: &Arc<Self>, module: &str, data: &str) {
        let req = (|| -> Result<AlarmRequest, String> {
            let j: serde_json::Value =
                serde_json::from_str(data).map_err(|e| format!("bad payload: {e}"))?;
            let id = format!(
                "{module}:{}",
                j.get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("missing field `id`")?
            );
            let due_ms = if let Some(d) = j.get("due_ms").and_then(|v| v.as_u64()) {
                d
            } else {
                let when = j
                    .get("when")
                    .and_then(|v| v.as_str())
                    .ok_or("missing field `when` or `due_ms`")?;
                Self::due_ms_from(when).ok_or("bad `when` (want HH:MM)")?
            };
            let text = j
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(no text)")
                .to_string();
            let target = j
                .get("target")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| HOST.to_string());
            if target != HOST && !self.broker.acl().permits(module, &target) {
                return Err(format!("ACL: {module} may not trigger {target}"));
            }
            Ok(AlarmRequest {
                id,
                due_ms,
                target,
                action: "alarm".into(),
                payload: serde_json::json!({ "ok": true, "text": text }).to_string(),
            })
        })();
        match req {
            Ok(req) => {
                let out = lock(&self.scheduler).add(req);
                let msg = match out {
                    Ok(()) => "alarm scheduled".to_string(),
                    Err(e) => format!("alarm rejected: {e}"),
                };
                self.notice(format!("[{module}] {msg}"));
            }
            Err(e) => self.notice(format!("[{module}] alarm rejected: {e}")),
        }
    }

    /// Cancel a module alarm by its id (namespaced `<module>:<id>`).
    fn unschedule_from_module(self: &Arc<Self>, module: &str, data: &str) {
        let id_raw = serde_json::from_str::<serde_json::Value>(data)
            .ok()
            .and_then(|j| j.get("id").and_then(|v| v.as_str()).map(String::from));
        let Some(id_raw) = id_raw else {
            return self.notice(format!("[{module}] unschedule: missing `id`"));
        };
        let id = format!("{module}:{id_raw}");
        let removed = lock(&self.scheduler).cancel(&id);
        let msg = match removed {
            Ok(true) => "alarm canceled".to_string(),
            Ok(false) => "no such alarm".to_string(),
            Err(e) => format!("unschedule failed: {e}"),
        };
        self.notice(format!("[{module}] {msg}"));
    }
}
