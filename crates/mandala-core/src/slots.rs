//! The fixed 15-slot tri-panel application matrix (5 slots x 3 panels).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Bottom dock: high-intent core.
    A,
    /// Left drawer: passive information & retrieval.
    B,
    /// Right drawer: active utilities & security.
    C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// Loaded as a signed dynamic module.
    Module,
    /// Implemented inside the host (the fuzzy All-Apps drawer).
    BuiltinAllApps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotDef {
    pub panel: Panel,
    /// 1-based position inside the panel.
    pub index: u8,
    /// Module id == file stem of the signed binary (`<id>.so`).
    pub id: &'static str,
    pub label: &'static str,
    pub kind: SlotKind,
}

impl SlotDef {
    /// Short tag such as `A1`, `B4`, `C2`.
    pub fn tag(&self) -> String {
        let p = match self.panel {
            Panel::A => 'A',
            Panel::B => 'B',
            Panel::C => 'C',
        };
        format!("{p}{}", self.index)
    }
}

macro_rules! slot {
    ($p:ident, $i:expr, $id:expr, $label:expr) => {
        SlotDef { panel: Panel::$p, index: $i, id: $id, label: $label, kind: SlotKind::Module }
    };
}

pub const SLOTS: [SlotDef; 15] = [
    // Panel A: bottom dock
    slot!(A, 1, "app-terminal", "Terminal"),
    slot!(A, 2, "app-notes", "Notes"),
    SlotDef { panel: Panel::A, index: 3, id: "app-drawer", label: "All Apps", kind: SlotKind::BuiltinAllApps },
    slot!(A, 4, "app-chat", "Secure Chat"),
    slot!(A, 5, "app-ai", "AI Assistant"),
    // Panel B: left drawer
    slot!(B, 1, "app-weather", "Weather"),
    slot!(B, 2, "app-rss", "RSS Reader"),
    slot!(B, 3, "app-tasks", "Tasks & Habits"),
    slot!(B, 4, "app-reader", "Doc Reader"),
    slot!(B, 5, "app-gallery", "Gallery"),
    // Panel C: right drawer
    slot!(C, 1, "app-aegis", "2FA Authenticator"),
    slot!(C, 2, "app-vault", "Password Vault"),
    slot!(C, 3, "app-localsend", "LocalSend"),
    slot!(C, 4, "app-calculator", "Calculator"),
    slot!(C, 5, "app-camera", "Camera"),
];

pub fn by_panel(panel: Panel) -> impl Iterator<Item = &'static SlotDef> {
    SLOTS.iter().filter(move |s| s.panel == panel)
}

pub fn by_id(id: &str) -> Option<&'static SlotDef> {
    SLOTS.iter().find(|s| s.id == id)
}

pub fn by_tag(tag: &str) -> Option<&'static SlotDef> {
    SLOTS.iter().find(|s| s.tag().eq_ignore_ascii_case(tag))
}

/// Ids of all slots that are backed by a loadable module.
pub fn module_ids() -> impl Iterator<Item = &'static str> {
    SLOTS.iter().filter(|s| s.kind == SlotKind::Module).map(|s| s.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn matrix_is_5x3_with_unique_ids() {
        for p in [Panel::A, Panel::B, Panel::C] {
            assert_eq!(by_panel(p).count(), 5);
        }
        let ids: HashSet<_> = SLOTS.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 15);
        let tags: HashSet<_> = SLOTS.iter().map(|s| s.tag()).collect();
        assert_eq!(tags.len(), 15);
    }

    #[test]
    fn lookups_work() {
        assert_eq!(by_tag("c4").unwrap().id, "app-calculator");
        assert_eq!(by_id("app-notes").unwrap().tag(), "A2");
        assert!(by_id("nope").is_none());
    }
}
