//! Print presets: named Print settings (printer, driver settings, paper, colour handling,
//! profile, intent, mirror, position) kept in the preferences, so one choice sets up a whole
//! product (a mug, a T-shirt, a slate) and nothing silently falls back to the defaults.

use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Where the presets live in `Preferences::dialogs`: `{name: settings}`.
pub const PREF_KEY: &str = "file.print.presets";

/// The Print params a preset keeps. Copies, the output path and `send` are per job.
pub const PRESET_KEYS: [&str; 19] = [
    "printer",
    "printerSettings",
    "paper",
    "orientation",
    "colorHandling",
    "printerProfile",
    "intent",
    "bpc",
    "emulsionDown",
    "center",
    "top",
    "left",
    "scale",
    "scaleToFit",
    "cornerCropMarks",
    "centerCropMarks",
    "registrationMarks",
    "description",
    "labels",
];

/// The longest preset name accepted.
const MAX_NAME: usize = 80;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn stored(s: &Session) -> Map<String, Value> {
    s.prefs().dialogs.get(PREF_KEY).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Only the keys a preset keeps.
pub fn preset_settings(p: &Map<String, Value>) -> Map<String, Value> {
    PRESET_KEYS.iter().filter_map(|k| p.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect()
}

/// A preset's settings by name.
pub fn preset(s: &Session, name: &str) -> Option<Map<String, Value>> {
    stored(s).get(name).and_then(Value::as_object).cloned()
}

/// `p` on top of the preset it names (`"preset"`): params given explicitly win.
pub fn resolve(s: &Session, p: &Value, cmd: &str) -> Result<Value> {
    let Some(name) = p.get("preset") else { return Ok(p.clone()) };
    let name = name.as_str().ok_or_else(|| bad(cmd, "\"preset\" must be a preset name"))?;
    let mut out = preset(s, name).ok_or_else(|| bad(cmd, format!("no print preset named \"{name}\"")))?;
    if let Some(o) = p.as_object() {
        for (k, v) in o {
            if k != "preset" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(Value::Object(out))
}

fn name_param(cmd: &str, p: &Value) -> Result<String> {
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Err(bad(cmd, "missing \"name\""));
    }
    if name.chars().count() > MAX_NAME {
        return Err(bad(cmd, format!("a preset name has at most {MAX_NAME} characters")));
    }
    Ok(name.to_string())
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let presets: Vec<Value> = stored(s).into_iter().map(|(name, settings)| json!({"name": name, "settings": settings})).collect();
    Ok(json!({ "presets": presets }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.print.savePreset";
    let name = name_param(cmd, p)?;
    let settings = p.get("settings").and_then(Value::as_object).ok_or_else(|| bad(cmd, "missing \"settings\" (the Print params to keep)"))?;
    let kept = preset_settings(settings);
    if kept.is_empty() {
        return Err(bad(cmd, "\"settings\" has none of the Print params a preset keeps"));
    }
    s.prefs.edit(|prefs| {
        let entry = prefs.dialogs.entry(PREF_KEY.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        if let Some(o) = entry.as_object_mut() {
            o.insert(name.clone(), Value::Object(kept.clone()));
        }
    });
    Ok(json!({ "name": name, "settings": kept }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.print.deletePreset";
    let name = name_param(cmd, p)?;
    if preset(s, &name).is_none() {
        return Err(bad(cmd, format!("no print preset named \"{name}\"")));
    }
    s.prefs.edit(|prefs| {
        if let Some(o) = prefs.dialogs.get_mut(PREF_KEY).and_then(Value::as_object_mut) {
            o.remove(&name);
        }
    });
    Ok(json!({ "deleted": name }))
}

pub fn specs() -> Vec<CommandSpec> {
    let spec = |id, label, params, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: |_| Ok(()), journal: false, run };
    vec![
        spec("file.print.presets", "Print Presets", r##"{} → {presets: [{name, settings}]}"##, list),
        spec(
            "file.print.savePreset",
            "Save Print Preset",
            r##"{"name":str,"settings":{Print params}} → {name, settings} (keeps printer, printerSettings, paper, orientation, colour handling, profile, intent, bpc, emulsionDown, position, scale and marks; replaces a preset of the same name)"##,
            save,
        ),
        spec("file.print.deletePreset", "Delete Print Preset", r##"{"name":str} → {deleted}"##, delete),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_resolve_and_delete() {
        let mut s = Session::new();
        let settings = json!({"printer": "Sublimation - EPSON ET-18100 Series", "printerSettings": "00ff", "emulsionDown": true,
            "colorHandling": "photocraftManages", "printerProfile": "C:/x.icm", "copies": 3, "output": "C:/a.pdf", "send": true});
        s.execute("file.print.savePreset", json!({"name": " Mug 11oz ", "settings": settings})).unwrap();
        let l = s.execute("file.print.presets", json!({})).unwrap();
        assert_eq!(l["presets"][0]["name"], "Mug 11oz");
        let kept = &l["presets"][0]["settings"];
        assert_eq!(kept["emulsionDown"], true);
        assert!(kept.get("copies").is_none() && kept.get("output").is_none() && kept.get("send").is_none(), "per-job params are not kept");
        // Explicit params win over the preset.
        let r = resolve(&s, &json!({"preset": "Mug 11oz", "emulsionDown": false, "copies": 2}), "file.print").unwrap();
        assert_eq!((r["emulsionDown"].clone(), r["copies"].clone(), r["printerSettings"].clone()), (json!(false), json!(2), json!("00ff")));
        assert!(r.get("preset").is_none());
        // Saving the same name replaces it.
        s.execute("file.print.savePreset", json!({"name": "Mug 11oz", "settings": {"paper": "a4"}})).unwrap();
        assert_eq!(preset(&s, "Mug 11oz").unwrap(), json!({"paper": "a4"}).as_object().unwrap().clone());
        s.execute("file.print.deletePreset", json!({"name": "Mug 11oz"})).unwrap();
        assert!(s.execute("file.print.presets", json!({})).unwrap()["presets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = Session::new();
        for p in [json!({}), json!({"name": ""}), json!({"name": 5, "settings": {}}), json!({"name": "x"}), json!({"name": "x", "settings": {"copies": 1}})] {
            assert!(s.execute("file.print.savePreset", p).is_err());
        }
        assert!(s.execute("file.print.savePreset", json!({"name": "x".repeat(81), "settings": {"paper": "a4"}})).is_err());
        assert!(s.execute("file.print.deletePreset", json!({"name": "missing"})).is_err());
        assert!(resolve(&s, &json!({"preset": "missing"}), "file.print").is_err());
        assert!(resolve(&s, &json!({"preset": 3}), "file.print").is_err());
        // A corrupt stored value doesn't break saving.
        s.prefs.edit(|p| p.dialogs.insert(PREF_KEY.into(), json!("junk")));
        assert!(s.execute("file.print.presets", json!({})).unwrap()["presets"].as_array().unwrap().is_empty());
        s.execute("file.print.savePreset", json!({"name": "ok", "settings": {"paper": "a4"}})).unwrap();
        assert!(preset(&s, "ok").is_some());
    }
}
