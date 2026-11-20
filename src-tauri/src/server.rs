//! Local HTTP listener that receives hook events from `atoll-bridge.exe`.
//!
//! Security:
//! - Binds to 127.0.0.1 only, on a random free port.
//! - Every request must carry `X-Crest-Token` equal to a 256-bit random token
//!   generated at launch. Port + token are written to `%USERPROFILE%\.atoll\runtime.json`
//!   (inside the user's own profile, outside AppData - see `paths::shared_dir`)
//!   for the bridge to read.
//! - Requests with an `Origin` header are rejected, so a web page in a browser
//!   cannot talk to the listener even if it guessed the port.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::alerts::{self, Alert, Kind};
use crate::permissions::{self, Choice};
use crate::sessions::{describe_tool, shorten};
use crate::{paths, AppState};

#[derive(Clone)]
struct Ctx {
    token: Arc<String>,
    app: AppHandle,
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random number generator unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time comparison so the token can't be guessed byte by byte.
fn token_matches(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Bind the listener, publish port + token, and serve in the background.
pub fn start(app: AppHandle) -> std::io::Result<u16> {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    std_listener.set_nonblocking(true)?;
    let port = std_listener.local_addr()?.port();
    let token = random_token();

    std::fs::create_dir_all(paths::shared_dir())?;
    let runtime = json!({ "port": port, "token": token, "pid": std::process::id() });
    std::fs::write(paths::runtime_file(), runtime.to_string())?;
    // An old-location file would only point a stale bridge at a dead port.
    let _ = std::fs::remove_file(paths::legacy_runtime_file());

    let ctx = Ctx { token: Arc::new(token), app };
    let router = Router::new()
        .route("/hook", post(handle_hook))
        .route("/statusline", post(handle_statusline))
        .route("/ping", post(handle_ping))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(ctx);

    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(std_listener) {
            Ok(listener) => {
                if let Err(e) = axum::serve(listener, router).await {
                    eprintln!("[crest] listener stopped: {e}");
                }
            }
            Err(e) => eprintln!("[crest] could not start listener: {e}"),
        }
    });
    Ok(port)
}

/// Remove the runtime file so the bridge stops trying to connect.
pub fn stop() {
    let _ = std::fs::remove_file(paths::runtime_file());
}

/// Shared checks for every route: no browser origin, valid token, JSON body.
fn authorize(ctx: &Ctx, headers: &HeaderMap, body: &Bytes) -> Result<Value, Response> {
    if headers.contains_key("origin") {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let given = headers.get("x-crest-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !token_matches(given, &ctx.token) {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }
    serde_json::from_slice::<Value>(body).map_err(|_| StatusCode::BAD_REQUEST.into_response())
}

/// Connection test from `atoll-bridge ping` (token-checked like every route).
async fn handle_ping(State(ctx): State<Ctx>, headers: HeaderMap, body: Bytes) -> Response {
    match authorize(&ctx, &headers, &body) {
        Ok(_) => Json(json!({ "ok": true, "app": "atoll" })).into_response(),
        Err(r) => r,
    }
}

/// Usage data from `atoll-bridge statusline`.
async fn handle_statusline(State(ctx): State<Ctx>, headers: HeaderMap, body: Bytes) -> Response {
    let payload = match authorize(&ctx, &headers, &body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let enabled = {
        let s = ctx.app.state::<AppState>().settings.lock().unwrap().clone();
        s.modules.claude && s.modules.usage_ring
    };
    if enabled {
        crate::usage::update(&ctx.app, &payload);
    }
    Json(json!({})).into_response()
}

/// Serve only `/spotify/callback` on the fixed sign-in port (127.0.0.1) until
/// the sign-in finishes, `stop` fires, or 10 minutes pass.
pub fn serve_spotify_callback(app: AppHandle, listener: std::net::TcpListener, stop: Arc<tokio::sync::Notify>) {
    let router = Router::new()
        .route("/spotify/callback", axum::routing::get(handle_spotify_callback))
        .with_state((app.clone(), stop.clone()));
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[crest] spotify callback listener: {e}");
                crate::spotify::callback_listener_ended(&app, &stop);
                return;
            }
        };
        let wait = stop.clone();
        let shutdown = async move {
            tokio::select! {
                _ = wait.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {}
            }
        };
        if let Err(e) = axum::serve(listener, router).with_graceful_shutdown(shutdown).await {
            eprintln!("[crest] spotify callback listener stopped: {e}");
        }
        crate::spotify::callback_listener_ended(&app, &stop);
    });
}

/// Spotify redirects the browser here after sign-in. No token (the browser
/// can't have it): instead the single-use random `state` from our own login
/// request must match, so nothing else can complete or forge a sign-in.
async fn handle_spotify_callback(
    State((app, stop)): State<(AppHandle, Arc<tokio::sync::Notify>)>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let result = crate::spotify::finish_login(&app, q.get("code").cloned(), q.get("state").cloned(), q.get("error").cloned()).await;
    // One attempt per sign-in: close the port once this response is sent.
    stop.notify_one();
    let (title, text) = match &result {
        Ok(()) => ("Connected", "Atoll is connected to Spotify. You can close this tab.".to_string()),
        Err(e) => ("Not connected", format!("Atoll could not connect to Spotify: {e}")),
    };
    let text = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Atoll · {title}</title>\
         <body style=\"font-family:Segoe UI,system-ui,sans-serif;background:#0b0c10;color:#e8eaf0;display:grid;place-items:center;height:100vh;margin:0\">\
         <div style=\"max-width:460px;text-align:center\"><h1 style=\"font-weight:600\">{title}</h1><p style=\"color:#9aa0ad;line-height:1.5\">{text}</p></div>"
    );
    axum::response::Html(page).into_response()
}

/// Google sign-in for the calendar: same shape as Spotify's, on a port
/// Windows picked (Google accepts any loopback port for desktop clients).
pub fn serve_google_callback(app: AppHandle, listener: std::net::TcpListener, stop: Arc<tokio::sync::Notify>) {
    let router = Router::new()
        .route("/google/callback", axum::routing::get(handle_google_callback))
        .with_state((app.clone(), stop.clone()));
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[crest] google callback listener: {e}");
                crate::calendar::callback_listener_ended(&app, &stop);
                return;
            }
        };
        let wait = stop.clone();
        let shutdown = async move {
            tokio::select! {
                _ = wait.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {}
            }
        };
        if let Err(e) = axum::serve(listener, router).with_graceful_shutdown(shutdown).await {
            eprintln!("[crest] google callback listener stopped: {e}");
        }
        crate::calendar::callback_listener_ended(&app, &stop);
    });
}

/// Google redirects the browser here after sign-in; the single-use `state`
/// from our own request must match (see handle_spotify_callback).
async fn handle_google_callback(
    State((app, stop)): State<(AppHandle, Arc<tokio::sync::Notify>)>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let result = crate::calendar::finish_login(&app, q.get("code").cloned(), q.get("state").cloned(), q.get("error").cloned()).await;
    stop.notify_one();
    let (title, text) = match &result {
        Ok(()) => ("Connected", "Atoll is connected to Google Calendar. You can close this tab.".to_string()),
        Err(e) => ("Not connected", format!("Atoll could not connect to Google Calendar: {e}")),
    };
    callback_page(title, &text)
}

fn callback_page(title: &str, text: &str) -> Response {
    let text = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Atoll · {title}</title>         <body style=\"font-family:Segoe UI,system-ui,sans-serif;background:#0b0c10;color:#e8eaf0;display:grid;place-items:center;height:100vh;margin:0\">         <div style=\"max-width:460px;text-align:center\"><h1 style=\"font-weight:600\">{title}</h1><p style=\"color:#9aa0ad;line-height:1.5\">{text}</p></div>"
    );
    axum::response::Html(page).into_response()
}

/// Chime / toast / flash for the events that matter.
fn trigger_alerts(app: &AppHandle, event: &Value, name: &str, session_id: &str) {
    let s = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let project = app.state::<AppState>().sessions.lock().unwrap().project_of(session_id);
    let project = if project.is_empty() { "Claude Code".to_string() } else { project };
    let from_desktop = s("crest_entrypoint").contains("desktop");

    let alert = match name {
        "Stop" => {
            let last = s("last_assistant_message");
            Some((Kind::Done, format!("{project} is done"), if last.is_empty() { "Finished.".into() } else { shorten(&last, 140) }))
        }
        "PermissionRequest" => {
            let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
            let a = describe_tool(&s("tool_name"), &input);
            Some((Kind::Attention, format!("{project} needs your permission"), format!("{} {}", a.label, a.detail)))
        }
        // idle_prompt repeats a finished turn we already announced.
        "Notification" if s("notification_type") != "idle_prompt" => {
            Some((Kind::Attention, format!("{project} needs you"), shorten(&s("message"), 140)))
        }
        "StopFailure" => {
            let detail = if s("error_details").is_empty() { s("error") } else { s("error_details") };
            Some((Kind::Error, format!("{project} stopped"), shorten(&detail, 140)))
        }
        _ => None,
    };
    if let Some((kind, title, body)) = alert {
        if from_desktop {
            // Lets the Chat tab watcher ignore signals caused by Code tab sessions.
            crate::chatwatch::note_desktop_activity(app);
        }
        alerts::fire(app, Alert { kind, session_id: Some(session_id.to_string()), title, body }, false);
    }
}

async fn handle_hook(State(ctx): State<Ctx>, headers: HeaderMap, body: Bytes) -> Response {
    let event = match authorize(&ctx, &headers, &body) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let state = ctx.app.state::<AppState>();
    state.last_hook_ms.store(crate::sessions::now_ms(), std::sync::atomic::Ordering::Relaxed);
    if !state.enabled() {
        // Following Claude Code is switched off: no opinion on anything.
        return Json(json!({})).into_response();
    }

    let name = event.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    let session_id = event.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();

    if state.sessions.lock().unwrap().apply(&event) {
        crate::emit_sessions(&ctx.app);
    }
    trigger_alerts(&ctx.app, &event, name, &session_id);

    // Desktop sessions don't run the status line: their log says when a limit is hit.
    if matches!(name, "Stop" | "StopFailure" | "SessionStart") {
        crate::usage::poke(&ctx.app);
        if let Some(path) = event.get("transcript_path").and_then(Value::as_str).map(String::from) {
            let app = ctx.app.clone();
            tauri::async_runtime::spawn(async move {
                // The failing reply may reach the log just after the hook fires.
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                let _ = tauri::async_runtime::spawn_blocking(move || crate::usage::check_transcript(&app, &path)).await;
            });
        }
    }

    // The turn moved on, so any open request of this session was answered elsewhere.
    // (Not PostToolUse: parallel tool calls can finish while another one waits.)
    if matches!(name, "Stop" | "UserPromptSubmit" | "SessionEnd")
        && state.permissions.lock().unwrap().pass_session(&session_id)
    {
        crate::emit_permissions(&ctx.app);
    }

    if name == "PermissionRequest" {
        // "Ask before allowing actions" off: Claude Code asks in its own window.
        // Claude's multiple-choice questions still come to the island: they are
        // questions for you, not safety prompts.
        let tool = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
        if !state.settings.lock().unwrap().claude.ask_permissions && !permissions::tool_is_question(tool) {
            return Json(json!({})).into_response();
        }
        let choice = wait_for_choice(&ctx.app, &event, &session_id).await;
        let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
        return Json(choice.to_hook_output(&input)).into_response();
    }

    // `{}` = "no opinion", Claude Code continues normally.
    Json(json!({})).into_response()
}

/// Removes the request from the island if the hook call goes away early
/// (bridge killed / Claude Code cancelled the hook -> axum drops this future).
struct PendingGuard {
    app: AppHandle,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self.app.state::<AppState>().permissions.lock().unwrap().remove(self.id) {
            crate::emit_permissions(&self.app);
        }
    }
}

/// Queue the request, show it in the island, and wait for a click (or give up after WAIT).
async fn wait_for_choice(app: &AppHandle, event: &Value, session_id: &str) -> Choice {
    let state = app.state::<AppState>();
    let project = state.sessions.lock().unwrap().project_of(session_id);
    // ---- permission-intel: begin ----
    // The session's starting folder is the project (the event's cwd follows every `cd`).
    let project_dir = state.sessions.lock().unwrap().snapshot().into_iter().find(|s| s.id == session_id).map(|s| s.cwd).unwrap_or_default();
    let intel = permissions::analyze(event, project_dir.clone()).await;
    // Rules: a rule you armed may allow it without asking (never a risky or unchecked
    // request, never wider than its dry run). The decision is logged; the island hears it.
    let (facts, rule) = crate::rules::check(event, &intel, &project_dir);
    if let Some(auto) = rule.auto {
        println!("[crest] permission for {session_id}: allowed by rule {:?}", auto.rule_name);
        use tauri::Emitter;
        let _ = app.emit("permissions:auto-allowed", &auto);
        crate::rules::commands::changed(app);
        if state.sessions.lock().unwrap().resume(session_id) {
            crate::emit_sessions(app);
        }
        return Choice::Allow;
    }
    let (id, rx) = state.permissions.lock().unwrap().push(event, project, Some(intel));
    state.permissions.lock().unwrap().set_rules(id, facts.clone(), rule.hint);
    // ---- permission-intel: end ----
    let _guard = PendingGuard { app: app.clone(), id };
    crate::emit_permissions(app);

    let choice = match tokio::time::timeout(permissions::WAIT, rx).await {
        Ok(Ok(choice)) => choice,
        _ => Choice::Pass, // timed out or dropped: let Claude Code ask in the terminal
    };
    crate::rules::record_answer(&facts, &choice); // permission-intel: kinds only, for dry runs
    println!("[crest] permission #{id} for {session_id}: answering {choice:?}");
    if choice != Choice::Pass && state.sessions.lock().unwrap().resume(session_id) {
        crate::emit_sessions(app);
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("ab", "abc"));
        assert_eq!(random_token().len(), 64);
    }
}
