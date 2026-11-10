//! Tauri glue for the recorder: app state, start-up, the hooks other files
//! call (one line each), and the `recorder_*` commands (event-store.md §5).

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use super::{AgentNode, Breakdown, EventDetail, EventQuery, EventRow, SessionQuery, SessionRow, Store, StoreStats, UsageSample, Window};
use crate::AppState;

/// `None` if the database could not be opened (Atoll keeps working without it).
pub struct RecorderState(pub Option<Store>);

/// How often old history is cleaned up while Atoll runs.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

fn settings(app: &AppHandle) -> crate::settings::Recorder {
    app.state::<AppState>().settings.lock().unwrap().recorder.clone()
}

fn store(app: &AppHandle) -> Option<Store> {
    app.try_state::<RecorderState>().and_then(|s| s.0.clone())
}

/// Open the database and start the clean-up timer. Called once from setup.
pub fn start(app: &AppHandle) {
    let store = match Store::open(&super::default_path()) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[crest] recorder off: {e}");
            None
        }
    };
    if let Some(s) = &store {
        let h = app.clone();
        s.on_change(move |session| {
            let _ = h.emit("recorder:changed", serde_json::json!({ "sessionId": session }));
        });
        let (s, h) = (s.clone(), app.clone());
        // Sleeps between runs: no idle CPU.
        std::thread::Builder::new()
            .name("recorder-prune".into())
            .spawn(move || loop {
                s.prune(settings(&h).retention_days);
                std::thread::sleep(PRUNE_EVERY);
            })
            .ok();
    }
    app.manage(RecorderState(store));
}

/// From server.rs: one hook event as received.
pub fn record_hook(app: &AppHandle, event: &Value) {
    if let Some(s) = store(app).filter(|_| settings(app).enabled) {
        s.record_hook(event);
    }
}

/// From usage.rs: the current limit windows after an update.
pub fn record_usage(app: &AppHandle, view: &crate::usage::UsageView, source: &str) {
    let Some(s) = store(app).filter(|_| settings(app).enabled) else { return };
    let ts = super::now_ms();
    for (window, w) in [(Window::FiveHour, view.five_hour), (Window::SevenDay, view.seven_day)] {
        if let Some(w) = w {
            s.record_usage(UsageSample { ts, window, used: w.used, resets_at: w.resets_at, source: source.into() });
        }
    }
}

/// From data.rs "Clear all data".
pub fn delete_all(app: &AppHandle) -> Result<(), String> {
    match store(app) {
        Some(s) => s.delete_all(),
        None => Ok(()),
    }
}

/// Settings changed: apply a shorter retention at once.
pub fn settings_changed(app: &AppHandle) {
    if let Some(s) = store(app) {
        s.prune(settings(app).retention_days);
    }
}

async fn read<T: Send + 'static>(app: AppHandle, f: impl FnOnce(&Store) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let s = store(&app).ok_or("History isn't available: the recorder database could not be opened.")?;
    tauri::async_runtime::spawn_blocking(move || f(&s)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn recorder_sessions(app: AppHandle, query: SessionQuery) -> Result<Vec<SessionRow>, String> {
    read(app, move |s| s.sessions(&query)).await
}

#[tauri::command]
pub async fn recorder_events(app: AppHandle, query: EventQuery) -> Result<Vec<EventRow>, String> {
    read(app, move |s| s.events(&query)).await
}

#[tauri::command]
pub async fn recorder_event(app: AppHandle, id: i64) -> Result<Option<EventDetail>, String> {
    read(app, move |s| s.event(id)).await
}

#[tauri::command]
pub async fn recorder_search(app: AppHandle, text: String, limit: Option<u32>) -> Result<Vec<EventRow>, String> {
    read(app, move |s| s.search(&text, limit.unwrap_or(100))).await
}

#[tauri::command]
pub async fn recorder_breakdown(app: AppHandle, session_id: String) -> Result<Breakdown, String> {
    read(app, move |s| s.breakdown(&session_id)).await
}

#[tauri::command]
pub async fn recorder_agent_tree(app: AppHandle, session_id: String) -> Result<Vec<AgentNode>, String> {
    read(app, move |s| s.agent_tree(&session_id)).await
}

#[tauri::command]
pub async fn recorder_usage(app: AppHandle, window: Window, since_ms: i64) -> Result<Vec<UsageSample>, String> {
    read(app, move |s| s.usage_samples(window, since_ms)).await
}

#[tauri::command]
pub async fn recorder_stats(app: AppHandle) -> Result<StoreStats, String> {
    read(app, |s| s.stats()).await
}

#[tauri::command]
pub async fn recorder_delete_all(app: AppHandle) -> Result<(), String> {
    read(app, |s| s.delete_all()).await
}
