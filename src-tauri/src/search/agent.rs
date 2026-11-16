//! Agent history for search, read from the flight recorder's event store.
//!
//! STUB until the `recorder-forecast` branch is merged. It codes against
//! docs/contracts/event-store.md v1.1:
//!   - `Store::events(&EventQuery { since_ms, limit, .. })` (oldest first)
//!   - `Store::on_change(f)` behind the `recorder:changed` frontend event
//! When `recorder` lands, replace `fetch_since` with
//! `app.state::<crate::recorder::Store>().events(&EventQuery { since_ms: Some(since), limit: Some(5_000), ..Default::default() })`
//! mapped through `AgentEvent::from(row)`; nothing else here changes.

use tauri::AppHandle;

/// The fields of an event-store `EventRow` that search uses.
#[derive(Clone, Debug)]
pub struct AgentEvent {
    pub id: i64,
    pub ts: i64,
    pub session_id: String,
    pub kind: String,
    pub tool: Option<String>,
    /// One line, already redacted by the recorder.
    pub summary: String,
}

/// Events with `ts >= since_ms`, oldest first, at most `limit`.
/// Stub: the event store isn't in this branch yet, so there are none.
pub fn fetch_since(_app: &AppHandle, _since_ms: i64, _limit: u32) -> Vec<AgentEvent> {
    Vec::new()
}

/// The frontend event the recorder sends after writes (at most 1/s per session).
pub const CHANGED_EVENT: &str = "recorder:changed";
