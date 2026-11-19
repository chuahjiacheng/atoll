//! Crest app entry: shared state, commands for the UI, startup wiring.
//!
//! Module map:
//! - `settings`    persisted user settings (appearance, modules, hotkeys)
//! - `windows`     island + dashboard windows, placement
//! - `shortcuts`   global hotkeys
//! - `server`      local 127.0.0.1 listener the hook bridge posts to
//! - `sessions`    hook events -> per-session status/activity
//! - `permissions` pending Allow/Deny requests
//! - `alerts`      chime, toast and island flash
//! - `usage`       5-hour / weekly limits from the status line (usage ring)
//! - `chatwatch`   experimental Chat tab "reply finished" watcher
//! - `spotify`     Spotify controls, devices (optional account link, PKCE)
//! - `spotify_local` Spotify on this PC via the Windows media session
//! - `calendar`    Google Calendar sign-in (PKCE) and upcoming events
//! - `lyrics`      synced lyrics from LRCLIB
//! - `clipboard`   clipboard history (listener + store)
//! - `board`       whiteboard cards
//! - `tray`        file tray and quick note
//! - `focus`       focus / break timer, alert digest, distraction nudge, history
//! - `winclip`     low-level Win32 clipboard read/write
//! - `paste`       paste into the app you were using
//! - `media`       images, thumbnails, JSON stores, drag-out
//! - `hooks`       install/remove our hooks in ~/.claude/settings.json
//! - `paths`       file locations
//! - `backdrop`    Windows acrylic blur under the island's exact shape
//! - `liquid`      refracting liquid glass under the island (screen capture + shader)

mod alerts;
mod backdrop;
mod liquid;
mod board;
mod calendar;
mod google_client;
mod chatwatch;
mod clipboard;
mod crypt;
mod data;
mod dnd;
mod focus;
mod hooks;
mod lyrics;
mod media;
mod paste;
mod paths;
mod permissions;
mod reply;
mod server;
mod sessions;
mod settings;
mod arbiter; // adaptive-island: decides what the island shows (statechart)
mod shape; // adaptive-island: the island's shape over time, shared by the glass and the hover test
mod shortcuts;
mod spotify;
mod spotify_local;
mod system;
mod tray;
mod usage;
mod winclip;
mod windows;
// ---- search-clipboard: begin ----
mod clipintel;
mod ocr;
mod search;
// ---- search-clipboard: end ----

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

use permissions::{Choice, PermissionQueue, PermissionView};
use sessions::{Session, SessionStore};
use settings::Settings;
use shortcuts::Shortcuts;
use windows::{MonitorInfo, DASHBOARD, ISLAND};

/// Interactive area of the island in CSS pixels, relative to the window.
#[derive(Clone, Copy, Default)]
struct HitRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

pub struct AppState {
    pub sessions: Mutex<SessionStore>,
    pub permissions: Mutex<PermissionQueue>,
    pub settings: Mutex<Settings>,
    /// Settings changed since the last save (saved by a background thread).
    settings_dirty: AtomicBool,
    hit_rect: Mutex<HitRect>,
    port: AtomicU16,
    /// When the last Claude Code hook event arrived (ms since epoch).
    pub last_hook_ms: std::sync::atomic::AtomicU64,
}

impl AppState {
    /// Is the Claude Code module switched on?
    pub fn enabled(&self) -> bool {
        self.settings.lock().unwrap().modules.claude
    }
}

/// Port of the local 127.0.0.1 listener (0 before it starts).
pub fn listener_port(app: &AppHandle) -> u16 {
    app.state::<AppState>().port.load(Ordering::Relaxed)
}

/// Push the current session list to the UI.
pub fn emit_sessions(app: &AppHandle) {
    let (list, feed) = {
        let state = app.state::<AppState>();
        let mut store = state.sessions.lock().unwrap();
        (store.snapshot(), store.feed())
    };
    let _ = app.emit("sessions", list);
    let _ = app.emit("agent-feed", feed);
    system::update_tray(app);
}

/// Push the pending permission requests to the UI.
pub fn emit_permissions(app: &AppHandle) {
    let list = app.state::<AppState>().permissions.lock().unwrap().views();
    let _ = app.emit("permissions", list);
    system::update_tray(app);
}

/// Store new settings, apply their side effects and tell every window.
pub(crate) fn commit_settings(app: &AppHandle, next: Settings) {
    let state = app.state::<AppState>();
    let prev = std::mem::replace(&mut *state.settings.lock().unwrap(), next.clone());
    state.settings_dirty.store(true, Ordering::Relaxed);

    if prev.appearance != next.appearance {
        windows::place_island(app, &next.appearance);
    }
    if prev.hotkeys != next.hotkeys {
        shortcuts::apply(app, &next.hotkeys);
    }
    if prev.general.start_with_windows != next.general.start_with_windows {
        sync_autostart(app, next.general.start_with_windows);
    }
    if prev.clipboard != next.clipboard {
        clipboard::settings_changed(app);
    }
    if prev.modules != next.modules {
        search::settings_changed(); // search-clipboard
    }
    if prev.usage.from_sign_in != next.usage.from_sign_in || prev.modules.usage_ring != next.modules.usage_ring {
        usage::poke(app);
    }
    if prev.general.show_tray != next.general.show_tray {
        system::update_tray(app);
    }
    if prev.spotify.connected != next.spotify.connected || prev.modules.spotify != next.modules.spotify {
        spotify::emit_status(app);
        spotify_local::poke();
    }
    if prev.calendar != next.calendar || prev.modules.calendar != next.modules.calendar {
        calendar::emit_status(app);
        if prev.calendar.shown != next.calendar.shown
            || prev.calendar.days_ahead != next.calendar.days_ahead
            || (next.modules.calendar && !prev.modules.calendar)
        {
            calendar::poke(app);
        }
    }
    if prev.focus != next.focus || prev.modules.focus != next.modules.focus {
        focus::settings_changed(app);
    }
    if prev.modules.claude != next.modules.claude && !next.modules.claude {
        // Switching Claude Code off hands any waiting prompts back to Claude Code.
        let ids: Vec<String> = state.permissions.lock().unwrap().views().into_iter().map(|p| p.session_id).collect();
        for id in ids {
            state.permissions.lock().unwrap().pass_session(&id);
        }
        emit_permissions(app);
    }
    let _ = app.emit("settings", &next);
}

fn sync_autostart(app: &AppHandle, wanted: bool) {
    let launcher = app.autolaunch();
    let current = launcher.is_enabled().unwrap_or(false);
    let result = match (wanted, current) {
        (true, false) => launcher.enable(),
        (false, true) => launcher.disable(),
        _ => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("[crest] start-with-Windows change failed: {e}");
    }
}

// ---------- commands called from the UI ----------

#[tauri::command]
fn get_sessions(state: State<AppState>) -> Vec<Session> {
    state.sessions.lock().unwrap().snapshot()
}

#[tauri::command]
fn get_agent_feed(state: State<AppState>) -> sessions::Feed {
    state.sessions.lock().unwrap().feed()
}

/// The island's power button.
#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn dismiss_session(app: AppHandle, id: String) {
    if app.state::<AppState>().sessions.lock().unwrap().dismiss(&id) {
        emit_sessions(&app);
    }
}

/// Answer Claude's end-of-turn question from the island: types `text` into the
/// Claude app and sends it (like its own reply buttons). Async: it waits a
/// moment for the app to come to the front.
#[tauri::command(async)]
fn reply_in_claude(app: AppHandle, session_id: String, text: String) -> Result<(), String> {
    let text: String = text.trim().chars().take(500).collect();
    if text.is_empty() {
        return Err("Nothing to send.".into());
    }
    // A reply can take several seconds to land (Claude shows its buttons late
    // while in the background). Ignore another pick for the same session until
    // then, so one answer is never sent twice.
    static IN_FLIGHT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    {
        let mut busy = IN_FLIGHT.lock().unwrap();
        if busy.contains(&session_id) {
            return Ok(());
        }
        busy.push(session_id.clone());
    }
    let is_choice = app.state::<AppState>().sessions.lock().unwrap().is_choice(&session_id, &text);
    let sent = reply::send_to_claude(&text, is_choice);
    IN_FLIGHT.lock().unwrap().retain(|s| s != &session_id);
    sent?;
    if app.state::<AppState>().sessions.lock().unwrap().answered(&session_id) {
        emit_sessions(&app);
    }
    Ok(())
}

/// Clear a session's error / "needs you" / "done" message; it goes back to idle.
#[tauri::command]
fn clear_session(app: AppHandle, id: String) {
    if app.state::<AppState>().sessions.lock().unwrap().clear(&id) {
        emit_sessions(&app);
    }
}

#[tauri::command]
fn get_permissions(state: State<AppState>) -> Vec<PermissionView> {
    state.permissions.lock().unwrap().views()
}

/// Allow / Deny / Pass ("answer in Claude") one pending request.
/// A deny can carry a typed reply, which Claude Code receives as the reason.
#[tauri::command]
fn decide_permission(app: AppHandle, id: u64, choice: String, reply: Option<String>, answers: Option<Value>) -> Result<(), String> {
    // "answer" = Claude's question answered in the island: { question: label }.
    let mut choice = if choice == "answer" {
        let answers = answers.filter(|a| a.as_object().is_some_and(|o| !o.is_empty() && o.values().all(Value::is_string)));
        Choice::Answer(answers.ok_or("Pick an answer first.")?)
    } else {
        Choice::parse(&choice).ok_or("choice must be allow, deny, pass or answer")?
    };
    if let (Choice::Deny(_), Some(r)) = (&choice, reply) {
        choice = Choice::Deny(Some(r.chars().take(2000).collect()));
    }
    let resolved = app.state::<AppState>().permissions.lock().unwrap().resolve(id, choice.clone());
    println!("[crest] permission #{id}: {choice:?} (resolved: {resolved})");
    emit_permissions(&app);
    if resolved { Ok(()) } else { Err("That request is no longer waiting.".into()) }
}

#[tauri::command]
fn set_hit_rect(state: State<AppState>, x: f64, y: f64, w: f64, h: f64) {
    *state.hit_rect.lock().unwrap() = HitRect { x, y, w, h };
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// Merge a full or partial settings object; returns the stored (clamped) result.
#[tauri::command]
fn update_settings(app: AppHandle, patch: Value) -> Result<Settings, String> {
    let next = app.state::<AppState>().settings.lock().unwrap().merged(&patch)?;
    commit_settings(&app, next.clone());
    Ok(next)
}

/// Reset one section ("appearance", "hotkeys", ...) or everything ("all").
#[tauri::command]
fn reset_settings(app: AppHandle, section: String) -> Result<Settings, String> {
    let defaults = Settings::default();
    let mut next = app.state::<AppState>().settings.lock().unwrap().clone();
    match section.as_str() {
        "appearance" => next.appearance = defaults.appearance,
        "colors" => {
            next.appearance.accent = defaults.appearance.accent;
            next.appearance.busy = defaults.appearance.busy;
            next.appearance.done = defaults.appearance.done;
            next.appearance.error = defaults.appearance.error;
        }
        "hotkeys" => next.hotkeys = defaults.hotkeys,
        "modules" => {
            next.modules = defaults.modules;
            next.module_order = defaults.module_order;
        }
        "general" => {
            next.general = defaults.general;
            next.appearance.monitor = defaults.appearance.monitor;
            next.appearance.position_x = defaults.appearance.position_x;
            next.appearance.position_y = defaults.appearance.position_y;
        }
        "agents" => {
            next.claude = defaults.claude;
            next.usage = defaults.usage;
            next.chat_watcher = defaults.chat_watcher;
            next.modules.usage_ring = defaults.modules.usage_ring;
            next.modules.chat_watcher = defaults.modules.chat_watcher;
            next.alerts.usage_threshold = defaults.alerts.usage_threshold;
        }
        // Display choices only: the Client ID and callback port stay, so you stay connected.
        "spotify" => {
            next.spotify.lyrics = defaults.spotify.lyrics;
            next.spotify.show_in_pill = defaults.spotify.show_in_pill;
            next.spotify.progress_line = defaults.spotify.progress_line;
            next.spotify.pill_text = defaults.spotify.pill_text;
            next.spotify.pill_priority = defaults.spotify.pill_priority;
        }
        // Display choices only: the client ID and calendar choices stay.
        "calendar" => {
            next.calendar.days_ahead = defaults.calendar.days_ahead;
            next.calendar.show_declined = defaults.calendar.show_declined;
            next.calendar.pill_minutes = defaults.calendar.pill_minutes;
        }
        "clipboard" => {
            next.clipboard = defaults.clipboard;
            next.whiteboard = defaults.whiteboard;
        }
        "alerts" => {
            let threshold = next.alerts.usage_threshold;
            next.alerts = defaults.alerts;
            next.alerts.usage_threshold = threshold;
        }
        "mascot" => next.mascot = defaults.mascot,
        // The focus page also sets the end-of-session sound.
        "focus" => {
            next.focus = defaults.focus;
            next.alerts.timer = defaults.alerts.timer;
        }
        "all" => next = defaults,
        other => return Err(format!("unknown section {other}")),
    }
    next.sanitize();
    commit_settings(&app, next.clone());
    Ok(next)
}

#[tauri::command]
fn get_hotkey_status(state: State<Shortcuts>) -> BTreeMap<String, Option<String>> {
    state.status.lock().unwrap().clone()
}

/// Check a combo before saving it (used by the hotkey recorder).
#[tauri::command]
fn validate_hotkey(combo: String) -> Result<(), String> {
    shortcuts::parse(&combo).map(|_| ())
}

/// Development builds only: renew Claude Code's sign-in now, to test renewal.
#[cfg(debug_assertions)]
#[tauri::command]
async fn debug_renew_sign_in() -> Result<String, String> {
    usage::debug_renew().await
}

#[tauri::command]
fn pause_hotkeys(app: AppHandle, paused: bool) {
    shortcuts::pause(&app, paused);
}

#[tauri::command]
fn list_monitors(app: AppHandle) -> Vec<MonitorInfo> {
    windows::list_monitors(&app)
}

#[tauri::command]
fn open_dashboard(app: AppHandle) {
    windows::open_dashboard(&app);
}

#[tauri::command]
fn get_island_visible(app: AppHandle) -> bool {
    app.get_webview_window(ISLAND).and_then(|w| w.is_visible().ok()).unwrap_or(false)
}

#[tauri::command]
fn set_island_visible(app: AppHandle, visible: bool) {
    windows::set_island_visible(&app, visible);
}

#[tauri::command]
fn autostart_enabled(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn app_info(state: State<AppState>) -> Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "port": state.port.load(Ordering::Relaxed),
        "settingsPath": settings::path().display().to_string(),
        "dataDir": paths::local_dir().display().to_string(),
        "exe": std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
    })
}

/// "Send test alert" buttons in Settings (ignores trigger switches, not sound/toast/flash).
#[tauri::command]
fn test_alert(app: AppHandle, kind: String) -> Result<(), String> {
    let (kind, title, body) = match kind.as_str() {
        "done" => (alerts::Kind::Done, "shop-api is done", "Updated cart.rs and all tests pass."),
        "attention" => (alerts::Kind::Attention, "shop-api needs your permission", "Running npm test"),
        "error" => (alerts::Kind::Error, "shop-api stopped", "429 Too Many Requests"),
        "usage" => (alerts::Kind::Usage, "Claude usage at 90%", "Your 5-hour limit resets in 1h 12m."),
        "chat" => (alerts::Kind::ChatReply, "Claude replied", "A reply finished in the Claude app."),
        "timer" => (alerts::Kind::Timer, "Focus session over", "Time for a 5 minute break."),
        other => return Err(format!("unknown alert kind {other}")),
    };
    alerts::fire(&app, alerts::Alert { kind, session_id: None, title: title.into(), body: body.into() }, true);
    Ok(())
}

#[tauri::command]
fn get_usage(app: AppHandle) -> usage::UsageView {
    usage::current(&app)
}

#[tauri::command]
fn get_chatwatch_status(app: AppHandle) -> chatwatch::ChatWatchStatus {
    chatwatch::status(&app)
}

#[tauri::command]
fn calendar_status(app: AppHandle) -> calendar::CalendarStatus {
    calendar::status(&app)
}

#[tauri::command]
fn calendar_agenda(app: AppHandle) -> calendar::Agenda {
    calendar::agenda(&app)
}

#[tauri::command]
fn calendar_set_secret(app: AppHandle, secret: String) -> Result<(), String> {
    calendar::set_secret(&app, &secret)
}

#[tauri::command]
fn calendar_login(app: AppHandle) -> Result<(), String> {
    calendar::begin_login(&app)
}

#[tauri::command]
fn calendar_cancel_login(app: AppHandle) {
    calendar::cancel_login(&app);
}

#[tauri::command]
async fn calendar_logout(app: AppHandle) {
    calendar::logout(&app).await;
}

#[tauri::command]
fn calendar_refresh(app: AppHandle) {
    calendar::poke(&app);
}

#[tauri::command]
fn calendar_open(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Only web links from Google's own event data (event page, video call).
    if !url.starts_with("https://") {
        return Err("Only https links can be opened.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn spotify_status(app: AppHandle) -> spotify::SpotifyStatus {
    spotify::status(&app)
}

#[tauri::command]
fn spotify_login(app: AppHandle) -> Result<(), String> {
    spotify::begin_login(&app)
}

#[tauri::command]
fn spotify_logout(app: AppHandle) {
    spotify::unlink_account(&app);
}

#[tauri::command]
fn spotify_open_app(app: AppHandle) -> Result<(), String> {
    spotify::open_app(&app)
}

#[tauri::command]
fn spotify_player(app: AppHandle) -> Option<spotify::PlayerView> {
    spotify::player(&app)
}

#[tauri::command]
async fn spotify_action(app: AppHandle, action: String, value: Option<Value>) -> Result<(), String> {
    spotify::action(&app, &action, value).await
}

#[tauri::command]
async fn spotify_app_volume() -> Option<u8> {
    tauri::async_runtime::spawn_blocking(spotify::read_app_volume).await.ok().flatten()
}

#[tauri::command]
async fn spotify_devices(app: AppHandle) -> Result<Vec<spotify::Device>, String> {
    spotify::devices(&app).await
}

#[tauri::command]
fn get_lyrics(app: AppHandle, track_id: String) -> Option<lyrics::Lyrics> {
    lyrics::cached(&app, &track_id)
}

// ----- focus timer -----

#[tauri::command]
fn focus_get(app: AppHandle) -> focus::FocusView {
    focus::view(&app)
}

/// start | pause | resume | toggle | skip | reset | intention | dismiss | clearHistory
#[tauri::command]
fn focus_action(app: AppHandle, action: String, text: Option<String>) -> Result<focus::FocusView, String> {
    focus::action(&app, &action, text)
}

/// Windows' own Do Not Disturb can't be switched by apps: open its settings page.
#[tauri::command]
fn open_windows_dnd(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url("ms-settings:notifications", None::<&str>).map_err(|e| e.to_string())
}

// ----- clipboard history -----

// ---- search-clipboard: begin ----
#[tauri::command]
fn clipboard_transform(app: AppHandle, id: String, op: String, paste: bool) -> Result<(), String> {
    clipboard::transform(&app, &id, &op, paste)
}

/// `source`: "clipboard" | "whiteboard" | "note" | "fileTray" | "calendar" | "agent".
#[tauri::command]
fn search_query(app: AppHandle, query: String, limit: Option<usize>, source: Option<String>) -> Vec<search::Result> {
    let only = source.and_then(|s| serde_json::from_value::<search::index::Source>(serde_json::Value::String(s)).ok());
    search::query(&app, &query, limit.unwrap_or(30), only)
}

#[tauri::command]
fn search_act(app: AppHandle, key: String, action: String) -> Result<(), String> {
    search::act(&app, &key, &action)
}

#[tauri::command]
fn search_stats(app: AppHandle) -> search::Stats {
    search::stats(&app)
}

#[tauri::command]
fn palette_ready(app: AppHandle) {
    search::palette::ready(&app);
}

#[tauri::command]
fn palette_hide(app: AppHandle) {
    search::palette::hide(&app);
}
// ---- search-clipboard: end ----

#[tauri::command]
fn clipboard_list(app: AppHandle) -> Vec<clipboard::Clip> {
    clipboard::list(&app)
}

#[tauri::command]
fn clipboard_use(app: AppHandle, id: String, paste: bool) -> Result<(), String> {
    clipboard::use_item(&app, &id, paste)
}

#[tauri::command]
fn clipboard_pin(app: AppHandle, id: String, pinned: bool) {
    clipboard::set_pinned(&app, &id, pinned);
}

#[tauri::command]
fn clipboard_delete(app: AppHandle, id: String) {
    clipboard::delete(&app, &id);
}

#[tauri::command]
fn clipboard_clear(app: AppHandle) {
    clipboard::clear(&app);
}

#[tauri::command]
fn clipboard_drag(app: AppHandle, id: String) -> Result<(), String> {
    clipboard::drag(&app, &id)
}

/// Send a history item to the whiteboard or the file tray.
#[tauri::command]
fn clipboard_send(app: AppHandle, id: String, to: String) -> Result<(), String> {
    let content = clipboard::content_of(&app, &id)?;
    match to.as_str() {
        "whiteboard" => board::add_content(&app, &content, None),
        "fileTray" => tray::add(&app, dnd::as_files(&content)?),
        _ => return Err(format!("Unknown place {to}")),
    }
    Ok(())
}

/// Open or show in Explorer the first file of a "files" history item.
#[tauri::command]
fn clipboard_open_file(app: AppHandle, id: String, reveal: bool) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let Ok(winclip::Content::Files(files)) = clipboard::content_of(&app, &id) else {
        return Err("That item has no file.".into());
    };
    let path = files.into_iter().find(|f| std::path::Path::new(f).exists()).ok_or("That file has been moved or deleted.")?;
    if reveal {
        app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
    } else {
        app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
    }
}

// ----- drag and drop onto the island -----

#[tauri::command]
fn drop_commit(app: AppHandle, target: String, x: Option<f64>, y: Option<f64>) -> Result<(), String> {
    dnd::commit(&app, &target, x.zip(y))
}

// ----- whiteboard -----

#[tauri::command]
fn board_get(app: AppHandle) -> board::Board {
    board::get(&app)
}

#[tauri::command]
fn board_add_files(app: AppHandle, paths: Vec<String>, x: Option<f64>, y: Option<f64>) {
    board::add_files(&app, paths, x.zip(y));
}

#[tauri::command]
fn board_add_text(app: AppHandle, text: String, x: f64, y: f64) -> board::Card {
    board::add_text(&app, text, x, y)
}

#[tauri::command]
fn board_add_clipboard(app: AppHandle, x: Option<f64>, y: Option<f64>) -> Result<(), String> {
    board::add_from_clipboard(&app, x.zip(y))
}

#[tauri::command]
async fn board_pick_files(app: AppHandle) -> Result<(), String> {
    let paths = pick_files(app.clone()).await?;
    board::add_files(&app, paths, None);
    Ok(())
}

#[tauri::command]
fn board_update(app: AppHandle, id: String, patch: board::CardPatch) {
    board::update(&app, &id, patch);
}

#[tauri::command]
fn board_remove(app: AppHandle, id: String) {
    board::remove(&app, &id);
}

#[tauri::command]
fn board_clear(app: AppHandle) {
    board::clear(&app);
}

#[tauri::command]
fn board_use(app: AppHandle, id: String, paste: bool) -> Result<(), String> {
    board::use_card(&app, &id, paste)
}

#[tauri::command]
fn board_drag(app: AppHandle, id: String) -> Result<(), String> {
    board::drag(&app, &id)
}

#[tauri::command]
fn board_open(app: AppHandle, id: String) -> Result<(), String> {
    board::open(&app, &id)
}

#[tauri::command]
fn board_reveal(app: AppHandle, id: String) -> Result<(), String> {
    board::reveal(&app, &id)
}

#[tauri::command]
fn board_send(app: AppHandle, id: String, to: String) -> Result<(), String> {
    match to.as_str() {
        "fileTray" => board::to_tray(&app, &id),
        "clipboard" => board::to_clipboard(&app, &id),
        _ => Err(format!("Unknown place {to}")),
    }
}

#[tauri::command]
fn board_duplicate(app: AppHandle, id: String) -> Result<(), String> {
    board::duplicate(&app, &id)
}

// ----- file tray + note -----

#[tauri::command]
fn tray_list(app: AppHandle) -> Vec<tray::TrayFile> {
    tray::list(&app)
}

#[tauri::command]
fn tray_add(app: AppHandle, paths: Vec<String>) {
    tray::add(&app, paths);
}

#[tauri::command]
async fn tray_pick_files(app: AppHandle) -> Result<(), String> {
    let paths = pick_files(app.clone()).await?;
    tray::add(&app, paths);
    Ok(())
}

#[tauri::command]
fn tray_remove(app: AppHandle, id: String) {
    tray::remove(&app, &id);
}

#[tauri::command]
fn tray_clear(app: AppHandle) {
    tray::clear(&app);
}

#[tauri::command]
fn tray_open(app: AppHandle, id: String) -> Result<(), String> {
    tray::open(&app, &id)
}

#[tauri::command]
fn tray_reveal(app: AppHandle, id: String) -> Result<(), String> {
    tray::reveal(&app, &id)
}

#[tauri::command]
fn tray_copy(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    tray::copy(&app, ids)
}

#[tauri::command]
fn tray_drag(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    tray::drag(&app, ids)
}

#[tauri::command]
fn tray_paste(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    tray::paste(&app, ids)
}

#[tauri::command]
fn tray_to_board(app: AppHandle, ids: Vec<String>) {
    tray::to_board(&app, ids);
}

#[tauri::command]
fn tray_add_clipboard(app: AppHandle) -> Result<(), String> {
    tray::add_clipboard(&app)
}

#[tauri::command]
fn note_get() -> String {
    tray::note_get()
}

#[tauri::command]
fn note_set(app: AppHandle, text: String) -> Result<(), String> {
    tray::note_set(&app, &text)
}

/// Native "open files" dialog (on a worker thread; it blocks until closed).
async fn pick_files(app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title("Add files to Atoll");
        if let Some(win) = app.get_webview_window(ISLAND) {
            dialog = dialog.set_parent(&win);
        }
        dialog
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| f.into_path().ok())
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Settings → Alerts: hear a sound at a volume.
#[tauri::command]
fn preview_sound(app: AppHandle, sound: String, volume: u32) {
    alerts::play(&app, &sound, volume);
}

#[tauri::command]
async fn export_settings(app: AppHandle) -> Result<bool, String> {
    data::export_settings(&app).await
}

#[tauri::command]
async fn import_settings(app: AppHandle) -> Result<bool, String> {
    data::import_settings(&app).await
}

#[tauri::command]
fn clear_all_data(app: AppHandle) -> Result<(), String> {
    data::clear_all_data(&app)
}

#[tauri::command]
async fn test_connection() -> Result<String, String> {
    data::test_connection().await
}

#[tauri::command]
fn claude_status(app: AppHandle) -> data::ClaudeStatus {
    data::claude_status(&app)
}

#[tauri::command]
fn open_folder(app: AppHandle, which: String) -> Result<(), String> {
    system::open_folder(&app, &which)
}

#[tauri::command]
fn clipboard_thumb(app: AppHandle, id: String) -> Result<String, String> {
    clipboard::thumb_data(&app, &id)
}

#[tauri::command]
fn hooks_status() -> hooks::HookStatus {
    hooks::status()
}

#[tauri::command]
fn install_hooks() -> Result<hooks::HookStatus, String> {
    hooks::install()
}

#[tauri::command]
fn uninstall_hooks() -> Result<hooks::HookStatus, String> {
    hooks::uninstall()
}

// ---------- background helpers ----------

/// The window is bigger than the island so it can animate open. Transparent
/// parts must not swallow clicks, so we poll the cursor and only accept mouse
/// input while it is over the island's rectangle.
fn start_click_through(app: AppHandle) {
    std::thread::spawn(move || {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut interactive: Option<bool> = None;
        // Window position and scale only change when the island is placed;
        // re-read them once a second instead of on every poll (each read is a
        // round trip to the main thread).
        let mut placed: Option<(tauri::PhysicalPosition<i32>, f64)> = None;
        let mut island_hwnd: isize = 0;
        let mut ticks: u32 = 0;
        loop {
            // ~60 Hz: hover feels instant. Each poll is one GetCursorPos plus a
            // mutex read, so this costs next to nothing.
            std::thread::sleep(Duration::from_millis(16));
            ticks = ticks.wrapping_add(1);
            let Some(win) = app.get_webview_window(ISLAND) else { continue };
            if placed.is_none() || ticks % 60 == 0 {
                if let (Ok(pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) {
                    placed = Some((pos, scale));
                }
            }
            let Some((pos, dpi)) = placed else { continue };
            // CSS px -> physical px: monitor scale times the island's zoom.
            let zoom = app.state::<AppState>().settings.lock().unwrap().appearance.scale as f64 / 100.0;
            let scale = dpi * zoom;
            let mut p = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut p) } == 0 {
                continue;
            }
            let cursor = tauri::PhysicalPosition::new(p.x as f64, p.y as f64);
            // adaptive-island: the island's rectangle at this moment, from its
            // current move (shape.rs); the last reported one until the page sends a move.
            let r = match shape::hit_rect(shape::now_ms(), 16.0) {
                Some(r) => HitRect { x: r.x, y: r.y, w: r.w, h: r.h },
                None => *app.state::<AppState>().hit_rect.lock().unwrap(),
            };
            let (left, top) = (pos.x as f64 + r.x * scale, pos.y as f64 + r.y * scale);
            if island_hwnd == 0 {
                island_hwnd = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
            }
            let inside = cursor.x >= left
                && cursor.x <= left + r.w * scale
                && cursor.y >= top
                && cursor.y <= top + r.h * scale
                // Hidden, or under the Snipping Tool overlay: not a hover.
                && system::can_hover(island_hwnd, (p.x, p.y));
            if interactive != Some(inside) {
                if inside {
                    // The app you were in is where "click to paste" pastes.
                    paste::remember_foreground();
                }
                let _ = win.set_ignore_cursor_events(!inside);
                // While ignoring cursor events the page gets no mouseleave,
                // so tell the UI about hover changes ourselves.
                let _ = app.emit_to(ISLAND, "island-hover", inside);
                interactive = Some(inside);
            }
        }
    });
}

/// Writes settings to disk at most every 400 ms (sliders send many updates).
fn start_settings_saver(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(400));
        save_if_dirty(&app);
    });
}

fn save_if_dirty(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.settings_dirty.swap(false, Ordering::Relaxed) {
        let snapshot = state.settings.lock().unwrap().clone();
        if let Err(e) = settings::save(&snapshot) {
            eprintln!("[crest] could not save settings: {e}");
            state.settings_dirty.store(true, Ordering::Relaxed);
        }
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show or hide the island", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    // Text and enabled state follow the timer (focus::update_tray).
    let focus_item = MenuItem::with_id(app, "focus", "Start focus", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Atoll", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &settings, &focus_item, &sep, &quit])?;
    // DEV ONLY: lets the island into screenshots (liquid glass freezes meanwhile).
    // Debug builds only, so release never has it. Remove when the project is done.
    if cfg!(debug_assertions) {
        let shots = tauri::menu::CheckMenuItem::with_id(app, "dev_screenshots", "Allow screenshots (dev)", true, false, None::<&str>)?;
        menu.insert(&shots, 3)?;
    }
    focus::set_tray_item(app.handle(), focus_item);

    let mut tray = TrayIconBuilder::with_id(system::TRAY_ID)
        .tooltip("Atoll")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "toggle" => windows::toggle_island(app),
            "settings" => windows::open_dashboard(app),
            "focus" => focus::toggle(app),
            "dev_screenshots" => backdrop::toggle_screenshots(),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left click opens the settings dashboard; right click shows the menu.
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                windows::open_dashboard(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    // Run by the Windows uninstaller: take Atoll's hooks and status line out of Claude Code's
    // settings, then quit without starting the app.
    if std::env::args().any(|a| a == "--uninstall-cleanup") {
        if let Err(e) = hooks::uninstall() {
            eprintln!("[crest] uninstall cleanup failed: {e}");
        }
        return;
    }
    system::redirect_logs();
    let loaded = settings::load();

    let app = tauri::Builder::default()
        // Must be first: a second launch just opens the dashboard of the running one.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| windows::open_dashboard(app)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(shortcuts::on_shortcut).build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(AppState {
            sessions: Mutex::new(SessionStore::default()),
            permissions: Mutex::new(PermissionQueue::default()),
            settings: Mutex::new(loaded),
            settings_dirty: AtomicBool::new(false),
            hit_rect: Mutex::new(HitRect::default()),
            port: AtomicU16::new(0),
            last_hook_ms: std::sync::atomic::AtomicU64::new(0),
        })
        .manage(Shortcuts::default())
        .manage(alerts::AlertState::default())
        .manage(usage::UsageState::default())
        .manage(chatwatch::ChatWatchState::default())
        .manage(spotify::SpotifyState::default())
        .manage(calendar::CalendarState::default())
        .manage(lyrics::LyricsCache::default())
        .manage(clipboard::ClipboardState::default())
        .manage(search::SearchState::default()) // search-clipboard
        .manage(board::BoardState::default())
        .manage(tray::TrayState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_sessions,
            get_agent_feed,
            quit_app,
            dismiss_session,
            clear_session,
            reply_in_claude,
            get_permissions,
            decide_permission,
            set_hit_rect,
            backdrop::set_backdrop,
            // adaptive-island: begin
            shape::island_clock,
            shape::set_island_motion,
            shape::island_metrics,
            arbiter::arbiter_post,
            // adaptive-island: end
            get_settings,
            update_settings,
            reset_settings,
            get_hotkey_status,
            validate_hotkey,
            list_monitors,
            open_dashboard,
            get_island_visible,
            set_island_visible,
            autostart_enabled,
            app_info,
            test_alert,
            get_usage,
            get_chatwatch_status,
            calendar_status,
            calendar_agenda,
            calendar_set_secret,
            calendar_login,
            calendar_cancel_login,
            calendar_logout,
            calendar_refresh,
            calendar_open,
            spotify_status,
            spotify_login,
            spotify_logout,
            spotify_open_app,
            spotify_player,
            spotify_action,
            spotify_devices,
            spotify_app_volume,
            get_lyrics,
            focus_get,
            focus_action,
            open_windows_dnd,
            clipboard_list,
            clipboard_use,
            clipboard_pin,
            clipboard_delete,
            clipboard_clear,
            clipboard_drag,
            clipboard_send,
            clipboard_open_file,
            drop_commit,
            board_reveal,
            board_send,
            board_duplicate,
            tray_paste,
            tray_to_board,
            tray_add_clipboard,
            board_get,
            board_add_files,
            board_add_text,
            board_add_clipboard,
            board_pick_files,
            board_update,
            board_remove,
            board_clear,
            board_use,
            board_drag,
            board_open,
            tray_list,
            tray_add,
            tray_pick_files,
            tray_remove,
            tray_clear,
            tray_open,
            tray_reveal,
            tray_copy,
            tray_drag,
            note_get,
            note_set,
            preview_sound,
            export_settings,
            import_settings,
            clear_all_data,
            test_connection,
            claude_status,
            open_folder,
            clipboard_thumb,
            // ---- search-clipboard: begin ----
            clipboard_transform,
            search_query,
            search_act,
            search_stats,
            palette_ready,
            palette_hide,
            // ---- search-clipboard: end ----
            pause_hotkeys,
            #[cfg(debug_assertions)]
            debug_renew_sign_in,
            hooks_status,
            install_hooks,
            uninstall_hooks,
        ])
        .on_window_event(|window, event| {
            // Closing the dashboard ends any "preview expanded" on the island.
            if window.label() == DASHBOARD && matches!(event, WindowEvent::Destroyed) {
                let _ = window.app_handle().emit_to(ISLAND, "preview-expand", false);
                // A hotkey recording cut short by closing the window must not leave hotkeys off.
                shortcuts::pause(window.app_handle(), false);
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let port = server::start(handle.clone())?;
            // Replies from the island hand the keyboard back to your last app.
            reply::track_your_app();
            app.state::<AppState>().port.store(port, Ordering::Relaxed);
            println!("[crest] listening on 127.0.0.1:{port}");

            let current = app.state::<AppState>().settings.lock().unwrap().clone();
            windows::place_island(&handle, &current.appearance);
            shortcuts::apply(&handle, &current.hotkeys);
            sync_autostart(&handle, current.general.start_with_windows);

            let island = app.get_webview_window(ISLAND).expect("island window is defined in tauri.conf.json");
            island.set_ignore_cursor_events(true)?;
            island.show()?;
            start_click_through(handle.clone());
            dnd::install(&handle);
            backdrop::start(handle.clone());
            alerts::register_toast_identity();
            chatwatch::start(handle.clone());
            spotify::start(handle.clone());
            spotify::load_user(handle.clone());
            clipboard::start(handle.clone());
            focus::start(handle.clone());
            calendar::start(handle.clone());
            search::start(handle.clone()); // search-clipboard
            system::start_fullscreen_watch(handle.clone());
            {
                // A usage limit hit in a desktop session before Atoll started.
                let h = handle.clone();
                tauri::async_runtime::spawn_blocking(move || usage::scan_recent(&h));
            }
            usage::start_sign_in_checks(handle.clone());
            start_settings_saver(handle);
            build_tray(app)?;
            system::update_tray(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Crest");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            save_if_dirty(app);
            server::stop();
        }
    });
}
