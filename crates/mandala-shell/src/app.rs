//! Application controller: owns the UDF state and connects
//! Slint callbacks -> reducer -> effects -> IPC broker -> modules -> UI.

use crate::store::Store;
use mandala_core::gesture::{Gesture, GestureConfig, GestureTracker};
use mandala_core::ipc_bus::{IpcEnvelope, MicrokernelIpcBroker, HOST};
use mandala_core::loader::{format_monitor, ModuleHost};
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
    broker: Arc<MicrokernelIpcBroker>,
    host: Arc<ModuleHost>,
    store: Store,
    rt: tokio::runtime::Handle,
    ui: Mutex<slint::Weak<MainWindow>>,
    started: Instant,
}

impl App {
    pub fn new(
        broker: Arc<MicrokernelIpcBroker>,
        host: Arc<ModuleHost>,
        themes: ThemeManager,
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
            broker,
            host,
            store,
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
        }
    }

    /// Apply the current theme to a freshly created window (UI thread).
    pub fn apply_current_theme(&self, ui: &MainWindow) {
        mandala_ui::apply_theme(ui, lock(&self.themes).current());
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
}
