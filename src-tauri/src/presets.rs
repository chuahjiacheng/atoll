//! Settings presets: named snapshots of your settings you can switch between.
//! Kept on this PC in `%APPDATA%\Crest\presets.json`, next to settings.json.
//!
//! A preset holds every preference except things tied to this PC or to an
//! account (sign-ins, start with Windows, first-run seeds): those always stay
//! as they are when you switch.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::settings::Settings;
use crate::AppState;

pub const MAX_PRESETS: usize = 50;
pub const MAX_NAME: usize = 40;

/// Settings a preset never carries (dotted, camelCase as in the JSON).
const KEEP_LOCAL: &[&str] = &[
    "version",
    "general.startWithWindows",
    "spotify.connected",
    "spotify.clientId",
    "spotify.callbackPort",
    "calendar.clientId",
    "calendar.shown",
    "search.seeded",
    "ask.seeded",
    "ask.seedVersion",
];

/// One read-modify-write of presets.json at a time.
pub(crate) static FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    /// ms since epoch
    pub saved_at: u64,
    pub settings: Value,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PresetFile {
    /// The preset last applied or saved, if it still exists.
    pub active: Option<String>,
    pub presets: Vec<Preset>,
    /// Automatic switches (src-tauri/src/routines.rs).
    pub routines: Vec<crate::routines::Routine>,
    pub routine_options: crate::routines::RoutineOptions,
    pub routine_state: crate::routines::RoutineState,
}

/// What the dashboard sees (no settings payload).
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub saved_at: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PresetList {
    pub active: Option<String>,
    /// The current settings differ from the active preset.
    pub modified: bool,
    pub presets: Vec<PresetInfo>,
}

pub fn path() -> PathBuf {
    crate::settings::path().with_file_name("presets.json")
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub(crate) fn new_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------- file ----------

/// A missing file is an empty list; a broken one is kept aside as
/// `presets.json.broken` so nothing is silently lost.
pub fn load_from(p: &Path) -> PresetFile {
    let Ok(text) = std::fs::read_to_string(p) else { return PresetFile::default() };
    match serde_json::from_str::<PresetFile>(&text) {
        Ok(mut f) => {
            f.presets.retain(|x| !x.id.is_empty() && x.settings.is_object());
            if f.active.as_ref().is_some_and(|a| !f.presets.iter().any(|x| &x.id == a)) {
                f.active = None;
            }
            crate::routines::clean(&mut f);
            f
        }
        Err(e) => {
            eprintln!("[atoll] presets.json unreadable ({e}); starting empty");
            let _ = std::fs::rename(p, p.with_extension("json.broken"));
            PresetFile::default()
        }
    }
}

pub fn save_to(p: &Path, f: &PresetFile) -> Result<(), String> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Could not save presets: {e}"))?;
    std::fs::rename(&tmp, p).map_err(|e| format!("Could not save presets: {e}"))
}

// ---------- pure logic (tested below) ----------

fn get_path<'a>(v: &'a Value, dotted: &str) -> Option<&'a Value> {
    dotted.split('.').try_fold(v, |cur, k| cur.get(k))
}

fn set_path(v: &mut Value, dotted: &str, value: Value) {
    let keys: Vec<&str> = dotted.split('.').collect();
    let mut cur = v;
    for k in &keys[..keys.len() - 1] {
        if !cur.get(*k).is_some_and(Value::is_object) {
            cur[*k] = Value::Object(Default::default());
        }
        cur = cur.get_mut(*k).unwrap();
    }
    cur[keys[keys.len() - 1]] = value;
}

fn remove_path(v: &mut Value, dotted: &str) {
    let (parent, last) = match dotted.rsplit_once('.') {
        Some((p, l)) => (dotted_mut(v, p), l),
        None => (Some(v), dotted),
    };
    if let Some(Value::Object(m)) = parent {
        m.remove(last);
    }
}

fn dotted_mut<'a>(v: &'a mut Value, dotted: &str) -> Option<&'a mut Value> {
    dotted.split('.').try_fold(v, |cur, k| cur.get_mut(k))
}

/// The part of `s` a preset stores.
pub fn snapshot(s: &Settings) -> Result<Value, String> {
    let mut v = serde_json::to_value(s).map_err(|e| e.to_string())?;
    for p in KEEP_LOCAL {
        remove_path(&mut v, p);
    }
    Ok(v)
}

/// `current` with the preset's preferences in place (PC/account bits kept).
/// Sections missing from an older preset fall back to defaults.
pub fn apply_to(current: &Settings, preset: &Value) -> Result<Settings, String> {
    let mut v = preset.clone();
    let cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    for p in KEEP_LOCAL {
        if let Some(x) = get_path(&cur, p) {
            set_path(&mut v, p, x.clone());
        }
    }
    Settings::default().merged(&v)
}

pub fn clean_name(name: &str, f: &PresetFile, except: Option<&str>) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the preset a name.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("Keep the name to {MAX_NAME} characters or fewer."));
    }
    if f.presets.iter().any(|p| Some(p.id.as_str()) != except && p.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("There is already a preset called \u{201c}{name}\u{201d}."));
    }
    Ok(name)
}

pub fn list_of(f: &PresetFile, current: &Settings) -> PresetList {
    let modified = f
        .active
        .as_ref()
        .and_then(|a| f.presets.iter().find(|p| &p.id == a))
        .map(|p| apply_to(current, &p.settings).map(|s| &s != current).unwrap_or(true))
        .unwrap_or(false);
    PresetList {
        active: f.active.clone(),
        modified,
        presets: f.presets.iter().map(|p| PresetInfo { id: p.id.clone(), name: p.name.clone(), saved_at: p.saved_at }).collect(),
    }
}

pub(crate) fn find<'a>(f: &'a mut PresetFile, id: &str) -> Result<&'a mut Preset, String> {
    f.presets.iter_mut().find(|p| p.id == id).ok_or_else(|| "That preset no longer exists.".to_string())
}

// ---------- commands ----------

pub(crate) fn current(app: &AppHandle) -> Settings {
    app.state::<AppState>().settings.lock().unwrap().clone()
}

/// Run `edit` on the stored presets and save them.
pub(crate) fn with_file<T>(edit: impl FnOnce(&mut PresetFile) -> Result<T, String>) -> Result<T, String> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = path();
    let mut f = load_from(&p);
    let out = edit(&mut f)?;
    save_to(&p, &f)?;
    Ok(out)
}

#[tauri::command]
pub fn list_presets(app: AppHandle) -> PresetList {
    let f = {
        let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load_from(&path())
    };
    list_of(&f, &current(&app))
}

/// Save the current settings as a new preset (it becomes the active one).
#[tauri::command]
pub fn save_preset(app: AppHandle, name: String) -> Result<PresetList, String> {
    let s = current(&app);
    let snap = snapshot(&s)?;
    let f = with_file(|f| {
        if f.presets.len() >= MAX_PRESETS {
            return Err(format!("You can keep up to {MAX_PRESETS} presets. Delete one first."));
        }
        let name = clean_name(&name, f, None)?;
        let id = new_id();
        f.presets.push(Preset { id: id.clone(), name, saved_at: now_ms(), settings: snap });
        f.active = Some(id);
        f.routine_state.last_switch = None;
        Ok(f.clone())
    })?;
    Ok(list_of(&f, &s))
}

/// Replace a preset's settings with the current ones.
#[tauri::command]
pub fn overwrite_preset(app: AppHandle, id: String) -> Result<PresetList, String> {
    let s = current(&app);
    let snap = snapshot(&s)?;
    let f = with_file(|f| {
        let p = find(f, &id)?;
        p.settings = snap;
        p.saved_at = now_ms();
        f.active = Some(id.clone());
        Ok(f.clone())
    })?;
    Ok(list_of(&f, &s))
}

/// Switch to a preset: its settings replace the current ones.
#[tauri::command]
pub fn apply_preset(app: AppHandle, id: String) -> Result<PresetList, String> {
    let s = current(&app);
    let (f, next) = with_file(|f| {
        let next = apply_to(&s, &find(f, &id)?.settings)?;
        f.active = Some(id.clone());
        f.routine_state.last_switch = None;
        Ok((f.clone(), next))
    })?;
    crate::commit_settings(&app, next.clone());
    Ok(list_of(&f, &next))
}

#[tauri::command]
pub fn rename_preset(app: AppHandle, id: String, name: String) -> Result<PresetList, String> {
    let f = with_file(|f| {
        let name = clean_name(&name, f, Some(&id))?;
        find(f, &id)?.name = name;
        Ok(f.clone())
    })?;
    Ok(list_of(&f, &current(&app)))
}

/// Delete a preset (and the routines that switch to it). Your current settings are not changed.
#[tauri::command]
pub fn delete_preset(app: AppHandle, id: String) -> Result<PresetList, String> {
    let f = with_file(|f| {
        f.presets.retain(|p| p.id != id);
        if f.active.as_deref() == Some(id.as_str()) {
            f.active = None;
        }
        crate::routines::clean(f);
        Ok(f.clone())
    })?;
    Ok(list_of(&f, &current(&app)))
}

/// Settings → Privacy → Clear all data.
pub fn clear_all() {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn custom() -> Settings {
        Settings::default()
            .merged(&json!({
                "appearance": { "opacity": 70, "theme": "light" },
                "modules": { "spotify": false },
                "focus": { "focusMin": 50 },
                "hotkeys": { "toggleIsland": "Ctrl+Alt+KeyQ" },
            }))
            .unwrap()
    }

    #[test]
    fn snapshot_leaves_out_pc_and_account_bits() {
        let mut s = custom();
        s.spotify.connected = true;
        s.general.start_with_windows = true;
        let v = snapshot(&s).unwrap();
        assert!(get_path(&v, "spotify.connected").is_none());
        assert!(get_path(&v, "general.startWithWindows").is_none());
        assert!(get_path(&v, "calendar.clientId").is_none());
        assert!(get_path(&v, "version").is_none());
        assert_eq!(get_path(&v, "appearance.opacity"), Some(&json!(70)));
        assert_eq!(get_path(&v, "general.showTray"), Some(&json!(true)));
    }

    #[test]
    fn applying_restores_preferences_and_keeps_local_bits() {
        let snap = snapshot(&custom()).unwrap();
        // Later the user changes things and links Spotify.
        let mut now = Settings::default();
        now.spotify.connected = true;
        now.spotify.client_id = "abc".into();
        now.general.start_with_windows = true;
        now.appearance.opacity = 95;

        let next = apply_to(&now, &snap).unwrap();
        assert_eq!(next.appearance.opacity, 70);
        assert_eq!(next.appearance.theme, "light");
        assert!(!next.modules.spotify);
        assert_eq!(next.focus.focus_min, 50);
        assert_eq!(next.hotkeys.toggle_island, "Ctrl+Alt+KeyQ");
        assert!(next.spotify.connected);
        assert_eq!(next.spotify.client_id, "abc");
        assert!(next.general.start_with_windows);
        assert_eq!(next.version, now.version);
    }

    #[test]
    fn switching_between_two_presets_round_trips() {
        let a = custom();
        let b = Settings::default();
        let (sa, sb) = (snapshot(&a).unwrap(), snapshot(&b).unwrap());
        let on_b = apply_to(&a, &sb).unwrap();
        assert_eq!(on_b, b);
        assert_eq!(apply_to(&on_b, &sa).unwrap(), a);
    }

    #[test]
    fn maps_are_replaced_not_unioned() {
        let mut a = Settings::default();
        a.hotkeys.modules.insert("extra".into(), "Ctrl+Alt+KeyE".into());
        let snap_default = snapshot(&Settings::default()).unwrap();
        let next = apply_to(&a, &snap_default).unwrap();
        assert!(!next.hotkeys.modules.contains_key("extra"));
    }

    #[test]
    fn old_partial_preset_fills_defaults_and_is_clamped() {
        let next = apply_to(&Settings::default(), &json!({ "appearance": { "opacity": 5 } })).unwrap();
        assert!(next.appearance.opacity >= 50);
        assert_eq!(next.focus, Settings::default().focus);
    }

    #[test]
    fn modified_flag_follows_changes() {
        let s = custom();
        let f = PresetFile {
            active: Some("x".into()),
            presets: vec![Preset { id: "x".into(), name: "Work".into(), saved_at: 1, settings: snapshot(&s).unwrap() }],
            ..Default::default()
        };
        assert!(!list_of(&f, &s).modified);
        let mut changed = s.clone();
        changed.appearance.opacity = 90;
        assert!(list_of(&f, &changed).modified);
        // Linking an account is not a preset change.
        let mut linked = s.clone();
        linked.spotify.connected = true;
        assert!(!list_of(&f, &linked).modified);
        assert!(!list_of(&PresetFile { active: None, ..f }, &changed).modified);
    }

    #[test]
    fn names_are_trimmed_and_unique() {
        let f = PresetFile {
            active: None,
            presets: vec![Preset { id: "x".into(), name: "Work".into(), saved_at: 1, settings: json!({}) }],
            ..Default::default()
        };
        assert_eq!(clean_name("  Late   night ", &f, None).unwrap(), "Late night");
        assert!(clean_name("   ", &f, None).is_err());
        assert!(clean_name("work", &f, None).is_err());
        assert_eq!(clean_name("WORK", &f, Some("x")).unwrap(), "WORK");
        assert!(clean_name(&"a".repeat(MAX_NAME + 1), &f, None).is_err());
    }

    #[test]
    fn file_round_trips_and_survives_damage() {
        let dir = std::env::temp_dir().join(format!("atoll-presets-test-{}", new_id()));
        let p = dir.join("presets.json");
        assert_eq!(load_from(&p), PresetFile::default());

        let f = PresetFile {
            active: Some("x".into()),
            presets: vec![Preset { id: "x".into(), name: "Work".into(), saved_at: 7, settings: snapshot(&custom()).unwrap() }],
            ..Default::default()
        };
        save_to(&p, &f).unwrap();
        assert_eq!(load_from(&p), f);

        // Active id pointing at a deleted preset is dropped.
        save_to(&p, &PresetFile { active: Some("gone".into()), presets: f.presets.clone(), ..Default::default() }).unwrap();
        assert_eq!(load_from(&p).active, None);

        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(load_from(&p), PresetFile::default());
        assert!(p.with_extension("json.broken").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
