//! Global hotkeys (tauri-plugin-global-shortcut, https://v2.tauri.app/plugin/global-shortcut/).
//!
//! Hotkeys are stored as "Ctrl+Alt+Space" style strings. Every time the
//! settings change we unregister everything and register the new set,
//! recording per-hotkey errors (invalid text, duplicate, taken by another
//! app) so the dashboard can show them.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::settings::Hotkeys;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    ToggleIsland,
    ExpandIsland,
    OpenSettings,
    /// Per-module hotkey, by module key (e.g. "claude", "spotify").
    Module(String),
}

/// Managed state: what each registered hotkey does, and per-hotkey errors.
#[derive(Default)]
pub struct Shortcuts {
    /// Registered shortcut id -> what it does.
    actions: Mutex<HashMap<u32, Action>>,
    /// Hotkey key ("toggleIsland", "module:spotify") -> error message, if any.
    pub status: Mutex<BTreeMap<String, Option<String>>>,
}

/// Parse "Ctrl+Alt+Space" into a plugin shortcut.
pub fn parse(combo: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = combo.parse().map_err(|_| "Not a valid key combination".to_string())?;
    if !combo.contains('+') {
        return Err("Add at least one modifier (Ctrl, Alt, Shift or Win)".into());
    }
    Ok(shortcut)
}

/// Settings → Hotkeys: while a combination is being recorded, release every
/// hotkey so the recorder can see combinations Atoll already uses.
pub fn pause(app: &AppHandle, paused: bool) {
    if paused {
        let _ = app.global_shortcut().unregister_all();
    } else {
        let hotkeys = app.state::<crate::AppState>().settings.lock().unwrap().hotkeys.clone();
        apply(app, &hotkeys);
    }
}

/// Replace all registered hotkeys with the ones in `hotkeys`.
pub fn apply(app: &AppHandle, hotkeys: &Hotkeys) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let mut wanted: Vec<(String, String, Action)> = vec![
        ("toggleIsland".into(), hotkeys.toggle_island.clone(), Action::ToggleIsland),
        ("expandIsland".into(), hotkeys.expand_island.clone(), Action::ExpandIsland),
        ("openSettings".into(), hotkeys.open_settings.clone(), Action::OpenSettings),
    ];
    for (module, combo) in &hotkeys.modules {
        wanted.push((format!("module:{module}"), combo.clone(), Action::Module(module.clone())));
    }

    let mut actions = HashMap::new();
    let mut status = BTreeMap::new();
    for (key, combo, action) in wanted {
        if combo.trim().is_empty() {
            status.insert(key, None);
            continue;
        }
        let result = parse(&combo).and_then(|sc| {
            if actions.contains_key(&sc.id()) {
                return Err("Already used by another Atoll hotkey".into());
            }
            gs.register(sc).map_err(|e| {
                eprintln!("[crest] hotkey {combo} not registered: {e}");
                "Another app already uses this combination. Pick a different one.".to_string()
            })?;
            actions.insert(sc.id(), action);
            Ok(())
        });
        status.insert(key, result.err());
    }

    let state = app.state::<Shortcuts>();
    *state.actions.lock().unwrap() = actions;
    *state.status.lock().unwrap() = status.clone();
    let _ = app.emit("hotkey-status", status);
}

/// Plugin callback for every hotkey press.
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let action = app.state::<Shortcuts>().actions.lock().unwrap().get(&shortcut.id()).cloned();
    match action {
        Some(Action::ToggleIsland) => crate::windows::toggle_island(app),
        Some(Action::ExpandIsland) => crate::windows::toggle_island_expanded(app),
        Some(Action::OpenSettings) => crate::windows::open_dashboard(app),
        Some(Action::Module(m)) if m == "claude" => crate::windows::toggle_island_expanded(app),
        Some(Action::Module(m)) if m == "spotify" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::spotify::action(&app, "toggle", None).await {
                    eprintln!("[crest] spotify hotkey: {e}");
                }
            });
        }
        Some(Action::Module(m)) if m == "allowPermission" || m == "denyPermission" => {
            // Answer the oldest waiting permission request.
            let state = app.state::<crate::AppState>();
            let first = state.permissions.lock().unwrap().views().first().map(|p| p.id);
            if let Some(id) = first {
                let choice = if m == "allowPermission" {
                    crate::permissions::Choice::Allow
                } else {
                    crate::permissions::Choice::Deny(None)
                };
                state.permissions.lock().unwrap().resolve(id, choice);
                crate::emit_permissions(app);
            }
        }
        // Clear whatever message the island shows (it knows what that is).
        Some(Action::Module(m)) if m == "dismissMessage" => {
            let _ = app.emit_to(crate::windows::ISLAND, "dismiss-message", ());
        }
        // Start, pause or resume the focus timer.
        Some(Action::Module(m)) if m == "focus" => crate::focus::toggle(app),
        // ---- search-clipboard: begin ----
        Some(Action::Module(m)) if m == "palette" => crate::search::palette::toggle(app),
        // ---- search-clipboard: end ----
        Some(Action::Module(m)) if matches!(m.as_str(), "clipboard" | "whiteboard" | "fileTray" | "note" | "calendar" | "ask") => {
            crate::windows::open_tab(app, &m)
        }
        Some(Action::Module(m)) => {
            // Modules that are not built yet just announce the press.
            let _ = app.emit("module-hotkey", m);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dashboard_format() {
        for combo in ["Ctrl+Alt+Space", "Ctrl+Alt+Period", "Ctrl+Alt+Comma", "Ctrl+Shift+KeyK", "Alt+Digit1", "Super+F5"] {
            assert!(parse(combo).is_ok(), "{combo}");
        }
        assert!(parse("Space").is_err());
        assert!(parse("Ctrl+Nope").is_err());
        assert_ne!(parse("Ctrl+Alt+Space").unwrap().id(), parse("Ctrl+Alt+Period").unwrap().id());
    }
}
