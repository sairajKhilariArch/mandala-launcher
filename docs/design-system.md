# Mandala design system

The one page that says how the shell looks and behaves. Source of truth for
sizes is the `Scale` global in `crates/mandala-ui/ui/theme_tokens.slint`; the
source of truth for colour is the theme manifest (below) feeding the
`ThemePalette` global. Nothing else declares a font size, spacing value or
hex colour.

Derived from a ui-ux-pro-max `--design-system` run for a native
launcher/store/module-manager product: flat design, Inter, no gradients or
shadows, WCAG AA contrast, visible press feedback, 4/8 spacing rhythm. Its
web-marketing recommendations (hero sections, landing CTAs, brand-green
palette) were dropped as irrelevant to a native Slint shell.

## Colour roles

Colour comes from JSON manifests (`themes/*.json`, `themes/apps/*.json`,
user themes in `$MANDALA_HOME/themes/`), applied at runtime through
`ThemePalette` — the manifest schema is unchanged by this document except
one added optional token:

| Token | Role | Renders as |
|---|---|---|
| `canvas_bg` | app background | window, rails, overlays |
| `surface_primary` | raised surface | input, tiles, store rows, drawers |
| `surface_border` | separation + pressed/hover feedback | borders, all press states |
| `text_primary` | body text | canvas, rows, input |
| `text_secondary` | secondary text | placeholder, meta, empty states |
| `accent_focus` | focus ring, per-app accents | input focus border, monograms |
| `cta_accent` | primary action fill | EXIT / INSTALL buttons |
| `on_cta` *(new, optional)* | label on a CTA fill | primary button text; defaults to `canvas_bg` when absent |

`on_cta` exists because a light theme's canvas colour on an orange CTA fails
contrast (cel-shaded scored 2.67:1). Themes override only when needed;
`cel-shaded` sets `#111111`, `app-vault` sets `#FFFFFF`. Older manifests
without the token keep their exact previous rendering.

A scrim (`overlay-scrim`) is deliberately **not** themed: dimming behind
drawers/overlays stays dark over any theme's content.

## Typography

One family, themed (`font_family` token, default Inter). One exception: the
process monitor uses `Scale.font-mono` because it shows machine output.

| Token | Size | Used by |
|---|---|---|
| `Scale.font-label` | 10px | slot labels under monograms |
| `Scale.font-caption` | 11px | overlay headings, store meta |
| `Scale.font-small` | 12px | buttons, status lines, monitor |
| `Scale.font-icon` | 14px | every monogram glyph (one size everywhere) |
| `Scale.font-body` | 15px | canvas, list rows, placeholder |
| `Scale.font-input` | 16px | command input |

## Spacing

4/8 rhythm: `space-half` 2 (micro-gap inside tiles), `space-1` 4, `space-2` 8,
`space-3` 12, `space-4` 16, `space-5` 20, `space-6` 24, `space-8` 32,
`space-10` 40. Edge gutters that clear system chrome: `gutter-top` 48
(overlays/drawers), `gutter-rail-top` 96 (side rails below the status bar).

Fixed geometry that is layout, not scale: rails 76px, drawers 280px
(320px expanded), dock band 64px, rows 52px (store 64px), rail tiles 56px,
window default 412×915.

## Component states

* **Press:** every control darkens to `surface_border` within the frame —
  no layout shift, instant.
* **Hover (desktop):** non-primary buttons and rows gain the same
  `surface_border` fill; primary buttons keep the CTA fill.
* **Focus:** the command input shows `accent_focus` as its border.
* **Disabled/placeholder:** empty slots render at reduced opacity and open
  the Store instead of firing.
* **Timing:** 180ms ease-out for motion with direction (workspace glide on
  open/close, drawer/rail slides), 150ms ease-out for opacity fades
  (overlay scrim + panels). Transform/opacity only — geometry animates via
  layout padding and `x`, never width/height.
* **Keyboard:** Tab / Shift+Tab walk the focus chain — command bar first,
  then every visible control (Slint's window-level focus loop; the root
  scopes opt out so no invisible stops exist). Enter or Space activates the
  focused control; the ring is the theme accent at `Scale.ring-width` and
  appears only for Tab focus, instantly. Controls hidden behind a modal
  overlay or an open drawer are `enabled: false`, so the ring never lands
  under something opaque. Escape (and Android back) fires `back()` from the
  default focus state — the two root scopes compare against `"\u{1b}"`; the
  old `"\\e"` literal never matched, so back-via-Escape did nothing before
  this phase (caught by `tests/tab_focus.rs`).
  `shortcut:` Escape while a *control* has focus bubbles to the window and
  is dropped there; close overlays with Tab + Enter on the CLOSE button.
* **Shortcuts (rice):** Ctrl+Space focuses the command bar (the rofi /
  SUPER+D launcher role), Ctrl+T opens the Terminal. Both are chords, so
  they bubble past the focused input and any focused button without
  "clicking" it (buttons reject modifier chords; plain Space still
  activates). Covered by `tests/shortcuts.rs`.
  `shortcut:` Ctrl+Return is *not* wired: Slint's single-line TextInput
  consumes Return (fires `accepted`) before window scopes see it, so it
  could only work outside the command bar — Ctrl+T works from every focus
  state instead.

Touch targets: every control is ≥44px in its smallest dimension
(`Scale.touch-min`); inputs and buttons are 48px (`Scale.control-height`),
rail tiles 56px, rows 52px.

## Rice layer

The rice (hyprdots-style) layer sits on top of the themed shell and is
driven by one command, `wallpaper <path>`:

* **Derived palette.** `crates/mandala-core/src/palette.rs` thumbs the
  image to 64px, builds a 4096-bucket histogram, picks the most frequent
  *saturated* bucket (flat/grey wallpapers fall back to plain frequency)
  and turns it into an ordinary manifest (`themes/wallpaper-derived.json`):
  accent and CTA fill = the wallpaper's colour, canvas forced near-black,
  glass surfaces a tint of the accent, CTA label chosen by whichever of
  dark/light scores better in WCAG contrast. The manifest resolves through
  `ThemeManifest::resolve()` before it is written, so a bad derivation
  fails the command instead of shipping an unloadable theme; the
  `palette.rs` tests assert ≥4.5:1 for body text, secondary text and the
  CTA label on the derived theme.
* **Wallpaper surface.** The shell draws the image itself (`Image`,
  `image-fit: cover`, behind everything, `has-wallpaper`-gated) so the
  glass panels have something to sit on. The path persists in
  `wallpaper.path`; palette and image both restore on the next boot. On
  Android the same command also sets the *system* wallpaper via JNI
  (`shell/android.rs::set_wallpaper`).
* **Status bar.** A waybar-style row of `BarPill`s: active theme, install
  count, live clock (`%a %H:%M`, one `slint::Timer` tick per second).
  Status only — no pointer targets — themed through `Scale` and
  `ThemePalette` like everything else.

## Accessibility

* Body text ≥ 4.5:1 where it renders — enforced by
  `builtin_themes_meet_body_contrast` in `crates/mandala-core/src/theme.rs`,
  which checks all 21 builtin manifests (text on canvas, text on surface,
  CTA label on CTA fill) plus `contrast_ratio_math` for the WCAG formula.
  A theme edit that breaks AA fails `cargo test`.
* Colour is never the only signal: placeholders are dimmed *and* show `+`,
  store statuses are words, pressed state changes fill.
* Icons are monogram text glyphs (font-controlled, scalable, no emoji, no
  raster assets).

## Responsive behavior

Breakpoints in the `Responsive` global — compact < 600, medium < 840,
expanded ≥ 840 (phone / foldable unfolded / tablet+desktop):

| Class | Behavior |
|---|---|
| compact | 76px rails always visible left/right; drawers slide over them from lower-third edge swipes; center workspace = command bar + canvas + dock |
| medium | same rails, wider workspace; drawers overlay |
| expanded | both drawers dock permanently (320px), rails hidden, center column capped between them; overlays stay full-screen |

Overlays (All Apps, Store, monitor) are full-bleed at every width and scale
by wrapping rows, not by shrinking type. Verified statically against the
`rails-visible` / `expanded` logic in `main_window.slint`; the shell has no
runtime layout test harness.

## Adding a token

1. Declare it in `Scale` (geometry/type) or `ThemePalette` (colour) in
   `theme_tokens.slint` — the only file allowed to hold literals.
2. For colour from themes: add an optional field to `ThemeTokens` with a
   `#[serde(default)]` fallback in `theme.rs`, map it in `apply_theme`.
3. Use it everywhere the value appears; grep for the old literal.
4. If it carries text meaning, add a contrast pair to
   `builtin_themes_meet_body_contrast`.
