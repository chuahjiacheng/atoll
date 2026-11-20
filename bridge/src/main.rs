//! crest-bridge: the command Claude Code runs for each hook event.
//!
//! Flow: Claude Code writes the hook event JSON to our stdin -> we POST it to
//! the island's listener on 127.0.0.1 (port + token read from the runtime file
//! the island writes on start) -> if the island answers with a decision JSON we
//! print it to stdout for Claude Code to apply.
//!
//! Fail-open by design: if the island is not running, unreachable, or slow, we
//! print nothing and exit 0, so Claude Code carries on with its normal flow
//! (including its own terminal permission prompt).

// Console subsystem on purpose: Claude Code spawns hooks with hidden windows,
// and a GUI-subsystem exe did not reliably receive piped stdin.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

/// Longest string kept in forwarded payloads; the island only shows summaries.
const MAX_STRING: usize = 4000;

fn main() {
    // `crest-bridge statusline` = Claude Code status line command (usage ring).
    if std::env::args().nth(1).as_deref() == Some("statusline") {
        statusline();
        return;
    }
    // `crest-bridge ping` = Settings → AI agents → Test connection. Exit code 0
    // means the island answered through the same path the hooks use.
    if std::env::args().nth(1).as_deref() == Some("ping") {
        std::process::exit(match ping() {
            Ok(()) => 0,
            Err(why) => {
                eprintln!("{why}");
                1
            }
        });
    }
    // Any failure means "no opinion": exit 0 with no output.
    // Set CREST_DEBUG=1 to see why on stderr (stderr is ignored by Claude Code on exit 0).
    if let Err(why) = run() {
        if std::env::var_os("CREST_DEBUG").is_some() {
            eprintln!("crest-bridge: {why}");
        }
    }
}

fn run() -> Result<(), String> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).map_err(|e| format!("stdin: {e}"))?;
    let mut event: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("event is not JSON ({e}): {:.80}", raw))?;

    let (port, token) = read_runtime_file().ok_or("island not running (no runtime.json)")?;

    // Tag where the session runs (terminal CLI vs Claude desktop Code tab).
    if let Some(obj) = event.as_object_mut() {
        obj.remove("tool_response"); // can be huge, island doesn't need it
        let entry = std::env::var("CLAUDE_CODE_ENTRYPOINT").unwrap_or_default();
        obj.insert("crest_entrypoint".into(), Value::String(entry));
    }
    trim_strings(&mut event);

    // Permission requests wait for a human click; everything else is a quick
    // fire-and-forget notification.
    let is_permission = event.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest");
    let timeout = if is_permission { Duration::from_secs(590) } else { Duration::from_secs(2) };

    let url = format!("http://127.0.0.1:{port}/hook");
    let response = ureq::post(&url)
        .timeout(timeout)
        .set("Content-Type", "application/json")
        .set("X-Crest-Token", &token)
        .send_string(&event.to_string())
        .map_err(|e| format!("POST {url}: {e}"))?;

    let body = response.into_string().map_err(|e| format!("response: {e}"))?;
    let reply: Value = serde_json::from_str(body.trim()).map_err(|e| format!("reply not JSON: {e}"))?;
    // An empty object means "no decision": print nothing.
    if reply.as_object().is_some_and(|o| !o.is_empty()) {
        let mut out = std::io::stdout();
        out.write_all(reply.to_string().as_bytes()).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn ping() -> Result<(), String> {
    let (port, token) = read_runtime_file().ok_or("Atoll's runtime file is missing (is Atoll running?)")?;
    let url = format!("http://127.0.0.1:{port}/ping");
    ureq::post(&url)
        .timeout(Duration::from_secs(3))
        .set("Content-Type", "application/json")
        .set("X-Crest-Token", &token)
        .send_string("{}")
        .map_err(|e| format!("Atoll did not answer on 127.0.0.1:{port}: {e}"))?;
    Ok(())
}

/// Status line mode (https://code.claude.com/docs/en/statusline): Claude Code
/// sends session JSON on stdin and shows whatever we print. We forward the
/// usage part to the island (best effort, short timeout) and print a compact
/// line, e.g. `Opus · ctx 34% · 5h 23% · wk 41%`, which works even when the
/// island is closed.
fn statusline() {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let Ok(data) = serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')) else {
        println!("Atoll");
        return;
    };

    if let Some((port, token)) = read_runtime_file() {
        let payload = serde_json::json!({
            "session_id": data.get("session_id"),
            "rate_limits": data.get("rate_limits"),
        });
        let result = ureq::post(&format!("http://127.0.0.1:{port}/statusline"))
            .timeout(Duration::from_millis(800))
            .set("Content-Type", "application/json")
            .set("X-Crest-Token", &token)
            .send_string(&payload.to_string());
        if let Err(e) = result {
            if std::env::var_os("CREST_DEBUG").is_some() {
                eprintln!("crest-bridge statusline: {e}");
            }
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = data.pointer("/model/display_name").and_then(Value::as_str) {
        parts.push(model.to_string());
    }
    if let Some(ctx) = data.pointer("/context_window/used_percentage").and_then(Value::as_f64) {
        parts.push(format!("ctx {ctx:.0}%"));
    }
    if let Some(p) = data.pointer("/rate_limits/five_hour/used_percentage").and_then(Value::as_f64) {
        parts.push(format!("5h {p:.0}%"));
    }
    if let Some(p) = data.pointer("/rate_limits/seven_day/used_percentage").and_then(Value::as_f64) {
        parts.push(format!("wk {p:.0}%"));
    }
    println!("{}", if parts.is_empty() { "Atoll".to_string() } else { parts.join(" · ") });
}

/// The island writes `%USERPROFILE%\.atoll\runtime.json` = {"port":..,"token":".."}.
/// Not AppData: inside the Claude desktop app (an MSIX package) AppData is a
/// private copy, so a file there can be a stale one the island never sees.
/// `%LOCALAPPDATA%\Crest\runtime.json` is only read for islands up to 1.0.2.
fn read_runtime_file() -> Option<(u16, String)> {
    let current = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".atoll").join("runtime.json"));
    let legacy = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Crest").join("runtime.json"));
    let text = [current, legacy].into_iter().flatten().find_map(|p| std::fs::read_to_string(p).ok())?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
    let token = v.get("token")?.as_str()?.to_string();
    Some((port, token))
}

/// Shorten very long strings (e.g. full file contents in a Write tool call).
fn trim_strings(v: &mut Value) {
    match v {
        Value::String(s) if s.len() > MAX_STRING => {
            let mut cut = MAX_STRING;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
            s.push_str("…");
        }
        Value::Array(items) => items.iter_mut().for_each(trim_strings),
        Value::Object(map) => map.values_mut().for_each(trim_strings),
        _ => {}
    }
}
