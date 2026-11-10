//! Claude usage limits for the usage ring.
//!
//! Source: Claude Code's status line (https://code.claude.com/docs/en/statusline).
//! Claude Code runs our status line command with JSON on stdin that includes
//! `rate_limits.five_hour` / `rate_limits.seven_day` = { used_percentage, resets_at }
//! for Pro/Max/Ultra subscribers, after the first reply of a session.
//! `crest-bridge.exe statusline` forwards that to `POST /statusline`.
//!
//! Second source, for the Claude desktop app (its Code tab doesn't run the
//! status line): each session's log file (the transcript Claude Code writes
//! under ~/.claude/projects) records Claude's answer when a limit is hit, as
//! `apiError: "usage_limit_reached"` with `apiErrorParams.rate_limit_info`
//! { status: "rejected", resetsAt, rateLimitType }. That gives "limit reached,
//! resets at …" (not a running percentage), read after each turn and at start.
//!
//! Third source, the live percentage without a terminal (opt-in, Settings →
//! AI agents → Usage ring): Anthropic's usage numbers, asked for with Claude
//! Code's own sign-in (`claudeAiOauth.accessToken` in ~/.claude/.credentials.json).
//! [Unverified] The address is the one Claude Code's /usage screen and community
//! usage meters use; it is not documented for other apps, so any failure just
//! leaves the ring as it was and says why. The token is read each time and is
//! never kept, logged or sent anywhere but Anthropic.
//!
//! Renewal (the user asked for it): a terminal sign-in lasts about 8 hours and
//! only Claude Code renews it. When it is about to run out, Atoll renews it the
//! same way Claude Code does (the token address and client ID come from Claude
//! Code's own sign-in settings) and writes the new sign-in back into the same
//! file, so the terminal stays signed in too. To avoid two programs renewing at
//! once it re-reads the file first and skips if someone else already renewed,
//! and only writes back if the file still holds the sign-in it renewed.

use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::alerts::{self, Alert, Kind};
use crate::sessions::now_ms;
use crate::AppState;

#[derive(Clone, Copy, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// 0..=100
    pub used: f64,
    /// Unix epoch seconds.
    pub resets_at: u64,
}

#[derive(Clone, Serialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    pub five_hour: Option<LimitWindow>,
    pub seven_day: Option<LimitWindow>,
    /// When Claude Code last sent data (ms since epoch); 0 = never.
    pub updated_at: u64,
    /// Why the sign-in usage check isn't working, if it isn't.
    pub sign_in_error: Option<String>,
}

#[derive(Default)]
pub struct UsageState {
    view: Mutex<UsageView>,
    /// `resets_at` of the 5-hour window we already warned about.
    warned_for: Mutex<Option<u64>>,
    /// Wakes the sign-in check early (a turn finished, or the setting changed).
    poke: tokio::sync::Notify,
}

fn window(v: &Value) -> Option<LimitWindow> {
    Some(LimitWindow {
        used: v.get("used_percentage")?.as_f64()?.clamp(0.0, 100.0),
        resets_at: v.get("resets_at")?.as_u64()?,
    })
}

/// Drop windows whose reset time has passed (they no longer mean anything).
/// `resets_at == 0` means no window is running yet (nothing used).
fn fresh(mut v: UsageView) -> UsageView {
    let now_s = now_ms() / 1000;
    let gone = |w: LimitWindow| w.resets_at != 0 && w.resets_at <= now_s;
    if v.five_hour.is_some_and(gone) {
        v.five_hour = None;
    }
    if v.seven_day.is_some_and(gone) {
        v.seven_day = None;
    }
    v
}

pub fn current(app: &AppHandle) -> UsageView {
    fresh(app.state::<UsageState>().view.lock().unwrap().clone())
}

/// The tail of a session log that can hold its latest limit answer.
const TAIL_BYTES: u64 = 256 * 1024;

/// A limit Claude refused a request for, read from a session log line.
fn limit_hit(line: &str) -> Option<(&'static str, u64)> {
    if !line.contains("rate_limit_info") && !line.contains("quotaLimits") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let info = v.pointer("/apiErrorParams/rate_limit_info").or_else(|| v.get("quotaLimits"))?;
    if info.get("status").and_then(Value::as_str) != Some("rejected") {
        return None;
    }
    let resets_at = info.get("resetsAt").and_then(Value::as_u64)?;
    let window = match info.get("rateLimitType").and_then(Value::as_str)? {
        "five_hour" => "five_hour",
        t if t.starts_with("seven_day") => "seven_day",
        _ => return None,
    };
    Some((window, resets_at))
}

/// The newest limit hit in a log file whose window hasn't reset yet.
fn hits_in(path: &std::path::Path) -> Vec<(&'static str, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)));
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    let now_s = now_ms() / 1000;
    String::from_utf8_lossy(&buf).lines().filter_map(limit_hit).filter(|(_, at)| *at > now_s).collect()
}

/// Mark the windows from these hits as full (100 %) until they reset.
fn apply_hits(app: &AppHandle, hits: &[(&'static str, u64)]) {
    if hits.is_empty() {
        return;
    }
    let state = app.state::<UsageState>();
    let next = {
        let mut view = state.view.lock().unwrap();
        for &(window, resets_at) in hits {
            let w = Some(LimitWindow { used: 100.0, resets_at });
            let slot = if window == "five_hour" { &mut view.five_hour } else { &mut view.seven_day };
            // Keep the latest window (a later reset time is a newer window).
            if slot.is_none_or(|old| old.resets_at <= resets_at) {
                *slot = w;
            }
        }
        view.updated_at = now_ms();
        fresh(view.clone())
    };
    crate::recorder::commands::record_usage(app, &next, "log"); // recorder-forecast
    let _ = app.emit("usage", &next);
    warn(app, &next);
}

fn ring_on(app: &AppHandle) -> bool {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    s.modules.claude && s.modules.usage_ring
}

/// After a turn: look in that session's log for a limit Claude just hit.
pub fn check_transcript(app: &AppHandle, path: &str) {
    if path.is_empty() || !ring_on(app) {
        return;
    }
    apply_hits(app, &hits_in(std::path::Path::new(path)));
}

/// At start: the newest session logs may hold a limit hit before Atoll ran.
pub fn scan_recent(app: &AppHandle) {
    if !ring_on(app) {
        return;
    }
    let root = crate::paths::claude_dir().join("projects");
    let week = std::time::Duration::from_secs(7 * 24 * 3600);
    let mut logs: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|d| std::fs::read_dir(d.path()).ok())
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(t, _)| t.elapsed().map(|age| age < week).unwrap_or(false))
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    let hits: Vec<_> = logs.iter().take(40).flat_map(|(_, p)| hits_in(p)).collect();
    apply_hits(app, &hits);
}

// ---------- live percentage with Claude Code's sign-in ----------

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// How often to ask (a finished turn asks sooner, but never within MIN_GAP).
const EVERY: std::time::Duration = std::time::Duration::from_secs(180);
const MIN_GAP: std::time::Duration = std::time::Duration::from_secs(60);

fn sign_in_on(app: &AppHandle) -> bool {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    s.modules.claude && s.modules.usage_ring && s.usage.from_sign_in
}

const NOT_SIGNED_IN: &str = "Claude Code isn't signed in on this PC. Run claude auth login in a terminal once.";
/// Claude Code's sign-in settings (from its own config: the token address and its public client ID).
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Renew this long before the sign-in runs out.
const RENEW_BEFORE_MS: u64 = 5 * 60 * 1000;

fn credentials_path() -> std::path::PathBuf {
    crate::paths::claude_dir().join(".credentials.json")
}

/// The saved sign-in, read fresh each time (never kept).
#[derive(Clone, PartialEq)]
struct Login {
    access: String,
    refresh: String,
    /// ms since epoch; 0 = unknown.
    expires_at: u64,
    scopes: Vec<String>,
}

fn read_login() -> Result<Login, String> {
    let text = std::fs::read_to_string(credentials_path()).map_err(|_| NOT_SIGNED_IN.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|_| "Couldn't read Claude Code's sign-in file.".to_string())?;
    let o = v.get("claudeAiOauth").ok_or_else(|| NOT_SIGNED_IN.to_string())?;
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let login = Login {
        access: s("accessToken"),
        refresh: s("refreshToken"),
        expires_at: o.get("expiresAt").and_then(Value::as_u64).unwrap_or(0),
        scopes: o
            .get("scopes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
    };
    if login.access.is_empty() && login.refresh.is_empty() {
        return Err(NOT_SIGNED_IN.to_string());
    }
    Ok(login)
}

/// Put renewed tokens into the file's JSON, keeping everything else as it was.
fn with_renewed(file: &Value, reply: &Value, now: u64) -> Option<Value> {
    let mut v = file.clone();
    let o = v.get_mut("claudeAiOauth")?.as_object_mut()?;
    let access = reply.get("access_token")?.as_str()?;
    o.insert("accessToken".into(), Value::from(access));
    if let Some(r) = reply.get("refresh_token").and_then(Value::as_str) {
        o.insert("refreshToken".into(), Value::from(r));
    }
    if let Some(secs) = reply.get("expires_in").and_then(Value::as_u64) {
        o.insert("expiresAt".into(), Value::from(now + secs * 1000));
    }
    if let Some(secs) = reply.get("refresh_token_expires_in").and_then(Value::as_u64) {
        o.insert("refreshTokenExpiresAt".into(), Value::from(now + secs * 1000));
    }
    if let Some(scope) = reply.get("scope").and_then(Value::as_str) {
        o.insert("scopes".into(), Value::from(scope.split_whitespace().collect::<Vec<_>>()));
    }
    Some(v)
}

/// Only one renewal at a time inside Atoll.
static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Renew the sign-in like Claude Code does and save it back. Returns the new access token.
async fn renew(http: &reqwest::Client, used: &Login) -> Result<String, String> {
    let _one = RENEWING.lock().await;
    // Someone (Claude Code, or an earlier call) may have renewed already.
    if let Ok(now) = read_login() {
        if now.refresh != used.refresh && !now.access.is_empty() {
            return Ok(now.access);
        }
    }
    if used.refresh.is_empty() {
        return Err(NOT_SIGNED_IN.to_string());
    }
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": used.refresh,
        "client_id": CLIENT_ID,
        "scope": used.scopes.join(" "),
    });
    let resp = http.post(TOKEN_URL).json(&body).send().await.map_err(|_| "Couldn't reach Anthropic to renew the sign-in.".to_string())?;
    if !resp.status().is_success() {
        return Err(format!(
            "Anthropic wouldn't renew Claude Code's sign-in ({}). Run claude auth login in a terminal once.",
            resp.status().as_u16()
        ));
    }
    let reply: Value = resp.json().await.map_err(|_| "Anthropic's sign-in reply wasn't readable.".to_string())?;
    let access = reply.get("access_token").and_then(Value::as_str).ok_or("Anthropic's sign-in reply had no token.")?.to_string();

    // Save it back only if the file still holds the sign-in we renewed (the
    // old refresh token no longer works, so the terminal needs the new one).
    let path = credentials_path();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let still_ours = file.pointer("/claudeAiOauth/refreshToken").and_then(Value::as_str) == Some(used.refresh.as_str());
    if still_ours {
        if let Some(next) = with_renewed(&file, &reply, now_ms()) {
            let tmp = path.with_extension("json.atoll-tmp");
            std::fs::write(&tmp, serde_json::to_string(&next).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                e.to_string()
            })?;
            eprintln!("[crest] renewed Claude Code's sign-in");
        }
    }
    Ok(access)
}

/// A usable access token: the saved one, renewed first if it's about to run out.
async fn access_token(http: &reqwest::Client) -> Result<(String, Login), String> {
    let login = read_login()?;
    let fresh = !login.access.is_empty() && (login.expires_at == 0 || login.expires_at > now_ms() + RENEW_BEFORE_MS);
    if fresh {
        return Ok((login.access.clone(), login));
    }
    let token = renew(http, &login).await?;
    Ok((token, login))
}

/// "2025-11-07T20:00:00.123+00:00" (or …Z) → Unix seconds.
fn parse_time(s: &str) -> Option<u64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (clock, offset_s) = match rest.find(['Z', '+', '-']) {
        Some(i) => {
            let (c, o) = rest.split_at(i);
            let off = if o.starts_with('Z') {
                0
            } else {
                let sign = if o.starts_with('-') { -1 } else { 1 };
                let mut hm = o[1..].split(':').map(|p| p.parse::<i64>().ok());
                sign * (hm.next()?? * 3600 + hm.next().flatten().unwrap_or(0) * 60)
            };
            (c, off)
        }
        None => (rest, 0),
    };
    let mut t = clock.split(':');
    let (hh, mm) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let ss = t.next().and_then(|x| x.split('.').next()?.parse::<i64>().ok()).unwrap_or(0);
    // Days since 1970-01-01 (the days-from-civil algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + hh * 3600 + mm * 60 + ss - offset_s).ok()
}

/// One window from the reply: { utilization: 0..100, resets_at: ISO time | null }.
fn signed_window(v: &Value) -> Option<LimitWindow> {
    let used = v.get("utilization")?.as_f64()?.clamp(0.0, 100.0);
    let resets_at = v.get("resets_at").and_then(Value::as_str).and_then(parse_time).unwrap_or(0);
    Some(LimitWindow { used, resets_at })
}

fn set_sign_in_error(app: &AppHandle, error: Option<String>) {
    let next = {
        let state = app.state::<UsageState>();
        let mut view = state.view.lock().unwrap();
        if view.sign_in_error == error {
            return;
        }
        view.sign_in_error = error;
        fresh(view.clone())
    };
    let _ = app.emit("usage", &next);
}

/// Ask once. Returns how long to wait before asking again.
async fn check_sign_in(app: &AppHandle, http: &reqwest::Client) -> std::time::Duration {
    let (token, login) = match access_token(http).await {
        Ok(t) => t,
        Err(e) => {
            set_sign_in_error(app, Some(e));
            return EVERY;
        }
    };
    let ask = |token: String| {
        http.get(USAGE_URL)
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header(reqwest::header::USER_AGENT, concat!("Atoll/", env!("CARGO_PKG_VERSION")))
            .send()
    };
    let mut resp = ask(token).await;
    // Refused although it hadn't run out yet: renew once and ask again.
    if resp.as_ref().is_ok_and(|r| r.status().as_u16() == 401) {
        if let Ok(token) = renew(http, &login).await {
            resp = ask(token).await;
        }
    }
    let resp = match resp {
        Ok(r) => r,
        Err(_) => {
            set_sign_in_error(app, Some("Couldn't reach Anthropic to check usage. Trying again in a few minutes.".into()));
            return EVERY;
        }
    };
    match resp.status().as_u16() {
        200 => {
            let Ok(v) = resp.json::<Value>().await else {
                set_sign_in_error(app, Some("Anthropic's usage reply wasn't readable.".into()));
                return EVERY;
            };
            let state = app.state::<UsageState>();
            let next = {
                let mut view = state.view.lock().unwrap();
                view.five_hour = v.get("five_hour").and_then(signed_window);
                view.seven_day = v.get("seven_day").and_then(signed_window);
                view.updated_at = now_ms();
                view.sign_in_error = None;
                fresh(view.clone())
            };
            crate::recorder::commands::record_usage(app, &next, "signin"); // recorder-forecast
            let _ = app.emit("usage", &next);
            warn(app, &next);
            EVERY
        }
        401 | 403 => {
            set_sign_in_error(app, Some("Claude Code's sign-in on this PC has expired. Run claude auth login in a terminal once.".into()));
            EVERY
        }
        429 => {
            let secs = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(600u64);
            set_sign_in_error(app, Some(format!("Anthropic asked Atoll to check usage less often. Next try in {} min.", secs.div_ceil(60))));
            std::time::Duration::from_secs(secs.max(EVERY.as_secs()))
        }
        code => {
            set_sign_in_error(app, Some(format!("Usage check failed (Anthropic answered {code}).")));
            EVERY
        }
    }
}

/// Ask every few minutes while the switch is on; a finished turn asks sooner.
pub fn start_sign_in_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().unwrap_or_default();
        let mut last: Option<std::time::Instant> = None;
        loop {
            let wait = if !sign_in_on(&app) {
                set_sign_in_error(&app, None);
                EVERY
            } else if let Some(gap) = last.map(|t| t.elapsed()).filter(|e| *e < MIN_GAP) {
                MIN_GAP - gap
            } else {
                last = Some(std::time::Instant::now());
                check_sign_in(&app, &http).await
            };
            let state = app.state::<UsageState>();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = state.poke.notified() => {}
            }
        }
    });
}

/// Development builds only: renew now, whatever the expiry (tests renewal).
#[cfg(debug_assertions)]
pub async fn debug_renew() -> Result<String, String> {
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let login = read_login()?;
    renew(&http, &login).await.map(|t| format!("renewed (new token {} characters)", t.len()))
}

/// A turn finished or the setting changed: check soon (never more than once a minute).
pub fn poke(app: &AppHandle) {
    app.state::<UsageState>().poke.notify_one();
}

/// Apply one status line payload.
pub fn update(app: &AppHandle, payload: &Value) {
    let Some(limits) = payload.get("rate_limits") else { return };
    let state = app.state::<UsageState>();
    let next = {
        let mut view = state.view.lock().unwrap();
        // Each window may be missing independently; keep what we had.
        if let Some(w) = limits.get("five_hour").and_then(window) {
            view.five_hour = Some(w);
        }
        if let Some(w) = limits.get("seven_day").and_then(window) {
            view.seven_day = Some(w);
        }
        view.updated_at = now_ms();
        fresh(view.clone())
    };
    crate::recorder::commands::record_usage(app, &next, "statusline"); // recorder-forecast
    let _ = app.emit("usage", &next);
    warn(app, &next);
}

/// One warning per 5-hour window when it crosses the threshold.
fn warn(app: &AppHandle, next: &UsageView) {
    let state = app.state::<UsageState>();
    let threshold = app.state::<AppState>().settings.lock().unwrap().alerts.usage_threshold as f64;
    if let Some(w) = next.five_hour {
        let mut warned = state.warned_for.lock().unwrap();
        if w.used >= threshold && *warned != Some(w.resets_at) {
            *warned = Some(w.resets_at);
            drop(warned);
            let mins = (w.resets_at.saturating_sub(now_ms() / 1000)) / 60;
            alerts::fire(
                app,
                Alert {
                    kind: Kind::Usage,
                    session_id: None,
                    title: format!("Claude usage at {:.0}%", w.used),
                    body: format!("Your 5-hour limit resets in {}h {:02}m.", mins / 60, mins % 60),
                },
                false,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renewed_sign_in_keeps_the_rest_of_the_file() {
        let file = json!({
            "mcpOAuth": { "x": { "accessToken": "mcp" } },
            "claudeAiOauth": { "accessToken": "old", "refreshToken": "r1", "expiresAt": 1, "scopes": ["a"], "subscriptionType": "max" }
        });
        let reply = json!({ "access_token": "new", "refresh_token": "r2", "expires_in": 3600, "scope": "a b" });
        let next = with_renewed(&file, &reply, 1000).unwrap();
        assert_eq!(next["claudeAiOauth"]["accessToken"], "new");
        assert_eq!(next["claudeAiOauth"]["refreshToken"], "r2");
        assert_eq!(next["claudeAiOauth"]["expiresAt"], 1000 + 3_600_000);
        assert_eq!(next["claudeAiOauth"]["scopes"], json!(["a", "b"]));
        assert_eq!(next["claudeAiOauth"]["subscriptionType"], "max");
        assert_eq!(next["mcpOAuth"], file["mcpOAuth"]);
        // A reply without a new refresh token keeps the old one.
        let kept = with_renewed(&file, &json!({ "access_token": "n", "expires_in": 1 }), 0).unwrap();
        assert_eq!(kept["claudeAiOauth"]["refreshToken"], "r1");
    }

    #[test]
    fn parses_usage_reset_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2025-11-07T20:00:00.123456+00:00"), Some(1762545600));
        assert_eq!(parse_time("2025-11-08T04:00:00+08:00"), Some(1762545600));
        let w = signed_window(&json!({ "utilization": 37.0, "resets_at": "2025-11-07T20:00:00+00:00" })).unwrap();
        assert_eq!((w.used, w.resets_at), (37.0, 1762545600));
        let idle = signed_window(&json!({ "utilization": 0.0, "resets_at": null })).unwrap();
        assert_eq!(idle.resets_at, 0);
    }

    #[test]
    fn reads_a_limit_hit_from_a_session_log_line() {
        // Shape copied from a real Claude desktop session log (text trimmed).
        let line = r#"{"type":"assistant","apiError":"usage_limit_reached","apiErrorParams":{"rate_limit_info":{"status":"rejected","resetsAt":1793894400,"rateLimitType":"five_hour","overageStatus":"rejected"}}}"#;
        assert_eq!(limit_hit(line), Some(("five_hour", 1793894400)));
        let weekly = r#"{"quotaLimits":{"status":"rejected","resetsAt":5,"rateLimitType":"seven_day_opus"}}"#;
        assert_eq!(limit_hit(weekly), Some(("seven_day", 5)));
        assert_eq!(limit_hit(r#"{"rate_limit_info":{"status":"allowed","resetsAt":5,"rateLimitType":"five_hour"}}"#), None);
        assert_eq!(limit_hit(r#"{"type":"user","text":"hello"}"#), None);
    }

    #[test]
    fn parses_windows_and_drops_expired() {
        let now = now_ms() / 1000;
        let w = window(&json!({"used_percentage": 23.5, "resets_at": now + 3600})).unwrap();
        assert_eq!(w.used, 23.5);
        let v = fresh(UsageView {
            five_hour: Some(w),
            seven_day: Some(LimitWindow { used: 10.0, resets_at: now - 1 }),
            updated_at: 1,
            sign_in_error: None,
        });
        assert!(v.five_hour.is_some());
        assert!(v.seven_day.is_none());
        assert!(window(&json!({"used_percentage": "x"})).is_none());
    }
}
