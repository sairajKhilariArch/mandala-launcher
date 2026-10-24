//! Application controller: owns the UDF state and connects
//! Slint callbacks -> reducer -> effects -> IPC broker -> modules -> UI.

use mandala_core::gesture::{Gesture, GestureConfig, GestureTracker};
use mandala_core::ipc_bus::{IpcEnvelope, MicrokernelIpcBroker, HOST};
use mandala_core::loader::{format_monitor, ModuleHost};
use mandala_core::slots;
use mandala_core::state::{reduce, Action, AppState, Effect};
use mandala_core::theme::ThemeManager;
use mandala_module_api::ModuleReply;
use mandala_ui::MainWindow;
use slint::ComponentHandle;
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
    ) -> Arc<Self> {
        let mut state = AppState::default();
        state.theme_id = themes.current().id.clone();
        Arc::new(Self {
            state: Mutex::new(state),
            gestures: Mutex::new(GestureTracker::new(GestureConfig::default())),
            themes: Mutex::new(themes),
            broker,
            host,
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
            Effect::CallModule { module, action, payload } => {
                let env = self.broker.envelope(HOST, &module, &action, &payload);
                if let Err(e) = self.broker.try_dispatch(env) {
                    self.dispatch(Action::HostError { module, text: e.to_string() });
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
            Effect::SetTheme(key) => {
                let picked = lock(&self.themes).select(&key).map(|t| t.clone());
                match picked {
                    Ok(theme) => {
                        let id = theme.id.clone();
                        let weak = lock(&self.ui).clone();
                        let _ = weak.upgrade_in_event_loop(move |ui| mandala_ui::apply_theme(&ui, &theme));
                        self.dispatch(Action::ThemeApplied { id });
                    }
                    Err(e) => self.dispatch(Action::Notice(e.to_string())),
                }
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
            "reply" => Action::ModuleReply { module, ok: reply.ok, text: reply.text },
            "error" => Action::HostError { module, text: reply.text },
            // Unsolicited message from a module (e.g. a notification).
            _ => Action::Notice(format!("[{module}] {}", if reply.text.is_empty() { env.data_json } else { reply.text })),
        };
        self.dispatch(action);
    }
}
