//! Declarative theme engine: JSON manifest -> validated, resolved tokens.

use crate::error::{MandalaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const PRESET_AURORA: &str = include_str!("../../../themes/aurora-slate.json");
pub const PRESET_OLED: &str = include_str!("../../../themes/minimalist-oled.json");
pub const PRESET_CEL: &str = include_str!("../../../themes/cel-shaded.json");
pub const PRESET_RETRO: &str = include_str!("../../../themes/retro-terminal.json");
pub const PRESET_HYPR: &str = include_str!("../../../themes/hypr-rice.json");
pub const PRESET_KITTY: &str = include_str!("../../../themes/kitty.json");

/// Per-app theme presets: shown while that module is the active view.
pub const APP_PRESET_TERMINAL: &str = include_str!("../../../themes/apps/app-terminal.json");
pub const APP_PRESET_CAMERA: &str = include_str!("../../../themes/apps/app-camera.json");
pub const APP_PRESET_NOTES: &str = include_str!("../../../themes/apps/app-notes.json");
pub const APP_PRESET_CHAT: &str = include_str!("../../../themes/apps/app-chat.json");
pub const APP_PRESET_VAULT: &str = include_str!("../../../themes/apps/app-vault.json");
/// The Store overlay is a builtin (no module file), keyed `app-store`.
pub const APP_PRESET_STORE: &str = include_str!("../../../themes/apps/app-store.json");
pub const APP_PRESET_AI: &str = include_str!("../../../themes/apps/app-ai.json");
pub const APP_PRESET_WEATHER: &str = include_str!("../../../themes/apps/app-weather.json");
pub const APP_PRESET_RSS: &str = include_str!("../../../themes/apps/app-rss.json");
pub const APP_PRESET_TASKS: &str = include_str!("../../../themes/apps/app-tasks.json");
pub const APP_PRESET_READER: &str = include_str!("../../../themes/apps/app-reader.json");
pub const APP_PRESET_GALLERY: &str = include_str!("../../../themes/apps/app-gallery.json");
pub const APP_PRESET_AEGIS: &str = include_str!("../../../themes/apps/app-aegis.json");
pub const APP_PRESET_LOCALSEND: &str = include_str!("../../../themes/apps/app-localsend.json");
pub const APP_PRESET_CALCULATOR: &str = include_str!("../../../themes/apps/app-calculator.json");
pub const APP_PRESET_DRAWER: &str = include_str!("../../../themes/apps/app-drawer.json");
pub const APP_PRESET_MONITOR: &str = include_str!("../../../themes/apps/app-monitor.json");

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ThemeManifest {
    pub theme_id: String,
    pub theme_name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    pub tokens: ThemeTokens,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ThemeTokens {
    pub canvas_bg: String,
    pub surface_primary: String,
    pub surface_border: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub accent_focus: String,
    /// Colour of primary-action buttons; themes without it fall back to the
    /// trending orange so older user themes stay valid.
    #[serde(default = "default_cta")]
    pub cta_accent: String,
    /// Label colour on primary (CTA) buttons. Absent = the theme's own
    /// canvas colour (the historical behaviour), overridden per theme when
    /// that would fail WCAG AA against the CTA fill.
    #[serde(default)]
    pub on_cta: Option<String>,
    #[serde(default)]
    pub border_width_px: f32,
    #[serde(default = "default_radius")]
    pub corner_radius_px: f32,
    #[serde(default = "default_font")]
    pub font_family: String,
}

fn default_radius() -> f32 {
    4.0
}
fn default_cta() -> String {
    "#F97316".into()
}
fn default_font() -> String {
    "Inter".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
    pub fn parse(s: &str) -> Result<Self> {
        let bad = || MandalaError::Theme(format!("invalid color `{s}`"));
        let h = s.trim().strip_prefix('#').ok_or_else(bad)?;
        if !h.is_ascii() {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| bad());
        match h.len() {
            3 => {
                let n = |i: usize| {
                    u8::from_str_radix(&h[i..i + 1], 16)
                        .map(|v| v * 17)
                        .map_err(|_| bad())
                };
                Ok(Self {
                    r: n(0)?,
                    g: n(1)?,
                    b: n(2)?,
                    a: 255,
                })
            }
            6 => Ok(Self {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: 255,
            }),
            8 => Ok(Self {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => Err(bad()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTheme {
    pub id: String,
    pub name: String,
    pub canvas_bg: Rgba,
    pub surface_primary: Rgba,
    pub surface_border: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub accent_focus: Rgba,
    pub cta_accent: Rgba,
    /// Resolved: explicit `on_cta` token, else `canvas_bg` (see ThemeTokens).
    pub on_cta: Rgba,
    pub border_width_px: f32,
    pub corner_radius_px: f32,
    pub font_family: String,
}

impl ThemeManifest {
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| MandalaError::Theme(e.to_string()))
    }

    pub fn resolve(&self) -> Result<ResolvedTheme> {
        let t = &self.tokens;
        if self.theme_id.trim().is_empty() {
            return Err(MandalaError::Theme("theme_id is empty".into()));
        }
        if !(0.0..=16.0).contains(&t.border_width_px) {
            return Err(MandalaError::Theme(
                "border_width_px must be within 0..=16".into(),
            ));
        }
        if !(0.0..=64.0).contains(&t.corner_radius_px) {
            return Err(MandalaError::Theme(
                "corner_radius_px must be within 0..=64".into(),
            ));
        }
        let canvas_bg = Rgba::parse(&t.canvas_bg)?;
        Ok(ResolvedTheme {
            id: self.theme_id.clone(),
            name: self.theme_name.clone(),
            canvas_bg,
            surface_primary: Rgba::parse(&t.surface_primary)?,
            surface_border: Rgba::parse(&t.surface_border)?,
            text_primary: Rgba::parse(&t.text_primary)?,
            text_secondary: Rgba::parse(&t.text_secondary)?,
            accent_focus: Rgba::parse(&t.accent_focus)?,
            cta_accent: Rgba::parse(&t.cta_accent)?,
            on_cta: match &t.on_cta {
                Some(s) => Rgba::parse(s)?,
                None => canvas_bg,
            },
            border_width_px: t.border_width_px,
            corner_radius_px: t.corner_radius_px,
            font_family: t.font_family.clone(),
        })
    }
}

/// Holds the available themes, the active one, per-app overrides loaded from
/// `themes/apps/*.json` or assigned at runtime (`theme <app> <name>`), that
/// apply while an app is open.
pub struct ThemeManager {
    themes: Vec<ResolvedTheme>,
    current: usize,
    apps: HashMap<String, ResolvedTheme>,
    /// Runtime assignments (app id → theme id); win over `apps`.
    overrides: HashMap<String, String>,
}

impl ThemeManager {
    /// Built-in presets (first one is the default).
    pub fn with_presets() -> Self {
        let themes = [
            PRESET_AURORA,
            PRESET_OLED,
            PRESET_CEL,
            PRESET_RETRO,
            PRESET_HYPR,
            PRESET_KITTY,
        ]
        .iter()
        .map(|j| {
            ThemeManifest::from_json(j)
                .and_then(|m| m.resolve())
                .expect("built-in theme must be valid")
        })
        .collect();
        let apps = [
            ("app-terminal", APP_PRESET_TERMINAL),
            ("app-camera", APP_PRESET_CAMERA),
            ("app-notes", APP_PRESET_NOTES),
            ("app-chat", APP_PRESET_CHAT),
            ("app-vault", APP_PRESET_VAULT),
            ("app-store", APP_PRESET_STORE),
            ("app-ai", APP_PRESET_AI),
            ("app-weather", APP_PRESET_WEATHER),
            ("app-rss", APP_PRESET_RSS),
            ("app-tasks", APP_PRESET_TASKS),
            ("app-reader", APP_PRESET_READER),
            ("app-gallery", APP_PRESET_GALLERY),
            ("app-aegis", APP_PRESET_AEGIS),
            ("app-localsend", APP_PRESET_LOCALSEND),
            ("app-calculator", APP_PRESET_CALCULATOR),
            ("app-drawer", APP_PRESET_DRAWER),
            ("app-monitor", APP_PRESET_MONITOR),
        ]
        .iter()
        .map(|(id, j)| {
            let t = ThemeManifest::from_json(j)
                .and_then(|m| m.resolve())
                .expect("built-in app theme must be valid");
            ((*id).to_string(), t)
        })
        .collect();
        Self {
            themes,
            current: 0,
            apps,
            overrides: HashMap::new(),
        }
    }

    /// Add user themes from `dir/*.json`; invalid files are skipped and
    /// reported in the returned warnings. Same-id themes replace presets.
    pub fn load_user_dir(&mut self, dir: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return warnings;
        };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let loaded = std::fs::read_to_string(&p)
                .map_err(|e| MandalaError::io(&p, e))
                .and_then(|s| ThemeManifest::from_json(&s))
                .and_then(|m| m.resolve());
            match loaded {
                Ok(t) => self.add(t),
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            }
        }
        warnings
    }

    /// Per-app themes from `dir/<app-id>.json`; the file stem is the app id.
    /// Overrides built-in app themes. Fits the same manifest shape.
    pub fn load_app_dir(&mut self, dir: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return warnings;
        };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let loaded = std::fs::read_to_string(&p)
                .map_err(|e| MandalaError::io(&p, e))
                .and_then(|s| ThemeManifest::from_json(&s))
                .and_then(|m| m.resolve());
            match loaded {
                Ok(t) => {
                    self.apps.insert(id.to_string(), t);
                }
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            }
        }
        warnings
    }

    /// The theme to show while app `id` is the active view: a runtime
    /// assignment wins over the loaded per-app manifest.
    pub fn for_app(&self, id: &str) -> Option<&ResolvedTheme> {
        self.overrides
            .get(id)
            .and_then(|tid| self.themes.iter().find(|t| &t.id == tid))
            .or_else(|| self.apps.get(id))
    }

    /// Index of a theme by exact id, unique id prefix, or (used by `select`).
    fn resolve_index(&self, key: &str) -> Option<usize> {
        let key = key.trim().to_ascii_lowercase();
        let exact = self
            .themes
            .iter()
            .position(|t| t.id.to_ascii_lowercase() == key);
        exact.or_else(|| {
            let hits: Vec<_> = self
                .themes
                .iter()
                .enumerate()
                .filter(|(_, t)| t.id.to_ascii_lowercase().starts_with(&key))
                .map(|(i, _)| i)
                .collect();
            (hits.len() == 1).then(|| hits[0])
        })
    }

    /// Assign any known theme to `app`: applies while that app is on screen.
    pub fn assign_app(&mut self, app: &str, key: &str) -> Result<&ResolvedTheme> {
        let idx = self.resolve_index(key).ok_or_else(|| {
            MandalaError::Theme(format!(
                "unknown theme `{key}` (have: {})",
                self.ids().join(", ")
            ))
        })?;
        let tid = self.themes[idx].id.clone();
        self.overrides.insert(app.to_string(), tid);
        Ok(&self.themes[idx])
    }

    /// Forget `app`'s assignment (next view uses its manifest or builtin).
    /// Returns `false` when there was nothing assigned.
    pub fn clear_app(&mut self, app: &str) -> bool {
        self.overrides.remove(app).is_some()
    }

    /// Runtime assignments for persistence.
    pub fn overrides(&self) -> &HashMap<String, String> {
        &self.overrides
    }

    /// Restore a persisted assignment; only accepted when the theme id is
    /// known, so a removed theme degrades to the app manifest instead of
    /// silently breaking the app view.
    pub fn restore_override(&mut self, app: &str, theme_id: &str) {
        if self.themes.iter().any(|t| t.id == theme_id) {
            self.overrides.insert(app.to_string(), theme_id.to_string());
        }
    }

    pub fn add(&mut self, theme: ResolvedTheme) {
        if let Some(i) = self.themes.iter().position(|t| t.id == theme.id) {
            self.themes[i] = theme;
        } else {
            self.themes.push(theme);
        }
    }

    pub fn current(&self) -> &ResolvedTheme {
        &self.themes[self.current]
    }

    pub fn ids(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.id.as_str()).collect()
    }

    /// Select by exact id, unique id prefix, or the keyword `next`.
    pub fn select(&mut self, key: &str) -> Result<&ResolvedTheme> {
        let key = key.trim();
        if key.eq_ignore_ascii_case("next") {
            self.current = (self.current + 1) % self.themes.len();
            return Ok(self.current());
        }
        match self.resolve_index(key) {
            Some(i) => {
                self.current = i;
                Ok(self.current())
            }
            None => Err(MandalaError::Theme(format!(
                "unknown theme `{key}` (have: {})",
                self.ids().join(", ")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse_and_resolve() {
        let m = ThemeManager::with_presets();
        assert_eq!(
            m.ids(),
            [
                "aurora-slate-dark",
                "minimalist-oled-dark",
                "cel-shaded",
                "retro-terminal",
                "hypr-rice",
                "kitty"
            ]
        );
        assert_eq!(
            m.current().canvas_bg,
            Rgba {
                r: 0x0B,
                g: 0x0F,
                b: 0x14,
                a: 255
            }
        );
        assert_eq!(
            m.current().accent_focus,
            Rgba {
                r: 0x2D,
                g: 0xD4,
                b: 0xBF,
                a: 255
            }
        );
        assert_eq!(
            m.current().cta_accent,
            Rgba {
                r: 0xF9,
                g: 0x73,
                b: 0x16,
                a: 255
            }
        );
    }

    #[test]
    fn color_parsing() {
        assert_eq!(
            Rgba::parse("#fff").unwrap(),
            Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255
            }
        );
        assert_eq!(Rgba::parse("#00FF6680").unwrap().a, 0x80);
        for bad in ["", "000000", "#12", "#GGGGGG", "#12345", "#ééé"] {
            assert!(Rgba::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn selection() {
        let mut m = ThemeManager::with_presets();
        assert_eq!(m.select("cel").unwrap().id, "cel-shaded");
        assert_eq!(m.select("next").unwrap().id, "retro-terminal");
        assert_eq!(m.select("next").unwrap().id, "hypr-rice");
        assert_eq!(m.select("next").unwrap().id, "kitty");
        // Wraps back to the first preset (the default).
        assert_eq!(m.select("next").unwrap().id, "aurora-slate-dark");
        assert!(m.select("nope").is_err());
        // "r" is only a prefix of one theme id
        assert_eq!(m.select("r").unwrap().id, "retro-terminal");
        // Themes without the cta token fall back to the default orange.
        let m = ThemeManifest::from_json(PRESET_OLED).unwrap();
        assert_eq!(m.resolve().unwrap().cta_accent.r, 0xF9);
    }

    #[test]
    fn validation_rejects_bad_tokens() {
        let mut m = ThemeManifest::from_json(PRESET_OLED).unwrap();
        m.tokens.border_width_px = 99.0;
        assert!(m.resolve().is_err());
        let mut m = ThemeManifest::from_json(PRESET_OLED).unwrap();
        m.tokens.canvas_bg = "red".into();
        assert!(m.resolve().is_err());
        assert!(ThemeManifest::from_json("{").is_err());
    }

    #[test]
    fn app_themes_builtin_and_overridable() {
        let m = ThemeManager::with_presets();
        assert!(m.for_app("app-bogus").is_none());
        assert_eq!(
            m.for_app("app-terminal").unwrap().id,
            "app-terminal-phosphor"
        );
        assert_eq!(m.for_app("app-notes").unwrap().id, "app-notes-paper");
        assert_eq!(m.for_app("app-chat").unwrap().id, "app-chat-wave");
        assert_eq!(m.for_app("app-vault").unwrap().id, "app-vault-iron");
        assert_eq!(m.for_app("app-store").unwrap().id, "app-store-market");
        assert_eq!(m.for_app("app-camera").unwrap().font_family, "Inter");

        let dir = tempfile::tempdir().unwrap();
        let mut custom = ThemeManifest::from_json(APP_PRESET_TERMINAL).unwrap();
        custom.tokens.canvas_bg = "#123123".into();
        std::fs::write(
            dir.path().join("app-terminal.json"),
            serde_json::to_string(&custom).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.path().join("app-broken.json"), "{").unwrap();
        std::fs::write(dir.path().join("app-sketch.txt"), "ignored").unwrap();
        let mut m = ThemeManager::with_presets();
        let warnings = m.load_app_dir(dir.path());
        assert_eq!(warnings.len(), 1, "exactly the bad json is reported");
        assert!(warnings[0].contains("app-broken.json"));
        // A user file shares the id, it replaces the preset (test id matches).
        assert_eq!(
            m.for_app("app-terminal").unwrap().id,
            "app-terminal-phosphor"
        );
        let t = m.for_app("app-terminal").unwrap();
        assert_eq!(
            t.canvas_bg,
            Rgba {
                r: 0x12,
                g: 0x31,
                b: 0x23,
                a: 255
            }
        );
    }

    #[test]
    fn app_assignments_win_and_reset() {
        let mut m = ThemeManager::with_presets();
        assert_eq!(m.for_app("app-notes").unwrap().id, "app-notes-paper");
        // Assign by unique prefix.
        assert_eq!(m.assign_app("app-notes", "cel").unwrap().id, "cel-shaded");
        assert_eq!(m.for_app("app-notes").unwrap().id, "cel-shaded");
        assert!(m.assign_app("app-notes", "zzz").is_err());
        // Assignments persist and restore only for known ids.
        let json = serde_json::to_string(m.overrides()).unwrap();
        let restored: HashMap<String, String> = serde_json::from_str(&json).unwrap();
        let mut m = ThemeManager::with_presets();
        m.restore_override("app-notes", "cel-shaded");
        m.restore_override("app-weather", "gone-theme");
        assert_eq!(m.for_app("app-notes").unwrap().id, "cel-shaded");
        assert_eq!(m.for_app("app-weather").unwrap().id, "app-weather-horizon");
        assert!(restored.contains_key("app-notes"));
        // Reset falls back to the builtin manifest.
        assert!(m.clear_app("app-notes"));
        assert!(!m.clear_app("app-notes"));
        assert_eq!(m.for_app("app-notes").unwrap().id, "app-notes-paper");
    }

    /// Every module slot and every builtin overlay starts with a theme of
    /// its own; adding a slot without a manifest fails here.
    #[test]
    fn builtin_app_themes_cover_every_surface() {
        let m = ThemeManager::with_presets();
        for slot in crate::slots::SLOTS.iter() {
            assert!(
                m.for_app(slot.id).is_some(),
                "{} has no builtin theme",
                slot.id
            );
        }
        for overlay in ["app-store", "app-monitor"] {
            assert!(
                m.for_app(overlay).is_some(),
                "{overlay} has no builtin theme"
            );
        }
    }

    /// WCAG 2.1 relative luminance of an sRGB colour.
    fn luminance(c: Rgba) -> f64 {
        let lin = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG 2.1 contrast ratio (1..=21) between two opaque colours.
    fn contrast(a: Rgba, b: Rgba) -> f64 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Every builtin manifest must keep body text readable where the UI
    /// actually renders it: text on the canvas, text on the primary surface,
    /// and the label on a primary (CTA) button. 4.5:1 is the WCAG AA floor
    /// for normal text; this locks future theme edits to it.
    #[test]
    fn builtin_themes_meet_body_contrast() {
        const MIN: f64 = 4.5;
        let all = [
            PRESET_AURORA,
            PRESET_OLED,
            PRESET_CEL,
            PRESET_RETRO,
            PRESET_HYPR,
            PRESET_KITTY,
            APP_PRESET_TERMINAL,
            APP_PRESET_CAMERA,
            APP_PRESET_NOTES,
            APP_PRESET_CHAT,
            APP_PRESET_VAULT,
            APP_PRESET_STORE,
            APP_PRESET_AI,
            APP_PRESET_WEATHER,
            APP_PRESET_RSS,
            APP_PRESET_TASKS,
            APP_PRESET_READER,
            APP_PRESET_GALLERY,
            APP_PRESET_AEGIS,
            APP_PRESET_LOCALSEND,
            APP_PRESET_CALCULATOR,
            APP_PRESET_DRAWER,
            APP_PRESET_MONITOR,
        ];
        let mut failures = Vec::new();
        for json in all {
            let t = ThemeManifest::from_json(json)
                .and_then(|m| m.resolve())
                .expect("builtin theme must resolve");
            let pairs = [
                ("text_primary/canvas", t.text_primary, t.canvas_bg),
                ("text_primary/surface", t.text_primary, t.surface_primary),
                ("text_secondary/canvas", t.text_secondary, t.canvas_bg),
                (
                    "text_secondary/surface",
                    t.text_secondary,
                    t.surface_primary,
                ),
                // ActionButton: the theme's on-cta label on the CTA fill.
                ("cta label/cta", t.on_cta, t.cta_accent),
            ];
            for (what, fg, bg) in pairs {
                let r = contrast(fg, bg);
                if r < MIN {
                    failures.push(format!("{} {what}: {r:.2}:1", t.id));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "theme contrast below {MIN}:1:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn contrast_ratio_math() {
        // Known WCAG values: black on white is 21:1, same colour is 1:1.
        let black = Rgba::parse("#000000").unwrap();
        let white = Rgba::parse("#FFFFFF").unwrap();
        assert!((contrast(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast(white, white) - 1.0).abs() < 1e-9);
        // #767676 is the classic lightest-grey passing 4.5:1 on white.
        let grey = Rgba::parse("#767676").unwrap();
        assert!(contrast(grey, white) >= 4.5);
        assert!(contrast(Rgba::parse("#777777").unwrap(), white) < 4.5);
    }

    #[test]
    fn user_dir_overrides_and_reports_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut custom = ThemeManifest::from_json(PRESET_OLED).unwrap();
        custom.theme_id = "mine".into();
        custom.tokens.canvas_bg = "#102030".into();
        std::fs::write(
            dir.path().join("mine.json"),
            serde_json::to_string(&custom).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ nope").unwrap();
        std::fs::write(dir.path().join("readme.txt"), "ignored").unwrap();

        let mut m = ThemeManager::with_presets();
        let warnings = m.load_user_dir(dir.path());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            m.select("mine").unwrap().canvas_bg,
            Rgba {
                r: 0x10,
                g: 0x20,
                b: 0x30,
                a: 255
            }
        );
    }
}
