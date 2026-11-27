//! Settings → Privacy and data, and Settings → AI agents → connection:
//! export / import settings, clear all data, test the Claude Code connection.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::settings::Settings;
use crate::AppState;

/// Start a console program without flashing a window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

async fn dialog_path(app: &AppHandle, save: bool) -> Result<Option<std::path::PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().add_filter("Atoll settings", &["json"]);
        if let Some(win) = app.get_webview_window(crate::windows::DASHBOARD) {
            d = d.set_parent(&win);
        }
        let picked = if save {
            d.set_file_name("atoll-settings.json").set_title("Export Atoll settings").blocking_save_file()
        } else {
            d.set_title("Import Atoll settings").blocking_pick_file()
        };
        picked.and_then(|p| p.into_path().ok())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Save the current settings to a JSON file the user picks. `Ok(false)` = cancelled.
pub async fn export_settings(app: &AppHandle) -> Result<bool, String> {
    let Some(path) = dialog_path(app, true).await? else { return Ok(false) };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("Could not write the file: {e}"))?;
    Ok(true)
}

/// Replace the settings with the ones in a file (checked and clamped first).
pub async fn import_settings(app: &AppHandle) -> Result<bool, String> {
    let Some(path) = dialog_path(app, false).await? else { return Ok(false) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Could not read the file: {e}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|_| "That file isn't an Atoll settings file.".to_string())?;
    if !value.is_object() || value.get("appearance").is_none() {
        return Err("That file isn't an Atoll settings file.".into());
    }
    let next = Settings::default().merged(&value)?;
    crate::commit_settings(app, next);
    Ok(true)
}

/// Settings back to defaults and presets deleted; clipboard history, whiteboard, file tray list,
/// note, focus history, the Spotify sign-in and the Ask chat and key removed. Claude Code hooks are
/// left alone (remove them in AI agents) and your own files are never touched.
pub fn clear_all_data(app: &AppHandle) -> Result<(), String> {
    crate::commit_settings(app, Settings::default());
    crate::presets::clear_all();
    crate::focus::clear_history();
    crate::focus::settings_changed(app);
    crate::clipboard::clear_everything(app);
    crate::board::clear(app);
    crate::tray::clear(app);
    crate::tray::note_set(app, "")?;
    crate::spotify::unlink_account(app);
    crate::ask::clear_everything(app); // ask-assistant
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    /// "connected" | "notfound" | "error" | "off"
    pub state: String,
    /// Claude Code seems to be installed for this user.
    pub installed: bool,
    pub hooks: crate::hooks::HookStatus,
    /// Last hook event received (ms since epoch, 0 = none since Atoll started).
    pub last_event_at: u64,
}

fn claude_installed() -> bool {
    if crate::paths::claude_settings().parent().is_some_and(|d| d.exists()) {
        return true;
    }
    Command::new("where.exe")
        .arg("claude")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn claude_status(app: &AppHandle) -> ClaudeStatus {
    let hooks = crate::hooks::status();
    let state_ = app.state::<AppState>();
    let on = state_.settings.lock().unwrap().modules.claude;
    let installed = claude_installed();
    let state = if !on {
        "off"
    } else if !installed {
        "notfound"
    } else if hooks.all_installed && hooks.bridge_present {
        "connected"
    } else {
        "error"
    };
    ClaudeStatus { state: state.into(), installed, hooks, last_event_at: state_.last_hook_ms.load(Ordering::Relaxed) }
}

/// Run the installed bridge in `ping` mode: the same program, file and token
/// Claude Code's hooks use, so success means hooks can reach the island.
pub async fn test_connection() -> Result<String, String> {
    let bridge = crate::paths::installed_bridge();
    if !bridge.exists() {
        return Err("The hooks aren't installed yet. Select Install hooks first.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut child = Command::new(&bridge)
            .arg("ping")
            .creation_flags(CREATE_NO_WINDOW)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start the bridge: {e}"))?;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if status.success() {
                    return Ok(format!("The connection works ({} ms).", started.elapsed().as_millis()));
                }
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                return Err(if err.trim().is_empty() { "The bridge could not reach Atoll.".into() } else { err.trim().to_string() });
            }
            if started.elapsed() > Duration::from_secs(6) {
                let _ = child.kill();
                return Err("The test timed out.".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
