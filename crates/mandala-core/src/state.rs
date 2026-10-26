//! Unidirectional-data-flow state machine.
//!
//! `reduce(state, action) -> effects` is a pure function: the UI renders
//! `AppState`, user input becomes `Action`s, and side effects are returned as
//! data (`Effect`) for the shell to execute. Nothing here touches I/O.

use crate::command::{fuzzy_score, parse_command, search_slots, Command};
use crate::gesture::Gesture;
use crate::slots::{self, SlotKind, SLOTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutClass {
    /// < 600 px: phone, single drawer at a time.
    Compact,
    /// 600..840 px: foldable / small tablet.
    Medium,
    /// >= 840 px: tri-fold / large format, both drawers docked.
    Expanded,
}

impl LayoutClass {
    pub fn from_width(w: f32) -> Self {
        if w < 600.0 {
            Self::Compact
        } else if w < 840.0 {
            Self::Medium
        } else {
            Self::Expanded
        }
    }
}

/// One row of the Store overlay. Status is computed by the shell (it has to
/// hash the installed file against the catalog) and lives here purely for
/// rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRow {
    pub id: String,
    pub label: String,
    pub description: String,
    pub version: String,
    pub status: StoreStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreStatus {
    NotInstalled,
    Installed,
    /// On disk but differing from the catalog: an update is available.
    Update,
    Installing,
}

impl StoreStatus {
    /// Text shown on the right-hand button of a Store row.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotInstalled => "INSTALL",
            Self::Installed => "INSTALLED ✓",
            Self::Update => "UPDATE",
            Self::Installing => "INSTALLING…",
        }
    }

    /// Whether tapping the row starts an install (exactly one in flight).
    pub fn actionable(self) -> bool {
        matches!(self, Self::NotInstalled | Self::Update)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub layout: LayoutClass,
    pub left_open: bool,
    pub right_open: bool,
    pub monitor_open: bool,
    pub active_module: Option<String>,
    pub canvas_text: String,
    pub query: String,
    pub monitor_text: String,
    pub store_open: bool,
    pub store_rows: Vec<StoreRow>,
    /// Status line of the Store overlay: loading, offline note, or the last
    /// install result (failures are prefixed with `! `).
    pub store_status: String,
    pub theme_id: String,
    /// Module ids currently on disk (seeded at boot, refreshed after store
    /// installs). The panels compose from this — they are app holders, not a
    /// permanent app list.
    pub installed: Vec<String>,
    /// The All Apps overlay (the A3 builtin slot) is open.
    pub apps_open: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            layout: LayoutClass::Compact,
            left_open: false,
            right_open: false,
            monitor_open: false,
            active_module: None,
            canvas_text: HOME_HINT.to_string(),
            query: String::new(),
            monitor_text: String::new(),
            store_open: false,
            store_rows: Vec::new(),
            store_status: String::new(),
            theme_id: String::new(),
            installed: Vec::new(),
            apps_open: false,
        }
    }
}

pub const HOME_HINT: &str = "Type to search apps.\n= 2+2   calculate\nnote ...   quick note\n> ls   terminal\nask ...   assistant\nweather Berlin   forecast\nrss headlines   feeds\ntasks   habits\nchat   Matrix\ncamera   capture\n2fa   authenticator\nvault   passwords\nlsend   local transfer\ntheme next   switch theme\nmonitor   module processes\nstore   browse & install modules";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Resize(f32),
    Gesture(Gesture),
    OpenSlot(String),
    QueryChanged(String),
    QuerySubmitted,
    CloseOverlays,
    /// The set of installed module ids changed (boot scan / store install).
    InstalledChanged(Vec<String>),
    /// A module answered a request.
    ModuleReply {
        module: String,
        ok: bool,
        text: String,
    },
    /// A host-side failure (not installed, quarantined, ...).
    HostError {
        module: String,
        text: String,
    },
    MonitorUpdated(String),
    /// Result of a theme switch attempt.
    ThemeApplied {
        id: String,
    },
    Notice(String),
    /// User pressed a button on the process monitor.
    MonitorUnloadAll,
    MonitorRestartAll,
    /// Back key (Android system back, or desktop Escape): closest overlay,
    /// else the open app, else a no-op. The shell itself never exits here.
    Back,
    /// The explicit exit button (bottom-left of home): the only path that
    /// closes Mandala itself.
    ExitShell,
    /// The Store overlay was opened (`store` command or Refresh).
    StoreOpen,
    /// A freshly fetched (or cached) catalog arrived; rows carry the
    /// per-module install status computed by the shell.
    StoreCatalog {
        rows: Vec<StoreRow>,
        /// Extra status line: the offline-cache note, `None` when live.
        note: Option<String>,
    },
    StoreCatalogError(String),
    /// User tapped Install/Update on a Store row.
    StoreInstall(String),
    /// The install effect finished; rows are recomputed from disk so this
    /// state never has to duplicate install logic.
    StoreInstallDone {
        ok: bool,
        msg: String,
        rows: Vec<StoreRow>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    CallModule {
        module: String,
        action: String,
        payload: String,
    },
    RefreshMonitor,
    UnloadAll,
    RestartAll,
    SetTheme(String),
    /// Android-only: set the system wallpaper from a local image.
    SetWallpaper(String),
    /// Android-only: ask for the draw-over-other-apps permission.
    RequestOverlay,
    /// Android-only: fire the camera intent into the C5 captures directory.
    Camera,
    /// Download + validate the store catalog (blocking; the shell runs it
    /// off the UI thread and reports back with `StoreCatalog*`).
    FetchCatalog,
    /// Download, verify and install one catalog entry (blocking).
    InstallModule {
        id: String,
    },
    /// Close the whole launcher: finish the Android activity (back to the
    /// system launcher) or quit the desktop event loop.
    ExitPlatform,
}

pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    let mut fx = Vec::new();
    match action {
        Action::Resize(w) => {
            state.layout = LayoutClass::from_width(w);
            if state.layout == LayoutClass::Expanded {
                // Both drawers are permanently docked; "open" flags are moot.
                state.left_open = false;
                state.right_open = false;
            }
        }

        Action::Gesture(g) => match g {
            Gesture::OpenPanelB => {
                state.left_open = true;
                state.right_open = false;
            }
            Gesture::OpenPanelC => {
                state.right_open = true;
                state.left_open = false;
            }
            Gesture::SwipeLeft => state.left_open = false,
            Gesture::SwipeRight => state.right_open = false,
            Gesture::SwipeUp => {
                // Minimize active sub-module, clear focus to workspace.
                state.active_module = None;
                state.left_open = false;
                state.right_open = false;
                state.monitor_open = false;
                state.apps_open = false;
                state.canvas_text = HOME_HINT.to_string();
            }
            Gesture::DoubleSwipeUp | Gesture::HoldBottom => {
                state.monitor_open = true;
                fx.push(Effect::RefreshMonitor);
            }
            Gesture::SystemBackPassthrough | Gesture::Tap | Gesture::None => {}
        },

        Action::CloseOverlays => {
            state.left_open = false;
            state.right_open = false;
            state.monitor_open = false;
            state.store_open = false;
            state.apps_open = false;
        }

        Action::Back => {
            let any_overlay = state.left_open
                || state.right_open
                || state.monitor_open
                || state.store_open
                || state.apps_open;
            if any_overlay {
                return reduce(state, Action::CloseOverlays);
            }
            if state.active_module.is_some() {
                // Same as the home SwipeUp gesture: back closes the app, the
                // shell stays alive.
                state.active_module = None;
                state.canvas_text = HOME_HINT.to_string();
            }
            // At home: no-op. The shell is only closed by the Exit button.
        }

        Action::ExitShell => {
            fx.push(Effect::ExitPlatform);
        }

        Action::InstalledChanged(ids) => {
            state.installed = ids;
        }

        Action::OpenSlot(id) => {
            state.left_open = false;
            state.right_open = false;
            // One surface at a time: tapping an app row in the All Apps
            // overlay closes it (the builtin branch re-opens it below).
            state.apps_open = false;
            match slots::by_id(&id) {
                None => {
                    // Modules outside the static matrix (apps added later)
                    // still launch when installed and tapped from All Apps.
                    if state.installed.iter().any(|i| i == &id) {
                        let label = slots::label_for(&id);
                        state.active_module = Some(id.clone());
                        state.canvas_text = format!("Opening {label}...");
                        fx.push(Effect::CallModule {
                            module: id.clone(),
                            action: "launch".into(),
                            payload: String::new(),
                        });
                    } else {
                        state.canvas_text = format!("Unknown slot `{id}`");
                    }
                }
                Some(slot) if slot.kind == SlotKind::BuiltinAllApps => {
                    state.active_module = None;
                    state.apps_open = true;
                }
                Some(slot) => {
                    state.active_module = Some(slot.id.to_string());
                    state.canvas_text = format!("Opening {}...", slot.label);
                    fx.push(Effect::CallModule {
                        module: slot.id.into(),
                        action: "launch".into(),
                        payload: String::new(),
                    });
                }
            }
        }

        Action::QueryChanged(q) => {
            state.query = q;
            if state.active_module.is_none() {
                state.canvas_text = if state.query.trim().is_empty() {
                    HOME_HINT.to_string()
                } else {
                    describe_all_apps(&state.query)
                };
            }
        }

        Action::QuerySubmitted => {
            let cmd = parse_command(&state.query);
            state.query.clear();
            match cmd {
                Command::Empty => {}
                Command::Calc(expr) => {
                    state.active_module = Some("app-calculator".into());
                    fx.push(Effect::CallModule {
                        module: "app-calculator".into(),
                        action: "eval".into(),
                        payload: expr,
                    });
                }
                Command::CalcSave(expr) => {
                    state.active_module = Some("app-calculator".into());
                    fx.push(Effect::CallModule {
                        module: "app-calculator".into(),
                        action: "save".into(),
                        payload: expr,
                    });
                }
                Command::Note(text) => {
                    state.active_module = Some("app-notes".into());
                    fx.push(Effect::CallModule {
                        module: "app-notes".into(),
                        action: "append".into(),
                        payload: text,
                    });
                }
                Command::Terminal(cmd) => {
                    state.active_module = Some("app-terminal".into());
                    state.canvas_text = format!("> {cmd}");
                    fx.push(Effect::CallModule {
                        module: "app-terminal".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Aegis(cmd) => {
                    state.active_module = Some("app-aegis".into());
                    fx.push(Effect::CallModule {
                        module: "app-aegis".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Vault(cmd) => {
                    state.active_module = Some("app-vault".into());
                    fx.push(Effect::CallModule {
                        module: "app-vault".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::LocalSend(cmd) => {
                    state.active_module = Some("app-localsend".into());
                    fx.push(Effect::CallModule {
                        module: "app-localsend".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Wallpaper(path) => {
                    state.active_module = None;
                    fx.push(Effect::SetWallpaper(path));
                }
                Command::Overlay => fx.push(Effect::RequestOverlay),
                Command::Weather(cmd) => {
                    state.active_module = Some("app-weather".into());
                    fx.push(Effect::CallModule {
                        module: "app-weather".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Rss(cmd) => {
                    state.active_module = Some("app-rss".into());
                    fx.push(Effect::CallModule {
                        module: "app-rss".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Tasks(cmd) => {
                    state.active_module = Some("app-tasks".into());
                    fx.push(Effect::CallModule {
                        module: "app-tasks".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Reader(cmd) => {
                    state.active_module = Some("app-reader".into());
                    fx.push(Effect::CallModule {
                        module: "app-reader".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Gallery(cmd) => {
                    state.active_module = Some("app-gallery".into());
                    fx.push(Effect::CallModule {
                        module: "app-gallery".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Chat(cmd) => {
                    state.active_module = Some("app-chat".into());
                    fx.push(Effect::CallModule {
                        module: "app-chat".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Ai(cmd) => {
                    state.active_module = Some("app-ai".into());
                    fx.push(Effect::CallModule {
                        module: "app-ai".into(),
                        action: "run".into(),
                        payload: cmd,
                    });
                }
                Command::Camera => {
                    state.active_module = None;
                    state.canvas_text = "Opening the camera...".into();
                    fx.push(Effect::Camera);
                }
                Command::Theme(key) => fx.push(Effect::SetTheme(key)),
                Command::Monitor => {
                    state.store_open = false;
                    state.monitor_open = true;
                    fx.push(Effect::RefreshMonitor);
                }
                Command::Store => return reduce(state, Action::StoreOpen),
                Command::Launch(id) => return reduce(state, Action::OpenSlot(id.to_string())),
                Command::NoMatch(q) => state.canvas_text = format!("No app matches `{q}`."),
            }
        }

        Action::ModuleReply { module, ok, text } => {
            // Ignore stale replies from a module the user already left.
            if state.active_module.as_deref() == Some(module.as_str())
                || state.active_module.is_none()
            {
                state.canvas_text = if ok { text } else { format!("! {text}") };
            }
        }

        Action::HostError { module, text } => {
            if state.active_module.as_deref() == Some(module.as_str()) {
                state.active_module = None;
            }
            state.canvas_text = format!("! {text}");
        }

        Action::MonitorUpdated(text) => state.monitor_text = text,

        Action::ThemeApplied { id } => {
            state.canvas_text = format!("Theme: {id}");
            state.theme_id = id;
        }

        Action::Notice(text) => state.canvas_text = text,

        Action::MonitorUnloadAll => {
            state.active_module = None;
            fx.push(Effect::UnloadAll);
            fx.push(Effect::RefreshMonitor);
        }
        Action::MonitorRestartAll => {
            state.active_module = None;
            fx.push(Effect::RestartAll);
            fx.push(Effect::RefreshMonitor);
        }

        Action::StoreOpen => {
            // One full-screen overlay at a time: the monitor closes, the
            // store opens, and the fetch runs off-thread via the effect.
            state.monitor_open = false;
            state.apps_open = false;
            state.store_open = true;
            state.store_status = "loading catalog…".into();
            fx.push(Effect::FetchCatalog);
        }

        Action::StoreCatalog { rows, note } => {
            let installed = rows
                .iter()
                .filter(|r| r.status == StoreStatus::Installed)
                .count();
            // A blank header after a successful refresh looks like a failure;
            // say what the catalog contains instead.
            state.store_status =
                note.unwrap_or_else(|| format!("{} available · {installed} installed", rows.len()));
            state.store_rows = rows;
        }

        Action::StoreCatalogError(msg) => {
            state.store_status = format!("! {msg}");
        }

        Action::StoreInstall(id) => {
            if let Some(row) = state.store_rows.iter_mut().find(|r| r.id == id) {
                // Guard against a double-tap starting a second download of
                // the same module while one is already running.
                if row.status.actionable() {
                    row.status = StoreStatus::Installing;
                    state.store_status = format!("installing {id}…");
                    fx.push(Effect::InstallModule { id });
                }
            }
        }

        Action::StoreInstallDone { ok, msg, rows } => {
            state.store_rows = rows;
            state.store_status = if ok { msg } else { format!("! {msg}") };
        }
    }
    fx
}

/// Text listing for the All-Apps drawer / live search results. The Store is
/// a host feature, not a slot, so it is spliced in alongside the matrix.
pub fn describe_all_apps(query: &str) -> String {
    let q = query.trim();
    // Labels only — `A2  `-style tags were explicitly rejected as
    // user-facing text (they stay internal for fuzzy search).
    let mut rows: Vec<String> = if q.is_empty() {
        SLOTS.iter().map(|s| s.label.to_string()).collect()
    } else {
        search_slots(q, 8)
            .iter()
            .map(|s| s.label.to_string())
            .collect()
    };
    if q.is_empty() || fuzzy_score(q, "Store").is_some() {
        rows.insert(0, "Store".into());
    }
    if rows.is_empty() {
        format!("No app matches `{q}`.")
    } else {
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> AppState {
        AppState::default()
    }

    #[test]
    fn layout_breakpoints() {
        assert_eq!(LayoutClass::from_width(412.0), LayoutClass::Compact);
        assert_eq!(LayoutClass::from_width(599.9), LayoutClass::Compact);
        assert_eq!(LayoutClass::from_width(600.0), LayoutClass::Medium);
        assert_eq!(LayoutClass::from_width(839.9), LayoutClass::Medium);
        assert_eq!(LayoutClass::from_width(840.0), LayoutClass::Expanded);
    }

    #[test]
    fn edge_gestures_open_exclusive_panels() {
        let mut s = st();
        reduce(&mut s, Action::Gesture(Gesture::OpenPanelB));
        assert!(s.left_open && !s.right_open);
        reduce(&mut s, Action::Gesture(Gesture::OpenPanelC));
        assert!(s.right_open && !s.left_open);
        reduce(&mut s, Action::Gesture(Gesture::SwipeRight));
        assert!(!s.right_open);
    }

    #[test]
    fn swipe_up_minimizes_and_clears() {
        let mut s = st();
        s.active_module = Some("app-notes".into());
        s.left_open = true;
        reduce(&mut s, Action::Gesture(Gesture::SwipeUp));
        assert_eq!(s.active_module, None);
        assert!(!s.left_open);
        assert_eq!(s.canvas_text, HOME_HINT);
    }

    #[test]
    fn double_swipe_and_hold_open_monitor() {
        for g in [Gesture::DoubleSwipeUp, Gesture::HoldBottom] {
            let mut s = st();
            let fx = reduce(&mut s, Action::Gesture(g));
            assert!(s.monitor_open);
            assert_eq!(fx, vec![Effect::RefreshMonitor]);
        }
    }

    #[test]
    fn passthrough_gesture_changes_nothing() {
        let mut s = st();
        let before = s.clone();
        assert!(reduce(&mut s, Action::Gesture(Gesture::SystemBackPassthrough)).is_empty());
        assert_eq!(s, before);
    }

    #[test]
    fn back_closes_overlays_then_app_then_noops() {
        // Overlay open: back falls one level to home.
        let mut s = st();
        s.store_open = true;
        reduce(&mut s, Action::Back);
        assert!(!s.store_open);
        // App open: back closes the app, the shell survives.
        s.active_module = Some("app-notes".into());
        s.canvas_text = "notes".into();
        reduce(&mut s, Action::Back);
        assert_eq!(s.active_module, None);
        assert_eq!(s.canvas_text, HOME_HINT);
        // At home: back is a no-op — the shell never exits from back.
        let before = s.clone();
        reduce(&mut s, Action::Back);
        assert_eq!(s, before);
    }

    #[test]
    fn exit_shell_is_the_only_exit_path() {
        let mut s = st();
        s.left_open = true;
        assert_eq!(
            reduce(&mut s, Action::ExitShell),
            vec![Effect::ExitPlatform]
        );
    }

    #[test]
    fn opening_a_module_slot_emits_launch() {
        let mut s = st();
        s.left_open = true;
        let fx = reduce(&mut s, Action::OpenSlot("app-notes".into()));
        assert!(!s.left_open);
        assert_eq!(s.active_module.as_deref(), Some("app-notes"));
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-notes".into(),
                action: "launch".into(),
                payload: "".into()
            }]
        );
    }

    #[test]
    fn all_apps_is_builtin_and_calls_no_module() {
        let mut s = st();
        let fx = reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(fx.is_empty());
        // The builtin opens the tappable All Apps overlay, not canvas text.
        assert!(s.apps_open);
        assert_eq!(s.active_module, None);
    }

    #[test]
    fn typing_filters_apps_live() {
        let mut s = st();
        reduce(&mut s, Action::QueryChanged("calc".into()));
        assert!(s.canvas_text.starts_with("Calculator"));
        reduce(&mut s, Action::QueryChanged("".into()));
        assert_eq!(s.canvas_text, HOME_HINT);
    }

    #[test]
    fn apps_overlay_clears_on_close_and_home() {
        let mut s = st();
        reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(s.apps_open);
        reduce(&mut s, Action::CloseOverlays);
        assert!(!s.apps_open);

        reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(s.apps_open);
        reduce(&mut s, Action::Gesture(Gesture::SwipeUp));
        assert!(!s.apps_open);
    }

    #[test]
    fn overflow_module_launches_when_installed() {
        let mut s = st();
        reduce(&mut s, Action::InstalledChanged(vec!["app-zzz".into()]));
        let fx = reduce(&mut s, Action::OpenSlot("app-zzz".into()));
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-zzz".into(),
                action: "launch".into(),
                payload: String::new()
            }]
        );
        assert_eq!(s.active_module.as_deref(), Some("app-zzz"));
        assert_eq!(s.canvas_text, "Opening Zzz...");
        // Unknown *and* uninstalled still reports the miss.
        let mut s = st();
        reduce(&mut s, Action::OpenSlot("app-nope".into()));
        assert_eq!(s.canvas_text, "Unknown slot `app-nope`");
    }

    #[test]
    fn tapping_app_row_closes_overlay_but_builtin_reopens_it() {
        let mut s = st();
        reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(s.apps_open);
        reduce(&mut s, Action::OpenSlot("app-notes".into()));
        assert!(!s.apps_open, "launching must close the All Apps overlay");
        reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(s.apps_open);
        reduce(&mut s, Action::StoreOpen);
        assert!(!s.apps_open, "store must replace the All Apps overlay");
    }

    #[test]
    fn installed_changed_replaces_the_list() {
        let mut s = st();
        assert!(s.installed.is_empty());
        reduce(
            &mut s,
            Action::InstalledChanged(vec!["app-notes".into(), "app-zzz".into()]),
        );
        assert_eq!(s.installed, vec!["app-notes", "app-zzz"]);
        reduce(&mut s, Action::InstalledChanged(vec![]));
        assert!(s.installed.is_empty());
    }

    #[test]
    fn module_replies_render_even_with_panels_composed() {
        // Regression guard for "app opens": an installed module's launch
        // reply must land on the canvas, and stale replies are still dropped.
        let mut s = st();
        reduce(&mut s, Action::InstalledChanged(vec!["app-notes".into()]));
        reduce(&mut s, Action::OpenSlot("app-notes".into()));
        reduce(
            &mut s,
            Action::ModuleReply {
                module: "app-notes".into(),
                ok: true,
                text: "opened 0 notes".into(),
            },
        );
        assert_eq!(s.canvas_text, "opened 0 notes");
        reduce(
            &mut s,
            Action::ModuleReply {
                module: "app-rss".into(),
                ok: true,
                text: "stale".into(),
            },
        );
        assert_eq!(
            s.canvas_text, "opened 0 notes",
            "stale reply must be dropped"
        );
    }

    #[test]
    fn submit_routes_commands() {
        let mut s = st();
        s.query = "= 1+1".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(s.query, "");
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-calculator".into(),
                action: "eval".into(),
                payload: "1+1".into()
            }]
        );

        s.query = "note hello".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-notes".into(),
                action: "append".into(),
                payload: "hello".into()
            }]
        );

        s.query = "theme next".into();
        assert_eq!(
            reduce(&mut s, Action::QuerySubmitted),
            vec![Effect::SetTheme("next".into())]
        );

        s.query = "monitor".into();
        assert_eq!(
            reduce(&mut s, Action::QuerySubmitted),
            vec![Effect::RefreshMonitor]
        );
        assert!(s.monitor_open);

        s.query = "> ls".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-terminal".into(),
                action: "run".into(),
                payload: "ls".into()
            }]
        );
        assert_eq!(s.active_module.as_deref(), Some("app-terminal"));

        s.query = "2fa code github".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-aegis".into(),
                action: "run".into(),
                payload: "code github".into()
            }]
        );

        s.query = "vault list".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-vault".into(),
                action: "run".into(),
                payload: "list".into()
            }]
        );

        s.query = "lsend recv 8080 tok!".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-localsend".into(),
                action: "run".into(),
                payload: "recv 8080 tok!".into()
            }]
        );

        s.query = "wallpaper /sdcard/a.png".into();
        assert_eq!(
            reduce(&mut s, Action::QuerySubmitted),
            vec![Effect::SetWallpaper("/sdcard/a.png".into())]
        );

        s.query = "overlay".into();
        assert_eq!(
            reduce(&mut s, Action::QuerySubmitted),
            vec![Effect::RequestOverlay]
        );

        s.query = "weather Berlin".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-weather".into(),
                action: "run".into(),
                payload: "Berlin".into()
            }]
        );
        assert_eq!(s.active_module.as_deref(), Some("app-weather"));

        s.query = "rss headlines".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-rss".into(),
                action: "run".into(),
                payload: "headlines".into()
            }]
        );

        s.query = "tasks add call mom".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-tasks".into(),
                action: "run".into(),
                payload: "add call mom".into()
            }]
        );

        s.query = "doc open 2".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-reader".into(),
                action: "run".into(),
                payload: "open 2".into()
            }]
        );

        s.query = "gallery list".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-gallery".into(),
                action: "run".into(),
                payload: "list".into()
            }]
        );

        s.query = "chat rooms".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-chat".into(),
                action: "run".into(),
                payload: "rooms".into()
            }]
        );

        s.query = "ask explain tls".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(
            fx,
            vec![Effect::CallModule {
                module: "app-ai".into(),
                action: "run".into(),
                payload: "ask explain tls".into()
            }]
        );

        s.query = "camera".into();
        assert_eq!(reduce(&mut s, Action::QuerySubmitted), vec![Effect::Camera]);
        assert!(s.canvas_text.contains("camera"));

        s.query = "termin".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx.len(), 1);
        assert_eq!(s.active_module.as_deref(), Some("app-terminal"));

        s.query = "qqqqzz".into();
        assert!(reduce(&mut s, Action::QuerySubmitted).is_empty());
        assert!(s.canvas_text.contains("No app matches"));
    }

    #[test]
    fn stale_reply_from_other_module_is_dropped() {
        let mut s = st();
        s.active_module = Some("app-notes".into());
        s.canvas_text = "notes".into();
        reduce(
            &mut s,
            Action::ModuleReply {
                module: "app-calculator".into(),
                ok: true,
                text: "4".into(),
            },
        );
        assert_eq!(s.canvas_text, "notes");
        reduce(
            &mut s,
            Action::ModuleReply {
                module: "app-notes".into(),
                ok: false,
                text: "boom".into(),
            },
        );
        assert_eq!(s.canvas_text, "! boom");
    }

    #[test]
    fn host_error_clears_active_module() {
        let mut s = st();
        s.active_module = Some("app-vault".into());
        reduce(
            &mut s,
            Action::HostError {
                module: "app-vault".into(),
                text: "quarantined".into(),
            },
        );
        assert_eq!(s.active_module, None);
        assert!(s.canvas_text.starts_with("! "));
    }

    #[test]
    fn expanded_layout_resets_drawer_flags() {
        let mut s = st();
        s.left_open = true;
        reduce(&mut s, Action::Resize(1200.0));
        assert_eq!(s.layout, LayoutClass::Expanded);
        assert!(!s.left_open);
    }

    #[test]
    fn monitor_buttons_emit_effects() {
        let mut s = st();
        assert_eq!(
            reduce(&mut s, Action::MonitorUnloadAll),
            vec![Effect::UnloadAll, Effect::RefreshMonitor]
        );
        assert_eq!(
            reduce(&mut s, Action::MonitorRestartAll),
            vec![Effect::RestartAll, Effect::RefreshMonitor]
        );
    }

    fn store_row(status: StoreStatus) -> StoreRow {
        StoreRow {
            id: "app-notes".into(),
            label: "Notes".into(),
            description: "Local notes.".into(),
            version: "1.0.0".into(),
            status,
        }
    }

    #[test]
    fn store_opens_fetches_and_installs() {
        let mut s = st();
        s.query = "store".into();
        assert_eq!(
            reduce(&mut s, Action::QuerySubmitted),
            vec![Effect::FetchCatalog]
        );
        assert!(s.store_open && !s.monitor_open);
        assert_eq!(s.store_status, "loading catalog…");

        let rows = vec![store_row(StoreStatus::NotInstalled)];
        reduce(
            &mut s,
            Action::StoreCatalog {
                rows: rows.clone(),
                note: None,
            },
        );
        assert_eq!(s.store_rows, rows);
        assert_eq!(s.store_status, "1 available · 0 installed");

        assert_eq!(
            reduce(&mut s, Action::StoreInstall("app-notes".into())),
            vec![Effect::InstallModule {
                id: "app-notes".into()
            }]
        );
        assert_eq!(s.store_rows[0].status, StoreStatus::Installing);

        // A second tap while the download runs must not start it twice.
        assert!(reduce(&mut s, Action::StoreInstall("app-notes".into())).is_empty());

        let mut installed = rows.clone();
        installed[0].status = StoreStatus::Installed;
        reduce(
            &mut s,
            Action::StoreInstallDone {
                ok: true,
                msg: "app-notes installed".into(),
                rows: installed.clone(),
            },
        );
        assert_eq!(s.store_rows, installed);
        assert_eq!(s.store_status, "app-notes installed");

        reduce(&mut s, Action::CloseOverlays);
        assert!(!s.store_open);
    }

    #[test]
    fn failed_install_keeps_previous_rows() {
        let mut s = st();
        let rows = vec![store_row(StoreStatus::Update)];
        reduce(
            &mut s,
            Action::StoreCatalog {
                rows: rows.clone(),
                note: Some("offline — cached catalog".into()),
            },
        );
        assert_eq!(s.store_status, "offline — cached catalog");

        reduce(&mut s, Action::StoreInstall("app-notes".into()));
        reduce(
            &mut s,
            Action::StoreInstallDone {
                ok: false,
                msg: "signature does not match".into(),
                rows: rows.clone(),
            },
        );
        assert_eq!(s.store_rows, rows);
        assert_eq!(s.store_status, "! signature does not match");

        // Unknown ids are inert, and an installed row cannot be re-installed.
        assert!(reduce(&mut s, Action::StoreInstall("app-nope".into())).is_empty());
        let mut done = rows.clone();
        done[0].status = StoreStatus::Installed;
        reduce(
            &mut s,
            Action::StoreInstallDone {
                ok: true,
                msg: "ok".into(),
                rows: done,
            },
        );
        assert!(reduce(&mut s, Action::StoreInstall("app-notes".into())).is_empty());
    }

    #[test]
    fn store_and_monitor_are_exclusive_overlays() {
        let mut s = st();
        reduce(&mut s, Action::StoreOpen);
        assert!(s.store_open && !s.monitor_open);

        s.query = "monitor".into();
        reduce(&mut s, Action::QuerySubmitted);
        assert!(s.monitor_open && !s.store_open);

        s.query = "store".into();
        reduce(&mut s, Action::QuerySubmitted);
        assert!(s.store_open && !s.monitor_open);

        // A catalog error is a status line, not an overlay close.
        reduce(&mut s, Action::StoreCatalogError("fetch failed".into()));
        assert_eq!(s.store_status, "! fetch failed");
        assert!(s.store_open);
    }

    #[test]
    fn all_apps_lists_the_store_without_tags() {
        let listing = describe_all_apps("");
        assert!(listing.starts_with("Store"), "{listing}");
        // Labels only — no `A1  `-style tags in user-facing text.
        assert!(listing.contains("Terminal") && listing.contains("Camera"));
        assert!(!listing.contains("A1") && !listing.contains("C5"));
        assert!(describe_all_apps("store").contains("Store"));
        assert!(!describe_all_apps("zzzqqq").contains("Store"));
    }
}
