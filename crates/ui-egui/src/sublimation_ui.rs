//! The Sublimation workspace's panel (a tab beside Navigator, Histogram and Info): the steps of a sublimation transfer in one place,
//! each running an ordinary command (Canvas Size with an edge bleed, Flip Canvas Horizontal,
//! Convert to Profile, Proof Colors, Print).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// The panel's remembered settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SublimationOptions {
    /// Bleed added on every side, in millimetres.
    pub bleed_mm: f32,
    /// `edge` (stretch the outer pixels) or `mirror`.
    pub bleed_mode: String,
    /// The print preset Print… opens with ("" = the last Print settings).
    pub print_preset: String,
}

impl Default for SublimationOptions {
    fn default() -> Self {
        Self { bleed_mm: 3.0, bleed_mode: "edge".into(), print_preset: String::new() }
    }
}

/// Canvas Size params that add `mm` of bleed on every side of a `dpi` document.
pub fn bleed_params(mm: f32, dpi: f32, mode: &str) -> Value {
    let side = (f64::from(mm) / 25.4 * f64::from(dpi.max(1.0))).round().max(0.0);
    json!({
        "width": side * 2.0,
        "height": side * 2.0,
        "relative": true,
        "anchor": "center",
        "extensionColor": if mode == "mirror" { "mirror" } else { "edge" },
        "guides": true,
    })
}

/// The Sublimation tab (Navigator | Histogram | Info | Sublimation), shown by the Sublimation
/// workspace.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let has_doc = app.session.active().is_some();
    let presets = crate::print_presets_ui::names(app);
    let mut run: Option<(&'static str, Value)> = None;
    let w = ui.available_width().max(120.0);
    let o = &mut app.ui.sublimation;
    widgets::section_label(ui, tl!("Bleed"));
    ui.horizontal(|ui| {
        widgets::value_field_prec(ui, &mut o.bleed_mm, 0.0..=50.0, "mm", 70.0, 1);
        let opts = [("edge".to_string(), tl!("Edge Pixels")), ("mirror".to_string(), tl!("Mirror"))];
        widgets::dropdown(ui, "subl-bleed-mode", &mut o.bleed_mode, &opts, 120.0);
    });
    ui.add_enabled_ui(has_doc && o.bleed_mm > 0.0, |ui| {
        if widgets::secondary_button(ui, tl!("Add Bleed"), w).on_hover_text(tl!("Grows the canvas on every side, continuing the image, with guides on the trim line")).clicked() {
            run = Some(("image.canvasSize", Value::Null));
        }
    });
    ui.add_space(6.0);
    widgets::section_label(ui, tl!("Transfer"));
    ui.add_enabled_ui(has_doc, |ui| {
        for (id, label) in [
            ("image.imageRotation.flipCanvasHorizontal", tl!("Mirror Image")),
            ("edit.convertToProfile", tl!("Convert to Profile…")),
            ("view.proofColors", tl!("Proof Colors")),
        ] {
            if widgets::secondary_button(ui, label, w).clicked() {
                run = Some((id, json!({})));
            }
        }
    });
    ui.add_space(6.0);
    widgets::section_label(ui, tl!("Print"));
    let o = &mut app.ui.sublimation;
    if !o.print_preset.is_empty() && !presets.contains(&o.print_preset) {
        o.print_preset.clear();
    }
    let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Last used settings"))];
    opts.extend(presets.iter().map(|n| (n.clone(), n.as_str())));
    widgets::dropdown(ui, "subl-print-preset", &mut o.print_preset, &opts, w);
    if presets.is_empty() {
        ui.label(egui::RichText::new(tl!("Save presets from the Print dialog")).color(t.text_dim).size(11.0));
    }
    ui.add_enabled_ui(has_doc, |ui| {
        if widgets::secondary_button(ui, tl!("Print…"), w).clicked() {
            run = Some(("file.print", json!({})));
        }
    });
    let Some((id, mut params)) = run else { return };
    let preset = app.ui.sublimation.print_preset.clone();
    if id == "file.print" && !preset.is_empty() {
        crate::print_presets_ui::open_print_with(app, &preset);
        return;
    }
    if params.is_null() {
        let dpi = app.session.active().map_or(300.0, |d| d.doc.resolution_dpi);
        params = bleed_params(app.ui.sublimation.bleed_mm, dpi, &app.ui.sublimation.bleed_mode);
    }
    if let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params) {
        app.ui.status = e;
    }
}

/// The Sublimation tab's index in the Navigator group.
pub const TAB: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_mm_at_300_ppi_is_35_px_a_side() {
        let p = bleed_params(3.0, 300.0, "edge");
        assert_eq!(p["width"], json!(70.0));
        assert_eq!(p["height"], json!(70.0));
        assert_eq!(p["extensionColor"], json!("edge"));
        assert_eq!(bleed_params(3.0, 300.0, "mirror")["extensionColor"], json!("mirror"));
        assert_eq!(bleed_params(3.0, 300.0, "nonsense")["extensionColor"], json!("edge"));
        assert_eq!(bleed_params(-1.0, 0.0, "edge")["width"], json!(0.0));
    }

    #[test]
    fn the_sublimation_workspace_opens_the_panel_and_add_bleed_grows_the_canvas() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "resolution": 300})).unwrap();
        app.ui.workspace = "Sublimation".into();
        crate::menus::apply_workspace(&mut app);
        assert_eq!(app.ui.dock_tabs.navigator, TAB, "the workspace opens on the Sublimation tab");
        assert!(app.ui.panels.navigator);
        assert!(app.ui.dock.visible_tabs(crate::dock::Group::Navigator, false).iter().any(|(i, _)| *i == TAB));
        let p = bleed_params(app.ui.sublimation.bleed_mm, 300.0, "edge");
        app.run("image.canvasSize", p).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height), (170, 120));
        assert_eq!(d.guides.vertical, vec![35.0, 135.0]);
        app.ui.workspace = "Essentials".into();
        crate::menus::apply_workspace(&mut app);
        app.ui.workspace = "Photography".into();
        crate::menus::apply_workspace(&mut app);
        assert!(!app.ui.dock.visible_tabs(crate::dock::Group::Navigator, false).iter().any(|(i, _)| *i == TAB), "hidden elsewhere");
    }
}
