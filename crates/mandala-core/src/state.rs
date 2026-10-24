//! Unidirectional-data-flow state machine.
//!
//! `reduce(state, action) -> effects` is a pure function: the UI renders
//! `AppState`, user input becomes `Action`s, and side effects are returned as
//! data (`Effect`) for the shell to execute. Nothing here touches I/O.

use crate::command::{parse_command, search_slots, Command};
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
    pub theme_id: String,
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
            theme_id: String::new(),
        }
    }
}

pub const HOME_HINT: &str = "Type to search apps.\n= 2+2   calculate\nnote ...   quick note\n> ls   terminal\n2fa   authenticator\nvault   passwords\nlsend   local transfer\ntheme next   switch theme\nmonitor   module processes";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Resize(f32),
    Gesture(Gesture),
    OpenSlot(String),
    QueryChanged(String),
    QuerySubmitted,
    CloseOverlays,
    /// A module answered a request.
    ModuleReply { module: String, ok: bool, text: String },
    /// A host-side failure (not installed, quarantined, ...).
    HostError { module: String, text: String },
    MonitorUpdated(String),
    /// Result of a theme switch attempt.
    ThemeApplied { id: String },
    Notice(String),
    /// User pressed a button on the process monitor.
    MonitorUnloadAll,
    MonitorRestartAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    CallModule { module: String, action: String, payload: String },
    RefreshMonitor,
    UnloadAll,
    RestartAll,
    SetTheme(String),
    /// Android-only: set the system wallpaper from a local image.
    SetWallpaper(String),
    /// Android-only: ask for the draw-over-other-apps permission.
    RequestOverlay,
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
        }

        Action::OpenSlot(id) => {
            state.left_open = false;
            state.right_open = false;
            match slots::by_id(&id) {
                None => state.canvas_text = format!("Unknown slot `{id}`"),
                Some(slot) if slot.kind == SlotKind::BuiltinAllApps => {
                    state.active_module = None;
                    state.canvas_text = describe_all_apps("");
                }
                Some(slot) => {
                    state.active_module = Some(slot.id.to_string());
                    state.canvas_text = format!("Opening {}...", slot.label);
                    fx.push(Effect::CallModule { module: slot.id.into(), action: "launch".into(), payload: String::new() });
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
                    fx.push(Effect::CallModule { module: "app-calculator".into(), action: "eval".into(), payload: expr });
                }
                Command::CalcSave(expr) => {
                    state.active_module = Some("app-calculator".into());
                    fx.push(Effect::CallModule { module: "app-calculator".into(), action: "save".into(), payload: expr });
                }
                Command::Note(text) => {
                    state.active_module = Some("app-notes".into());
                    fx.push(Effect::CallModule { module: "app-notes".into(), action: "append".into(), payload: text });
                }
                Command::Terminal(cmd) => {
                    state.active_module = Some("app-terminal".into());
                    state.canvas_text = format!("> {cmd}");
                    fx.push(Effect::CallModule { module: "app-terminal".into(), action: "run".into(), payload: cmd });
                }
                Command::Aegis(cmd) => {
                    state.active_module = Some("app-aegis".into());
                    fx.push(Effect::CallModule { module: "app-aegis".into(), action: "run".into(), payload: cmd });
                }
                Command::Vault(cmd) => {
                    state.active_module = Some("app-vault".into());
                    fx.push(Effect::CallModule { module: "app-vault".into(), action: "run".into(), payload: cmd });
                }
                Command::LocalSend(cmd) => {
                    state.active_module = Some("app-localsend".into());
                    fx.push(Effect::CallModule { module: "app-localsend".into(), action: "run".into(), payload: cmd });
                }
                Command::Wallpaper(path) => {
                    state.active_module = None;
                    fx.push(Effect::SetWallpaper(path));
                }
                Command::Overlay => fx.push(Effect::RequestOverlay),
                Command::Theme(key) => fx.push(Effect::SetTheme(key)),
                Command::Monitor => {
                    state.monitor_open = true;
                    fx.push(Effect::RefreshMonitor);
                }
                Command::Launch(id) => return reduce(state, Action::OpenSlot(id.to_string())),
                Command::NoMatch(q) => state.canvas_text = format!("No app matches `{q}`."),
            }
        }

        Action::ModuleReply { module, ok, text } => {
            // Ignore stale replies from a module the user already left.
            if state.active_module.as_deref() == Some(module.as_str()) || state.active_module.is_none() {
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
    }
    fx
}

/// Text listing for the All-Apps drawer / live search results.
pub fn describe_all_apps(query: &str) -> String {
    let rows: Vec<String> = if query.trim().is_empty() {
        SLOTS.iter().map(|s| format!("{}  {}", s.tag(), s.label)).collect()
    } else {
        search_slots(query, 8).iter().map(|s| format!("{}  {}", s.tag(), s.label)).collect()
    };
    if rows.is_empty() {
        format!("No app matches `{}`.", query.trim())
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
    fn opening_a_module_slot_emits_launch() {
        let mut s = st();
        s.left_open = true;
        let fx = reduce(&mut s, Action::OpenSlot("app-notes".into()));
        assert!(!s.left_open);
        assert_eq!(s.active_module.as_deref(), Some("app-notes"));
        assert_eq!(fx, vec![Effect::CallModule { module: "app-notes".into(), action: "launch".into(), payload: "".into() }]);
    }

    #[test]
    fn all_apps_is_builtin_and_calls_no_module() {
        let mut s = st();
        let fx = reduce(&mut s, Action::OpenSlot("app-drawer".into()));
        assert!(fx.is_empty());
        assert!(s.canvas_text.contains("A1  Terminal") && s.canvas_text.contains("C5  Camera"));
    }

    #[test]
    fn typing_filters_apps_live() {
        let mut s = st();
        reduce(&mut s, Action::QueryChanged("calc".into()));
        assert!(s.canvas_text.starts_with("C4  Calculator"));
        reduce(&mut s, Action::QueryChanged("".into()));
        assert_eq!(s.canvas_text, HOME_HINT);
    }

    #[test]
    fn submit_routes_commands() {
        let mut s = st();
        s.query = "= 1+1".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(s.query, "");
        assert_eq!(fx, vec![Effect::CallModule { module: "app-calculator".into(), action: "eval".into(), payload: "1+1".into() }]);

        s.query = "note hello".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx, vec![Effect::CallModule { module: "app-notes".into(), action: "append".into(), payload: "hello".into() }]);

        s.query = "theme next".into();
        assert_eq!(reduce(&mut s, Action::QuerySubmitted), vec![Effect::SetTheme("next".into())]);

        s.query = "monitor".into();
        assert_eq!(reduce(&mut s, Action::QuerySubmitted), vec![Effect::RefreshMonitor]);
        assert!(s.monitor_open);

        s.query = "> ls".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx, vec![Effect::CallModule { module: "app-terminal".into(), action: "run".into(), payload: "ls".into() }]);
        assert_eq!(s.active_module.as_deref(), Some("app-terminal"));

        s.query = "2fa code github".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx, vec![Effect::CallModule { module: "app-aegis".into(), action: "run".into(), payload: "code github".into() }]);

        s.query = "vault list".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx, vec![Effect::CallModule { module: "app-vault".into(), action: "run".into(), payload: "list".into() }]);

        s.query = "lsend recv 8080 tok!".into();
        let fx = reduce(&mut s, Action::QuerySubmitted);
        assert_eq!(fx, vec![Effect::CallModule { module: "app-localsend".into(), action: "run".into(), payload: "recv 8080 tok!".into() }]);

        s.query = "wallpaper /sdcard/a.png".into();
        assert_eq!(reduce(&mut s, Action::QuerySubmitted), vec![Effect::SetWallpaper("/sdcard/a.png".into())]);

        s.query = "overlay".into();
        assert_eq!(reduce(&mut s, Action::QuerySubmitted), vec![Effect::RequestOverlay]);

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
        reduce(&mut s, Action::ModuleReply { module: "app-calculator".into(), ok: true, text: "4".into() });
        assert_eq!(s.canvas_text, "notes");
        reduce(&mut s, Action::ModuleReply { module: "app-notes".into(), ok: false, text: "boom".into() });
        assert_eq!(s.canvas_text, "! boom");
    }

    #[test]
    fn host_error_clears_active_module() {
        let mut s = st();
        s.active_module = Some("app-vault".into());
        reduce(&mut s, Action::HostError { module: "app-vault".into(), text: "quarantined".into() });
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
        assert_eq!(reduce(&mut s, Action::MonitorUnloadAll), vec![Effect::UnloadAll, Effect::RefreshMonitor]);
        assert_eq!(reduce(&mut s, Action::MonitorRestartAll), vec![Effect::RestartAll, Effect::RefreshMonitor]);
    }
}
