//! Installs / removes Crest's Claude Code hooks in `~/.claude/settings.json`.
//!
//! - Merges: other hooks (e.g. your graphify hooks) are left untouched.
//! - Our entries are recognised by the command containing `atoll-bridge`
//!   (or `crest-bridge`, its name up to 1.0.3; those are moved on start).
//! - A timestamped backup of settings.json is written before every change.
//! - The bridge exe is copied to `%USERPROFILE%\.atoll\bin` so hooks keep
//!   working when the project is rebuilt or moved (not AppData: see
//!   `paths::shared_dir`). Hooks left at the old AppData path are moved on start.
//!
//! Config format: https://code.claude.com/docs/en/hooks ("Hook locations", "Matcher patterns").

use std::path::Path;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::paths;

/// Current bridge name first, then older names that still count as ours.
const MARKERS: &[&str] = &["atoll-bridge", "crest-bridge"];

fn is_our_command(command: &str) -> bool {
    MARKERS.iter().any(|m| command.contains(m))
}

/// (event, uses a tool-name matcher, hook timeout in seconds).
/// Events without matchers match everything. PermissionRequest waits for a
/// click in the island, so it gets a long timeout (the island itself gives up
/// after `permissions::WAIT` and hands the prompt back to Claude Code).
const EVENTS: &[(&str, bool, u32)] = &[
    ("SessionStart", false, 10),
    ("SessionEnd", false, 10),
    ("UserPromptSubmit", false, 10),
    ("PreToolUse", true, 10),
    ("PermissionRequest", true, 600),
    ("PostToolUse", true, 10),
    ("PostToolUseFailure", true, 10),
    ("Notification", false, 10),
    ("Stop", false, 10),
    ("StopFailure", false, 10),
];

/// Our status line (usage ring). Only installed when you have none of your own.
fn statusline_value() -> Value {
    json!({ "type": "command", "command": format!("{} statusline", bridge_command()), "refreshInterval": 60 })
}

fn statusline_is_ours(settings: &Value) -> bool {
    settings.pointer("/statusLine/command").and_then(Value::as_str).is_some_and(is_our_command)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed_events: Vec<String>,
    pub all_installed: bool,
    pub bridge_present: bool,
    pub settings_path: String,
    pub bridge_path: String,
    /// "crest" (ours), "other" (yours, left alone) or "none".
    pub status_line: String,
}

fn bridge_command() -> String {
    let p = paths::installed_bridge().to_string_lossy().replace('\\', "/");
    // Claude Code runs shell-form hooks with Git Bash on Windows. An unquoted
    // forward-slash path also works in PowerShell; quote only if needed.
    if p.contains(' ') { format!("\"{p}\"") } else { p }
}

fn read_settings(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("settings.json is not valid JSON: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn write_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let stamp = crate::sessions::now_ms();
        let backup = path.with_file_name(format!("settings.json.crest-backup-{stamp}"));
        std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.crest-tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn is_ours(handler: &Value) -> bool {
    handler.get("command").and_then(Value::as_str).is_some_and(is_our_command)
}

/// Remove every Crest handler from a settings object, pruning empty groups.
fn strip_ours(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut() {
        if let Some(list) = groups.as_array_mut() {
            for group in list.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    handlers.retain(|h| !is_ours(h));
                }
            }
            list.retain(|g| g.get("hooks").and_then(Value::as_array).is_some_and(|h| !h.is_empty()));
        }
    }
    hooks.retain(|_, groups| groups.as_array().is_some_and(|l| !l.is_empty()));
    if hooks.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
    }
}

fn add_ours(settings: &mut Value) {
    let obj = settings.as_object_mut().expect("settings root is an object");
    let hooks = obj.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().expect("hooks is an object");
    for (event, tool_matcher, timeout) in EVENTS {
        let handler = json!({ "type": "command", "command": bridge_command(), "timeout": timeout });
        let mut group = json!({ "hooks": [handler] });
        if *tool_matcher {
            group["matcher"] = json!("*");
        }
        let list = hooks.entry(*event).or_insert_with(|| json!([]));
        if let Some(list) = list.as_array_mut() {
            list.push(group);
        }
    }
}

pub fn status() -> HookStatus {
    let settings = read_settings(&paths::claude_settings()).unwrap_or(json!({}));
    let mut installed = Vec::new();
    if let Some(hooks) = settings.get("hooks").and_then(Value::as_object) {
        for (event, groups) in hooks {
            let has = groups.as_array().into_iter().flatten().any(|g| {
                g.get("hooks").and_then(Value::as_array).into_iter().flatten().any(is_ours)
            });
            if has {
                installed.push(event.clone());
            }
        }
    }
    let all = EVENTS.iter().all(|(e, _, _)| installed.iter().any(|i| i == e));
    let status_line = if statusline_is_ours(&settings) {
        "crest"
    } else if settings.get("statusLine").is_some() {
        "other"
    } else {
        "none"
    };
    HookStatus {
        status_line: status_line.into(),
        installed_events: installed,
        all_installed: all,
        bridge_present: paths::installed_bridge().exists(),
        settings_path: paths::claude_settings().display().to_string(),
        bridge_path: paths::installed_bridge().display().to_string(),
    }
}

pub fn install() -> Result<HookStatus, String> {
    // 1. Put the bridge in its stable location.
    let src = paths::built_bridge().filter(|p| p.exists()).ok_or(
        "atoll-bridge.exe was not found next to the app. Build it first: npm run bridge",
    )?;
    let dst = paths::installed_bridge();
    std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(&src, &dst).map_err(|e| format!("could not copy bridge to {}: {e}", dst.display()))?;

    // 2. Merge our hooks into settings.json (idempotent: strip, then add).
    let path = paths::claude_settings();
    let mut settings = read_settings(&path)?;
    if !settings.is_object() {
        return Err("settings.json root is not a JSON object".into());
    }
    strip_ours(&mut settings);
    add_ours(&mut settings);
    // Status line feeds the usage ring; never replace one you set up yourself.
    if settings.get("statusLine").is_none() || statusline_is_ours(&settings) {
        settings["statusLine"] = statusline_value();
    }
    write_settings(&path, &settings)?;
    Ok(status())
}

/// True when our hooks or status line are installed but run a bridge other
/// than the current one (e.g. the AppData path used up to 1.0.2, or the
/// `crest-bridge.exe` name used up to 1.0.3).
fn points_elsewhere(settings: &Value) -> bool {
    let want = bridge_command();
    let hook_moved = settings.get("hooks").and_then(Value::as_object).into_iter().flat_map(|h| h.values()).any(|groups| {
        groups.as_array().into_iter().flatten().any(|g| {
            g.get("hooks").and_then(Value::as_array).into_iter().flatten().any(|h| {
                is_ours(h) && h.get("command").and_then(Value::as_str) != Some(want.as_str())
            })
        })
    });
    let line_moved = statusline_is_ours(settings)
        && settings.pointer("/statusLine/command").and_then(Value::as_str) != Some(format!("{want} statusline").as_str());
    hook_moved || line_moved
}

/// On start: if the hooks still point at an old bridge location, or the
/// bridge they run is missing, reinstall them so you don't have to. Leaves
/// settings.json alone when the hooks aren't installed at all.
pub fn migrate_if_moved() {
    let Ok(settings) = read_settings(&paths::claude_settings()) else { return };
    let has_ours = !status().installed_events.is_empty() || statusline_is_ours(&settings);
    if !has_ours || (!points_elsewhere(&settings) && paths::installed_bridge().exists()) {
        return;
    }
    match install() {
        Ok(_) => {
            println!("[atoll] hooks moved to {}", paths::installed_bridge().display());
            let _ = std::fs::remove_dir_all(paths::legacy_bridge_dir());
            // May fail while a hook is still running it; then it goes next start.
            let _ = std::fs::remove_file(paths::legacy_installed_bridge());
        }
        Err(e) => eprintln!("[atoll] could not move the hooks: {e}"),
    }
}

/// Written when you select Remove in Settings → AI agents, so Atoll doesn't put
/// the hooks back by itself. Install deletes it. Kept with Atoll's own data, so
/// a reinstall that keeps the data keeps the choice.
fn removed_marker() -> std::path::PathBuf {
    paths::local_dir().join("hooks-removed")
}

/// Remember (or forget) that you removed the hooks yourself.
pub fn set_removed_by_user(removed: bool) {
    let marker = removed_marker();
    if removed {
        let _ = std::fs::create_dir_all(paths::local_dir());
        let _ = std::fs::write(&marker, b"");
    } else {
        let _ = std::fs::remove_file(&marker);
    }
}

/// Should a start set the bridge and hooks up by itself?
fn should_auto_install(removed_by_user: bool, claude_present: bool, has_ours: bool) -> bool {
    !removed_by_user && claude_present && !has_ours
}

/// On start (and when Connect is turned on): a fresh install has the bridge
/// only next to Atoll's exe, so nothing reaches the island until the hooks
/// are installed. Install them, unless Claude Code isn't on this PC, the hooks
/// are already there (even partly), or you removed them yourself.
pub fn install_if_missing() {
    let Ok(settings) = read_settings(&paths::claude_settings()) else { return };
    let has_ours = !status().installed_events.is_empty() || statusline_is_ours(&settings);
    if !should_auto_install(removed_marker().exists(), paths::claude_dir().is_dir(), has_ours) {
        return;
    }
    match install() {
        Ok(_) => println!("[atoll] bridge and hooks installed at {}", paths::installed_bridge().display()),
        Err(e) => eprintln!("[atoll] could not install the hooks: {e}"),
    }
}

pub fn uninstall() -> Result<HookStatus, String> {
    let path = paths::claude_settings();
    let mut settings = read_settings(&path)?;
    let before = settings.clone();
    strip_ours(&mut settings);
    if statusline_is_ours(&settings) {
        settings.as_object_mut().unwrap().remove("statusLine");
    }
    // Nothing of ours there (e.g. the uninstaller on a PC without hooks): leave the file alone.
    if settings != before {
        write_settings(&path, &settings)?;
    }
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_install_only_on_a_clean_pc_with_claude_code() {
        assert!(should_auto_install(false, true, false));
        assert!(!should_auto_install(true, true, false), "you removed them: leave them off");
        assert!(!should_auto_install(false, false, false), "no Claude Code: don't create its settings");
        assert!(!should_auto_install(false, true, true), "already installed (or partly): leave as is");
    }

    #[test]
    fn merge_keeps_foreign_hooks() {
        let mut s = json!({
            "model": "x",
            "hooks": { "PreToolUse": [ { "matcher": "Bash|Grep", "hooks": [ { "type": "command", "command": "graphify hook-guard search" } ] } ] }
        });
        add_ours(&mut s);
        add_ours(&mut s); // would duplicate without strip...
        strip_ours(&mut s);
        add_ours(&mut s); // ...so install always strips first
        let pre = s["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0]["matcher"], "Bash|Grep");

        strip_ours(&mut s);
        assert_eq!(s["hooks"].as_object().unwrap().len(), 1);
        assert_eq!(s["model"], "x");
    }

    #[test]
    fn statusline_ownership() {
        let mine = json!({ "statusLine": statusline_value() });
        assert!(statusline_is_ours(&mine));
        let yours = json!({ "statusLine": { "type": "command", "command": "~/.claude/statusline.sh" } });
        assert!(!statusline_is_ours(&yours));
        assert!(mine["statusLine"]["command"].as_str().unwrap().ends_with(" statusline"));
    }

    #[test]
    fn old_bridge_path_is_detected() {
        // Freshly installed: nothing to move.
        let mut fresh = json!({ "statusLine": statusline_value() });
        add_ours(&mut fresh);
        assert!(!points_elsewhere(&fresh));
        // Hooks from 1.0.2 (AppData bridge) must be moved.
        let old = "C:/Users/x/AppData/Local/Crest/bin/crest-bridge.exe";
        let stale = json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": old } ] } ] } });
        assert!(points_elsewhere(&stale));
        let stale_line = json!({ "statusLine": { "type": "command", "command": format!("{old} statusline") } });
        assert!(points_elsewhere(&stale_line));
        // Hooks from 1.0.3 (old exe name, same folder) must be moved too.
        let renamed = "C:/Users/x/.atoll/bin/crest-bridge.exe";
        let stale_name = json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": renamed } ] } ] } });
        assert!(points_elsewhere(&stale_name));
        assert!(paths::installed_bridge().ends_with("atoll-bridge.exe"));
        // ...and installing replaces them instead of adding a second set.
        let mut both = stale_name.clone();
        strip_ours(&mut both);
        add_ours(&mut both);
        assert_eq!(both["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(!points_elsewhere(&both));
        // Someone else's hooks are never ours to move.
        let foreign = json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "graphify hook" } ] } ] } });
        assert!(!points_elsewhere(&foreign));
        // New location is outside AppData.
        assert!(!paths::installed_bridge().to_string_lossy().contains("AppData"));
        assert!(!paths::runtime_file().to_string_lossy().contains("AppData"));
    }
}
