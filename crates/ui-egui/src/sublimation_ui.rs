//! The Sublimation workspace's floating panel: the steps of a sublimation transfer in one place,
//! each running an ordinary command (Canvas Size with an edge bleed, Flip Canvas Horizontal,
//! Convert to Profile, Proof Colors, Print).

use egui::{CornerRadius, Stroke};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

const WIDTH: f32 = 230.0;

/// The panel's remembered settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SublimationOptions {
    /// Bleed added on every side, in millimetres.
    pub bleed_mm: f32,
    /// `edge` (stretch the outer pixels) or `mirror`.
    pub bleed_mode: String,
}

impl Default for SublimationOptions {
    fn default() -> Self {
        Self { bleed_mm: 3.0, bleed_mode: "edge".into() }
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

pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.panels.sublimation {
        return;
    }
    let t = Tokens::get(ctx);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(10));
    let canvas = app.last_canvas_rect;
    let has_doc = app.session.active().is_some();
    let mut open = true;
    let mut run: Option<(&'static str, Value)> = None;
    egui::Window::new(tl!("Sublimation"))
        .id(egui::Id::new("sublimation-panel"))
        .title_bar(false)
        .resizable(false)
        .frame(frame)
        .default_pos(egui::pos2(canvas.left() + 16.0, canvas.top() + 24.0))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Sublimation")).font(crate::theme::semibold(12.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(6.0);
            widgets::hairline(ui);
            ui.add_space(6.0);
            let o = &mut app.ui.sublimation;
            widgets::section_label(ui, tl!("Bleed"));
            ui.horizontal(|ui| {
                widgets::value_field_prec(ui, &mut o.bleed_mm, 0.0..=50.0, "mm", 70.0, 1);
                let opts = [("edge".to_string(), tl!("Edge Pixels")), ("mirror".to_string(), tl!("Mirror"))];
                widgets::dropdown(ui, "subl-bleed-mode", &mut o.bleed_mode, &opts, 120.0);
            });
            ui.add_enabled_ui(has_doc && o.bleed_mm > 0.0, |ui| {
                if widgets::secondary_button(ui, tl!("Add Bleed"), WIDTH).on_hover_text(tl!("Grows the canvas on every side, continuing the image, with guides on the trim line")).clicked() {
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
                    ("file.print", tl!("Print…")),
                ] {
                    if widgets::secondary_button(ui, label, WIDTH).clicked() {
                        run = Some((id, json!({})));
                    }
                }
            });
        });
    if !open {
        app.ui.panels.sublimation = false;
    }
    if let Some((id, mut params)) = run {
        if params.is_null() {
            let dpi = app.session.active().map_or(300.0, |d| d.doc.resolution_dpi);
            params = bleed_params(app.ui.sublimation.bleed_mm, dpi, &app.ui.sublimation.bleed_mode);
        }
        if let Err(e) = crate::menus::invoke(app, ctx, id, params) {
            app.ui.status = e;
        }
    }
}

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
        assert!(app.ui.panels.sublimation);
        let p = bleed_params(app.ui.sublimation.bleed_mm, 300.0, "edge");
        app.run("image.canvasSize", p).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height), (170, 120));
        assert_eq!(d.guides.vertical, vec![35.0, 135.0]);
        app.ui.workspace = "Essentials".into();
        crate::menus::apply_workspace(&mut app);
        assert!(!app.ui.panels.sublimation);
    }
}
