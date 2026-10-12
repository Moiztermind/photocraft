//! The Print dialog's Preset row: pick a saved preset (printer, driver settings, paper, colour
//! handling, profile, intent, mirror…), save the current settings under a name, delete one.
//! Storage and the commands are `photocraft_engine::print_preset_cmds`.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The saved preset names, sorted.
pub fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session
        .prefs()
        .dialogs
        .get(photocraft_engine::print_preset_cmds::PREF_KEY)
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// Loads preset `name` into the Print dialog's fields. On Windows the paper is then read back
/// from the preset's driver settings, and a printer that no longer exists is reported.
pub fn apply(app: &PhotocraftApp, f: &mut Map<String, Value>, name: &str) {
    let Some(settings) = photocraft_engine::print_preset_cmds::preset(&app.session, name) else {
        f.insert("__presetNote".into(), json!(crate::i18n::fmt(tl!("No print preset named \"{name}\""), &[("name", name)])));
        return;
    };
    f.extend(settings);
    f.insert("__preset".into(), json!(name));
    f.remove("__presetNote");
    if let Some(printers) = f.get("__printers").and_then(Value::as_array) {
        let printer = f.get("printer").and_then(Value::as_str).unwrap_or_default().to_string();
        if !printers.iter().any(|p| p.as_str() == Some(printer.as_str())) {
            let msg = crate::i18n::fmt(tl!("The preset's printer \"{printer}\" isn't installed: choose a printer and check Print Settings…"), &[("printer", &printer)]);
            f.insert("__presetNote".into(), json!(msg));
            f.remove("printerSettings");
        }
        crate::file_ui::driver_page(f);
    }
}

/// The Preset row, drawn at the top of the Print dialog's settings column.
pub fn row(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let names = names(app);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Preset")).color(t.text_dim));
        let mut cur = f.get("__preset").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Custom"))];
        opts.extend(names.iter().map(|n| (n.clone(), n.as_str())));
        if crate::widgets::dropdown(ui, "print-preset", &mut cur, &opts, 180.0) {
            if cur.is_empty() {
                f.remove("__preset");
            } else {
                apply(app, f, &cur);
            }
        }
        let selected = f.get("__preset").and_then(Value::as_str).map(str::to_string);
        if let Some(name) = selected
            && ui.button(tl!("Delete")).on_hover_text(tl!("Delete this print preset")).clicked()
        {
            match app.run("file.print.deletePreset", json!({"name": name})) {
                Ok(_) => {
                    f.remove("__preset");
                }
                Err(e) => {
                    f.insert("__presetNote".into(), json!(e));
                }
            }
        }
    });
    ui.horizontal(|ui| {
        let mut name = f.get("__presetName").and_then(Value::as_str).unwrap_or_default().to_string();
        if ui.add(egui::TextEdit::singleline(&mut name).hint_text(tl!("New preset name (e.g. Mug 11oz)")).desired_width(200.0)).changed() {
            f.insert("__presetName".into(), json!(name));
        }
        let trimmed = name.trim().to_string();
        if ui.add_enabled(!trimmed.is_empty(), egui::Button::new(tl!("Save Preset"))).clicked() {
            let settings: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
            match app.run("file.print.savePreset", json!({"name": trimmed, "settings": settings})) {
                Ok(_) => {
                    f.insert("__preset".into(), json!(trimmed));
                    f.remove("__presetName");
                    f.remove("__presetNote");
                }
                Err(e) => {
                    f.insert("__presetNote".into(), json!(e));
                }
            }
        }
    });
    if let Some(note) = f.get("__presetNote").and_then(Value::as_str) {
        ui.label(egui::RichText::new(note).color(t.warning).size(11.0));
    }
}

/// File › Print… opened with preset `name` loaded (the Sublimation panel's Print button).
pub fn open_print_with(app: &mut PhotocraftApp, name: &str) -> u64 {
    let id = crate::file_ui::open_print(app);
    let mut fields = app.ui.dialog_mut(id).map(|d| std::mem::take(&mut d.fields)).unwrap_or_default();
    apply(app, &mut fields, name);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields = fields;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_print_with_a_preset_loads_its_settings() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        app.run(
            "file.print.savePreset",
            json!({"name": "Slate 20x15", "settings": {"emulsionDown": true, "colorHandling": "photocraftManages", "intent": "perceptual", "bpc": false}}),
        )
        .unwrap();
        assert_eq!(names(&app), vec!["Slate 20x15".to_string()]);
        let id = open_print_with(&mut app, "Slate 20x15");
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(f["emulsionDown"], json!(true));
        assert_eq!(f["intent"], json!("perceptual"));
        assert_eq!(f["bpc"], json!(false));
        assert_eq!(f["__preset"], json!("Slate 20x15"));
        // A missing preset leaves the dialog as it was and says so.
        let id = open_print_with(&mut app, "Nope");
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert!(f.get("__preset").is_none());
        assert!(f["__presetNote"].as_str().unwrap().contains("Nope"));
    }
}
