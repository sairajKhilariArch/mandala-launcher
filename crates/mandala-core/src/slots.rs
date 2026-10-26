//! The fixed 15-slot tri-panel application matrix (5 slots x 3 panels).
//!
//! `SLOTS` is the *assignment order*: which panel each known app belongs to
//! and how fuzzy search finds it. What a panel actually *shows* is decided by
//! [`compose_panel`], which overlays the set of installed modules — the
//! panels are app holders, not permanent apps. Installed modules outside the
//! matrix flow into free positions; uninstalled ones render as empty
//! placeholders.

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
    /// Short tag such as `A1`, `B4`, `C2`. Search/debug only — the UI never
    /// shows it (tags were explicitly rejected as user-facing labels).
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
        SlotDef {
            panel: Panel::$p,
            index: $i,
            id: $id,
            label: $label,
            kind: SlotKind::Module,
        }
    };
}

pub const SLOTS: [SlotDef; 15] = [
    // Panel A: bottom dock
    slot!(A, 1, "app-terminal", "Terminal"),
    slot!(A, 2, "app-notes", "Notes"),
    SlotDef {
        panel: Panel::A,
        index: 3,
        id: "app-drawer",
        label: "All Apps",
        kind: SlotKind::BuiltinAllApps,
    },
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
    SLOTS
        .iter()
        .filter(|s| s.kind == SlotKind::Module)
        .map(|s| s.id)
}

/// Display label for a module id: the matrix label when known, otherwise a
/// humanization of the id (`app-foo-bar` -> `Foo Bar`) so modules added after
/// the matrix was written still render sensibly.
pub fn label_for(id: &str) -> String {
    if let Some(s) = by_id(id) {
        return s.label.to_string();
    }
    let mut out = String::new();
    for word in id
        .trim_start_matches("app-")
        .split(|c: char| c == '-' || c == '_')
        .filter(|w| !w.is_empty())
    {
        let mut cs = word.chars();
        if let Some(first) = cs.next() {
            out.extend(first.to_uppercase());
            out.extend(cs.as_str().chars());
        }
        if !out.is_empty() {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    if out.is_empty() {
        id.to_string()
    } else {
        out
    }
}

/// Monogram for a label: its first alphanumeric character, uppercased. Zero
/// assets, works for modules added later; the text-first aesthetic does not
/// want photographic icons anyway.
pub fn glyph_for(label: &str) -> String {
    label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// One rendered position of a panel: either an app (module id) or an empty
/// holder (`placeholder`), which opens the Store when tapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSlot {
    pub id: String,
    pub label: String,
    pub placeholder: bool,
}

/// Positions per panel: the matrix keeps 5; panels are app holders, so a
/// full panel is the cap and overflow is reachable via the All Apps overlay.
const PANEL_CAPACITY: usize = 5;

/// Compose what the panels actually show, given the ids installed on disk.
///
/// Rules (all deterministic):
///  1. The builtin All Apps entry keeps its Panel A position 3 slot.
///  2. Matrix modules fill their assigned position only when installed.
///  3. Installed modules *unknown to the matrix* (apps added later) fill the
///     free positions globally in panel order A, B, C — one module appears on
///     exactly one panel.
///  4. Remaining positions are empty placeholders (`id == ""`).
pub fn compose_all(installed: &[String]) -> Vec<(Panel, Vec<PanelSlot>)> {
    let is_installed = |id: &str| installed.iter().any(|i| i == id);

    // Pass 1: matrix entries (builtin always; modules when installed).
    let mut per_panel: Vec<(Panel, Vec<Option<PanelSlot>>)> = [Panel::A, Panel::B, Panel::C]
        .into_iter()
        .map(|panel| {
            let slots = by_panel(panel)
                .map(|s| match s.kind {
                    SlotKind::BuiltinAllApps => Some(PanelSlot {
                        id: s.id.to_string(),
                        label: s.label.to_string(),
                        placeholder: false,
                    }),
                    SlotKind::Module if is_installed(s.id) => Some(PanelSlot {
                        id: s.id.to_string(),
                        label: s.label.to_string(),
                        placeholder: false,
                    }),
                    _ => None,
                })
                .collect();
            (panel, slots)
        })
        .collect();

    // Pass 2: overflow — sorted unknown ids fill free positions in panel
    // order, so each module lands on exactly one panel.
    let mut overflow: Vec<String> = installed
        .iter()
        .filter(|id| by_id(id).is_none())
        .cloned()
        .collect();
    overflow.sort();
    let mut it = overflow.into_iter();
    for (_, slots) in per_panel.iter_mut() {
        for free in slots.iter_mut().filter(|s| s.is_none()) {
            match it.next() {
                Some(id) => {
                    let label = label_for(&id);
                    *free = Some(PanelSlot {
                        id,
                        label,
                        placeholder: false,
                    });
                }
                None => break,
            }
        }
    }

    // Pass 3: remaining positions are empty holders.
    per_panel
        .into_iter()
        .map(|(panel, slots)| {
            let slots = slots
                .into_iter()
                .map(|s| {
                    s.unwrap_or_else(|| PanelSlot {
                        id: String::new(),
                        label: String::new(),
                        placeholder: true,
                    })
                })
                .collect();
            (panel, slots)
        })
        .collect()
}

/// Compose a single panel (view over [`compose_all`], so overflow ids are
/// distributed globally rather than repeated on every panel).
pub fn compose_panel(panel: Panel, installed: &[String]) -> Vec<PanelSlot> {
    compose_all(installed)
        .into_iter()
        .find(|(p, _)| *p == panel)
        .map(|(_, slots)| slots)
        .unwrap_or_default()
}

/// Guard used by tests: no panel ever exceeds its capacity.
pub fn panel_capacity() -> usize {
    PANEL_CAPACITY
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

    #[test]
    fn empty_install_is_placeholders_plus_builtin() {
        let none: Vec<String> = vec![];
        let a = compose_panel(Panel::A, &none);
        assert_eq!(a.len(), panel_capacity());
        // Builtin All Apps keeps its home at A3.
        assert_eq!(a[2].id, "app-drawer");
        assert!(!a[2].placeholder);
        // Everything else is an empty holder.
        for (i, s) in a.iter().enumerate() {
            if i != 2 {
                assert!(s.placeholder, "A{i} should be a placeholder");
                assert_eq!(s.id, "");
            }
        }
        assert!(compose_panel(Panel::B, &none).iter().all(|s| s.placeholder));
        assert!(compose_panel(Panel::C, &none).iter().all(|s| s.placeholder));
    }

    #[test]
    fn installed_matrix_module_fills_its_position() {
        let inst = vec!["app-notes".to_string(), "app-weather".to_string()];
        let a = compose_panel(Panel::A, &inst);
        assert_eq!(a[1].id, "app-notes");
        assert_eq!(a[1].label, "Notes");
        assert!(!a[1].placeholder);
        // Weather lives on B, not A: A stays otherwise placeholder.
        assert_eq!(a[0].placeholder, true);
        let b = compose_panel(Panel::B, &inst);
        assert_eq!(b[0].id, "app-weather");
        assert_eq!(b[0].label, "Weather");
    }

    #[test]
    fn uninstalled_matrix_module_does_not_render() {
        // Only notes installed; terminal (A1) must be an empty holder.
        let inst = vec!["app-notes".to_string()];
        let a = compose_panel(Panel::A, &inst);
        assert!(a[0].placeholder, "uninstalled app-terminal must not render");
        assert_eq!(a[1].id, "app-notes");
    }

    #[test]
    fn overflow_module_claims_free_positions_in_order() {
        // `app-zzz` is unknown to the matrix; with A1/A2 empty it lands in A1.
        let inst = vec!["app-zzz".to_string()];
        let a = compose_panel(Panel::A, &inst);
        assert_eq!(a[0].id, "app-zzz");
        assert_eq!(a[0].label, "Zzz");
        assert!(!a[0].placeholder);
        assert!(a[1].placeholder);

        // Two overflow ids: sorted order, first free positions.
        let inst = vec!["app-zzz".to_string(), "app-aaa".to_string()];
        let a = compose_panel(Panel::A, &inst);
        assert_eq!(a[0].id, "app-aaa");
        assert_eq!(a[1].id, "app-zzz");
    }

    #[test]
    fn builtin_is_never_displaced() {
        // A3 must stay All Apps even with overflow modules available.
        let inst: Vec<String> = (0..10).map(|i| format!("app-extra-{i}")).collect();
        let a = compose_panel(Panel::A, &inst);
        assert_eq!(a[2].id, "app-drawer");
        assert_eq!(a.len(), panel_capacity());
        // 5 positions on A, one taken by the builtin: 4 overflow modules show.
        let shown = a.iter().filter(|s| !s.placeholder).count();
        assert_eq!(shown, 5, "builtin + 4 overflow apps fill A exactly");
    }

    #[test]
    fn overflow_never_exceeds_panel_capacity() {
        // 14 known modules uninstalled + 40 extras: panels stay at 5.
        let inst: Vec<String> = (0..40).map(|i| format!("app-x{i:02}")).collect();
        for (panel, slots) in compose_all(&inst) {
            assert_eq!(slots.len(), panel_capacity(), "{panel:?} over capacity");
        }
    }

    #[test]
    fn label_for_known_and_unknown() {
        assert_eq!(label_for("app-notes"), "Notes");
        assert_eq!(label_for("app-foo-bar"), "Foo Bar");
        assert_eq!(label_for("app-aegis"), "2FA Authenticator");
        assert_eq!(label_for("weird"), "Weird");
    }

    #[test]
    fn glyph_for_takes_first_alnum_uppercased() {
        assert_eq!(glyph_for("Notes"), "N");
        assert_eq!(glyph_for("2FA Authenticator"), "2");
        assert_eq!(glyph_for("app"), "A");
        assert_eq!(glyph_for("!!!"), "?");
    }
}
