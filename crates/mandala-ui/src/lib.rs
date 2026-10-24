//! Slint UI glue: generated `MainWindow` plus helpers that map
//! `mandala-core` state/theme types onto Slint properties.

slint::include_modules!();

use mandala_core::slots::{self, Panel};
use mandala_core::state::AppState;
use mandala_core::theme::{ResolvedTheme, Rgba};
use slint::{Color, ModelRc, SharedString, VecModel};

fn color(c: Rgba) -> Color {
    Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

fn slot_model(panel: Panel) -> ModelRc<SlotItem> {
    let items: Vec<SlotItem> = slots::by_panel(panel)
        .map(|s| SlotItem { id: s.id.into(), tag: s.tag().into(), label: s.label.into() })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Fill the three panel models from the static slot matrix. Call once.
pub fn populate_slots(ui: &MainWindow) {
    ui.set_panel_a(slot_model(Panel::A));
    ui.set_panel_b(slot_model(Panel::B));
    ui.set_panel_c(slot_model(Panel::C));
}

/// Push the reducer state into the window. Must run on the UI thread.
pub fn render(ui: &MainWindow, st: &AppState) {
    ui.set_canvas_text(SharedString::from(st.canvas_text.as_str()));
    ui.set_monitor_text(SharedString::from(st.monitor_text.as_str()));
    ui.set_left_open(st.left_open);
    ui.set_right_open(st.right_open);
    ui.set_monitor_open(st.monitor_open);
    // Only touch the input when the state diverged (e.g. cleared on submit);
    // rewriting identical text would disturb the cursor while typing.
    if ui.get_query().as_str() != st.query {
        ui.set_query(SharedString::from(st.query.as_str()));
    }
}

/// Inject a resolved theme into the `ThemePalette` global. UI thread only.
pub fn apply_theme(ui: &MainWindow, t: &ResolvedTheme) {
    let p = ui.global::<ThemePalette>();
    p.set_canvas_bg(color(t.canvas_bg));
    p.set_surface_primary(color(t.surface_primary));
    p.set_surface_border(color(t.surface_border));
    p.set_text_primary(color(t.text_primary));
    p.set_text_secondary(color(t.text_secondary));
    p.set_accent_focus(color(t.accent_focus));
    p.set_border_width(t.border_width_px);
    p.set_corner_radius(t.corner_radius_px);
    p.set_font_family(SharedString::from(t.font_family.as_str()));
}
