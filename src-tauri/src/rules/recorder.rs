//! Past permission requests from the flight recorder, for dry runs.
//!
//! Coded against `docs/contracts/event-store.md` (v1.1): the recorder keeps every
//! hook event, redacted, in `%LOCALAPPDATA%\Crest\recorder\events.db`, and other
//! sessions read it only through `recorder::Store`. The recorder is built on the
//! `recorder-forecast` branch but not merged yet, so `past_requests` is a stub that
//! returns nothing. Once it is merged, `past_requests` becomes (contract §4):
//!
//! ```ignore
//! let store = app.state::<crate::recorder::Store>();
//! let rows = store.events(&EventQuery { kinds: Some(vec!["PermissionRequest".into()]), limit: Some(5000), ..Default::default() })?;
//! for row in rows {
//!     let detail = store.event(row.id)?;           // payload = the redacted hook event
//!     let project = /* SessionRow.cwd of row.session_id */;
//!     out.extend(from_payload(row.ts as u64, &detail.payload, &project));
//! }
//! ```

use serde_json::Value;

use super::logic::{facts, Past};

/// Every recorded PermissionRequest, oldest first. Empty until the recorder is merged.
pub fn past_requests() -> Vec<Past> {
    Vec::new()
}

/// One recorded PermissionRequest event (contract: `EventDetail.payload`) as rules see it.
/// Redacted text (`[redacted]`) only changes argument values, never which programs run,
/// so command kinds stay right.
pub fn from_payload(ts: u64, payload: &Value, project_dir: &str) -> Option<Past> {
    if payload.get("hook_event_name").and_then(Value::as_str) != Some("PermissionRequest") {
        return None;
    }
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    let cwd = payload.get("cwd").and_then(Value::as_str).unwrap_or(project_dir);
    let ctx = crest_intel::Context::from_system(project_dir, cwd);
    let analysis = crest_intel::analyze_tool(tool, &input, &ctx, None);
    let f = facts(payload, &analysis, project_dir);
    if f.question {
        return None;
    }
    Some(Past { at: ts, project: f.project, tool: f.tool, keys: f.keys, risk: f.risk })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_a_recorded_request() {
        let ev = json!({"hook_event_name": "PermissionRequest", "session_id": "s", "cwd": "C:/p", "tool_name": "Bash", "tool_input": {"command": "curl -H 'Authorization: [redacted]' https://api.example.com && npm test"}});
        let p = from_payload(42, &ev, "C:/p").unwrap();
        assert_eq!(p.at, 42);
        assert_eq!(p.keys, ["curl -H", "npm test"]);
        assert!(from_payload(1, &json!({"hook_event_name": "PreToolUse"}), "C:/p").is_none());
        assert!(past_requests().is_empty());
    }
}
