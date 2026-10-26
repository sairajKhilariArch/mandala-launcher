//! Slint UI glue: generated `MainWindow` plus helpers that map
//! `mandala-core` state/theme types onto Slint properties.

slint::include_modules!();

use mandala_core::slots::{self, Panel, PanelSlot};
use mandala_core::state::AppState;
use mandala_core::theme::{ResolvedTheme, Rgba};
use slint::{Color, ModelRc, SharedString, VecModel};

fn color(c: Rgba) -> Color {
    Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

fn slot_items(composed: &[PanelSlot]) -> Vec<SlotItem> {
    composed
        .iter()
        .map(|s| SlotItem {
            id: s.id.as_str().into(),
            label: s.label.as_str().into(),
            icon: slots::glyph_for(&s.label).as_str().into(),
            placeholder: s.placeholder,
        })
        .collect()
}

/// Fill the three panel models from the composed holder slots. Call once at
/// boot (before the first scan the panels are placeholders) and again after
/// `installed` changes.
pub fn populate_slots(ui: &MainWindow, installed: &[String]) {
    ui.set_panel_a(ModelRc::new(VecModel::from(slot_items(
        &slots::compose_panel(Panel::A, installed),
    ))));
    ui.set_panel_b(ModelRc::new(VecModel::from(slot_items(
        &slots::compose_panel(Panel::B, installed),
    ))));
    ui.set_panel_c(ModelRc::new(VecModel::from(slot_items(
        &slots::compose_panel(Panel::C, installed),
    ))));
}

/// Rows of the All Apps overlay: the Store entry first, then every installed
/// module sorted by label (known ids get matrix labels, unknown ids are
/// humanized by the core).
fn app_rows(installed: &[String]) -> Vec<SlotItem> {
    let mut rows: Vec<SlotItem> = vec![SlotItem {
        id: "store".into(),
        label: "Store".into(),
        icon: "S".into(),
        placeholder: false,
    }];
    let mut mods: Vec<(String, String)> = installed
        .iter()
        .map(|id| (slots::label_for(id), id.clone()))
        .collect();
    mods.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    rows.extend(mods.into_iter().map(|(label, id)| SlotItem {
        icon: slots::glyph_for(&label).as_str().into(),
        id: id.as_str().into(),
        label: label.as_str().into(),
        placeholder: false,
    }));
    rows
}

/// Push the reducer state into the window. Must run on the UI thread.
pub fn render(ui: &MainWindow, st: &AppState) {
    ui.set_canvas_text(SharedString::from(st.canvas_text.as_str()));
    ui.set_monitor_text(SharedString::from(st.monitor_text.as_str()));
    ui.set_left_open(st.left_open);
    ui.set_right_open(st.right_open);
    ui.set_monitor_open(st.monitor_open);
    ui.set_store_open(st.store_open);
    // A module is active: every panel vanishes until the home gesture.
    ui.set_app_open(st.active_module.is_some());
    ui.set_apps_open(st.apps_open);
    ui.set_store_status(SharedString::from(st.store_status.as_str()));
    let rows: Vec<StoreItem> = st
        .store_rows
        .iter()
        .map(|r| StoreItem {
            id: r.id.as_str().into(),
            label: r.label.as_str().into(),
            description: r.description.as_str().into(),
            version: r.version.as_str().into(),
            status: r.status.label().into(),
            actionable: r.status.actionable(),
        })
        .collect();
    ui.set_store_rows(ModelRc::new(VecModel::from(rows)));
    if st.apps_open {
        ui.set_app_rows(ModelRc::new(VecModel::from(app_rows(&st.installed))));
    }
    // Panels are app holders: recompose when the installed set changed (boot
    // scan, store install). Cheap (15 composed slots).
    populate_slots(ui, &st.installed);
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
