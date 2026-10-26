//! Declarative theme engine: JSON manifest -> validated, resolved tokens.

use crate::error::{MandalaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const PRESET_AURORA: &str = include_str!("../../../themes/aurora-slate.json");
pub const PRESET_OLED: &str = include_str!("../../../themes/minimalist-oled.json");
pub const PRESET_CEL: &str = include_str!("../../../themes/cel-shaded.json");
pub const PRESET_RETRO: &str = include_str!("../../../themes/retro-terminal.json");

/// Per-app theme presets: shown while that module is the active view.
pub const APP_PRESET_TERMINAL: &str = include_str!("../../../themes/apps/app-terminal.json");
pub const APP_PRESET_CAMERA: &str = include_str!("../../../themes/apps/app-camera.json");

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
        Ok(ResolvedTheme {
            id: self.theme_id.clone(),
            name: self.theme_name.clone(),
            canvas_bg: Rgba::parse(&t.canvas_bg)?,
            surface_primary: Rgba::parse(&t.surface_primary)?,
            surface_border: Rgba::parse(&t.surface_border)?,
            text_primary: Rgba::parse(&t.text_primary)?,
            text_secondary: Rgba::parse(&t.text_secondary)?,
            accent_focus: Rgba::parse(&t.accent_focus)?,
            cta_accent: Rgba::parse(&t.cta_accent)?,
            border_width_px: t.border_width_px,
            corner_radius_px: t.corner_radius_px,
            font_family: t.font_family.clone(),
        })
    }
}

/// Holds the available themes, the active one, and per-app overrides
/// (`<home>/themes/apps/<app-id>.json`) that apply while an app is open.
pub struct ThemeManager {
    themes: Vec<ResolvedTheme>,
    current: usize,
    apps: HashMap<String, ResolvedTheme>,
}

impl ThemeManager {
    /// Built-in presets (first one is the default).
    pub fn with_presets() -> Self {
        let themes = [PRESET_AURORA, PRESET_OLED, PRESET_CEL, PRESET_RETRO]
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

    /// The theme to show while app `id` is the active view.
    pub fn for_app(&self, id: &str) -> Option<&ResolvedTheme> {
        self.apps.get(id)
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
        let key = key.trim().to_ascii_lowercase();
        if key == "next" {
            self.current = (self.current + 1) % self.themes.len();
            return Ok(self.current());
        }
        let exact = self
            .themes
            .iter()
            .position(|t| t.id.to_ascii_lowercase() == key);
        let idx = exact.or_else(|| {
            let hits: Vec<_> = self
                .themes
                .iter()
                .enumerate()
                .filter(|(_, t)| t.id.to_ascii_lowercase().starts_with(&key))
                .map(|(i, _)| i)
                .collect();
            (hits.len() == 1).then(|| hits[0])
        });
        match idx {
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
                "retro-terminal"
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
        assert_eq!(m.select("next").unwrap().id, "aurora-slate-dark");
        assert!(m.select("nope").is_err());
        // "c" is only a prefix of one theme id; "r" likewise; "m" likewise
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
        assert!(m.for_app("app-notes").is_none());
        assert_eq!(
            m.for_app("app-terminal").unwrap().id,
            "app-terminal-phosphor"
        );
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
