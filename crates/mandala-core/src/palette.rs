//! pywal-style palette derivation: wallpaper image -> theme manifest.
//!
//! The rice look takes its accent from the wallpaper (the hyprdots
//! tradition): the dominant saturated colour of the image becomes the accent
//! and CTA fill, a darkened mix becomes the canvas and the translucent glass
//! surfaces, and text stays light on that always-dark base. The output is an
//! ordinary [`ThemeManifest`], so it validates through `resolve()` exactly
//! like a hand-written theme — a broken derivation can never load.

use crate::error::{MandalaError, Result};
use crate::theme::{ThemeManifest, ThemeTokens};
use std::collections::HashMap;
use std::path::Path;

/// Theme id of the derived manifest (`themes/wallpaper-derived.json`).
pub const DERIVED_ID: &str = "wallpaper-derived";

/// Histogram resolution: 4 bits per channel => 4096 buckets.
const BUCKET_BITS: u32 = 4;

/// Read `path` and derive the theme manifest for its dominant colour.
///
/// The image is thumbnailed to 64px first: a histogram does not need the
/// full photo and a 4K wallpaper must not stall the effect thread.
pub fn derive_manifest(path: &Path) -> Result<ThemeManifest> {
    let img = image::open(path).map_err(|e| MandalaError::Theme(format!("wallpaper: {e}")))?;
    let thumb = img.thumbnail(64, 64).into_rgb8();
    let accent = dominant(thumb.pixels().map(|p| p.0));
    let manifest = manifest_from(accent);
    // Validate now so a future edit that produces invalid tokens fails the
    // command instead of writing an unloadable file.
    manifest.resolve()?;
    Ok(manifest)
}

/// Most frequent bucket, preferring saturated colours so grey skies and
/// concrete do not out-vote the colour the wallpaper is actually about.
/// A fully grey image falls back to the plain most-frequent bucket.
fn dominant<'a>(pixels: impl Iterator<Item = [u8; 3]>) -> [u8; 3] {
    let mut buckets: HashMap<u16, (u32, u32, u32, u32)> = HashMap::new();
    for [r, g, b] in pixels {
        let key = ((r >> BUCKET_BITS) as u16) << 8
            | ((g >> BUCKET_BITS) as u16) << 4
            | (b >> BUCKET_BITS) as u16;
        let e = buckets.entry(key).or_insert((0, 0, 0, 0));
        e.0 += 1;
        e.1 += r as u32;
        e.2 += g as u32;
        e.3 += b as u32;
    }
    let mut best: Option<(u32, [u8; 3])> = None;
    // First pass: saturated buckets only (min_sat 0.20).
    for (count, sum_r, sum_g, sum_b) in buckets.values().copied() {
        let avg = [
            (sum_r / count) as u8,
            (sum_g / count) as u8,
            (sum_b / count) as u8,
        ];
        consider(&mut best, count, avg, 0.20);
    }
    if best.is_none() {
        // Second pass: everything (flat/grey wallpapers).
        for (count, sum_r, sum_g, sum_b) in buckets.values().copied() {
            let avg = [
                (sum_r / count) as u8,
                (sum_g / count) as u8,
                (sum_b / count) as u8,
            ];
            consider(&mut best, count, avg, 0.0);
        }
    }
    best.map(|(_, c)| c).unwrap_or([124, 131, 255])
}

/// Keep `best` as the highest-count bucket meeting `min_sat`.
fn consider(best: &mut Option<(u32, [u8; 3])>, count: u32, c: [u8; 3], min_sat: f32) {
    if saturation(c) >= min_sat && best.map_or(true, |(n, _)| count > n) {
        *best = Some((count, c));
    }
}

/// HSV-style saturation of an RGB triple (0.0 grey .. 1.0 vivid).
fn saturation(c: [u8; 3]) -> f32 {
    let (max, min) = (
        *c.iter().max().unwrap() as f32,
        *c.iter().min().unwrap() as f32,
    );
    if max == 0.0 {
        0.0
    } else {
        (max - min) / max
    }
}

/// Build the manifest for one accent colour. The canvas is always forced
/// near-black so light-on-dark text contrast holds for any wallpaper.
fn manifest_from(accent: [u8; 3]) -> ThemeManifest {
    let hex = |c: [u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
    // `t` = weight of the first colour.
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| -> [u8; 3] {
        let one = |i: usize| (a[i] as f32 * t + b[i] as f32 * (1.0 - t)).round() as u8;
        [one(0), one(1), one(2)]
    };
    // Dark blue-black base keeps the derived canvas in the rice palette
    // range no matter what the wallpaper is.
    let canvas = mix(accent, [8, 9, 16], 0.08);
    // Glass: a hint of the accent over near-black, 80% opaque.
    let surface = mix(accent, [16, 18, 30], 0.12);
    // Secondary text: grey tinted toward the accent.
    let secondary = mix(accent, [154, 160, 184], 0.25);
    let text = [0xEC, 0xEE, 0xF6];
    // The CTA label is whichever of dark/light reads better on the accent.
    let dark = [10u8, 11, 16];
    let light = [255u8, 255, 255];
    let on_cta = if contrast(dark, accent) >= contrast(light, accent) {
        dark
    } else {
        light
    };
    ThemeManifest {
        theme_id: DERIVED_ID.into(),
        theme_name: "Wallpaper Derived".into(),
        version: "1.0.0".into(),
        author: "wallpaper".into(),
        tokens: ThemeTokens {
            canvas_bg: hex(canvas),
            surface_primary: format!("{}CC", hex(surface)),
            surface_border: format!("{}55", hex(accent)),
            text_primary: hex(text),
            text_secondary: hex(secondary),
            accent_focus: hex(accent),
            cta_accent: hex(accent),
            on_cta: Some(hex(on_cta)),
            border_width_px: 1.0,
            corner_radius_px: 10.0,
            font_family: "Inter".into(),
        },
    }
}

fn luminance(c: [u8; 3]) -> f64 {
    let lin = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG 2.1 contrast ratio (1..=21).
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Rgba;

    fn write_solid(dir: &Path, name: &str, rgb: [u8; 3]) -> std::path::PathBuf {
        let img = image::RgbImage::from_pixel(16, 16, image::Rgb(rgb));
        let p = dir.join(name);
        img.save(&p).expect("write test image");
        p
    }

    #[test]
    fn vivid_wallpaper_drives_accent_and_passes_contrast() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_solid(dir.path(), "wall.png", [0xFF, 0x6B, 0x6B]);
        let m = derive_manifest(&path).expect("derive");
        assert_eq!(m.tokens.accent_focus, "#FF6B6B");
        let t = m.resolve().expect("resolve");
        let canvas = t.canvas_bg;
        assert_eq!(canvas.a, 255, "canvas must stay opaque");
        // Body text stays readable on the forced-dark canvas and glass.
        let pair = |fg: Rgba, bg: Rgba| {
            let f = [fg.r, fg.g, fg.b];
            let b = [bg.r, bg.g, bg.b];
            contrast(f, b)
        };
        assert!(pair(t.text_primary, canvas) >= 4.5, "text on canvas");
        assert!(pair(t.text_secondary, canvas) >= 4.5, "secondary on canvas");
        assert!(pair(t.on_cta, t.cta_accent) >= 4.5, "CTA label");
        // Round-trips to JSON for themes/wallpaper-derived.json.
        assert!(serde_json::to_string_pretty(&m).is_ok());
    }

    #[test]
    fn grey_wallpaper_still_yields_a_valid_theme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_solid(dir.path(), "grey.png", [0x80, 0x80, 0x80]);
        let m = derive_manifest(&path).expect("derive");
        assert_eq!(m.theme_id, DERIVED_ID);
        assert!(m.resolve().is_ok());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(derive_manifest(Path::new("/nonexistent/wall.png")).is_err());
    }
}
