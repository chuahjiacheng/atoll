//! Google Calendar: your calendars and upcoming events in the island.
//!
//! Google only gives calendar data to an OAuth client. Atoll ships its own
//! ("Desktop app" type, google_client.rs), so Connect just works; Settings →
//! Calendar → Advanced can use your own client instead. Sign-in is Authorization Code with
//! PKCE through the browser; Google allows any loopback port for desktop
//! clients, so the redirect is http://127.0.0.1:<random port>/google/callback,
//! opened only while a sign-in is in progress (at most 10 minutes).
//!
//! Scope: calendar.readonly (Atoll never changes your calendar). The client
//! secret and refresh token live in Windows Credential Manager, the access
//! token (1 h) in memory. Events are fetched every 5 minutes, when you open
//! the Calendar tab and when settings change; nothing else is sent.
//! https://developers.google.com/identity/protocols/oauth2/native-app
//! https://developers.google.com/calendar/api/v3/reference

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const API: &str = "https://www.googleapis.com/calendar/v3";
const KEYRING_SERVICE: &str = "Crest";
const KEY_REFRESH: &str = "google-refresh-token";
const KEY_SECRET: &str = "google-client-secret";
/// How often events are fetched while connected.
const POLL: Duration = Duration::from_secs(5 * 60);
/// Event window around now, for the island's month grid (this month and next).
const FETCH_BACK_DAYS: i64 = 31;
const FETCH_AHEAD_DAYS: i64 = 62;

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfo {
    pub id: String,
    pub name: String,
    pub color: String,
    pub primary: bool,
    /// Shown in the island (your choice, else Google's own "selected").
    pub on: bool,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    /// RFC 3339 date-time, or YYYY-MM-DD for all-day events.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: Option<String>,
    /// Video call link (Meet, Zoom, Teams…) when the event has one.
    pub meet: Option<String>,
    /// The event in Google Calendar.
    pub link: Option<String>,
    pub color: String,
    /// You said no to it.
    pub declined: bool,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    pub connected: bool,
    pub has_client_id: bool,
    pub has_secret: bool,
    pub signing_in: bool,
    /// The signed-in Google account (its primary calendar's id).
    pub account: Option<String>,
    pub error: Option<String>,
    pub calendars: Vec<CalendarInfo>,
}

#[derive(Clone, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Agenda {
    pub events: Vec<Event>,
    /// ms since epoch of the last successful fetch (0 = never).
    pub fetched_at: u64,
}

struct Pending {
    verifier: String,
    state: String,
    redirect_uri: String,
}

/// A calendar as Google lists it (before your on/off choices).
#[derive(Clone)]
struct RawCalendar {
    id: String,
    name: String,
    color: String,
    primary: bool,
    selected: bool,
}

pub struct CalendarState {
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
    pending: Mutex<Option<Pending>>,
    connected: Mutex<bool>,
    account: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
    calendars: Mutex<Vec<RawCalendar>>,
    agenda: Mutex<Agenda>,
    /// Running sign-in callback listener's stop signal.
    callback: Mutex<Option<Arc<tokio::sync::Notify>>>,
    poke: tokio::sync::Notify,
}

impl Default for CalendarState {
    fn default() -> Self {
        CalendarState {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .user_agent(crate::lyrics::USER_AGENT)
                .build()
                .expect("http client"),
            token: Mutex::new(None),
            pending: Mutex::new(None),
            connected: Mutex::new(load(KEY_REFRESH).is_some()),
            account: Mutex::new(None),
            error: Mutex::new(None),
            calendars: Mutex::new(Vec::new()),
            agenda: Mutex::new(Agenda::default()),
            callback: Mutex::new(None),
            poke: tokio::sync::Notify::new(),
        }
    }
}

// ---------- Credential Manager ----------

fn entry(key: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, key).ok()
}

fn load(key: &str) -> Option<String> {
    entry(key)?.get_password().ok().filter(|s| !s.is_empty())
}

fn save(key: &str, value: &str) -> Result<(), String> {
    entry(key)
        .ok_or("Windows Credential Manager is unavailable.")?
        .set_password(value)
        .map_err(|e| format!("Couldn't save to Windows Credential Manager: {e}"))
}

fn delete(key: &str) {
    if let Some(e) = entry(key) {
        let _ = e.delete_credential();
    }
}

// ---------- helpers ----------

fn random_string(len: usize, charset: &[u8]) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("OS random number generator unavailable");
    bytes.iter().map(|b| charset[*b as usize % charset.len()] as char).collect()
}

fn pkce_pair() -> (String, String) {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let verifier = random_string(64, CHARSET);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The OAuth client to use: your own (Settings → Advanced) when its ID is
/// set, otherwise Atoll's built-in one.
fn client(app: &AppHandle) -> Result<(String, String), String> {
    let own = app.state::<AppState>().settings.lock().unwrap().calendar.client_id.clone();
    if own.is_empty() {
        return Ok((crate::google_client::CLIENT_ID.into(), crate::google_client::CLIENT_SECRET.into()));
    }
    let secret = load(KEY_SECRET).ok_or("Your own client needs its secret too. Paste it under Settings → Calendar → Advanced, or clear the client ID to use Atoll's.")?;
    Ok((own, secret))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Seconds since the epoch as an RFC 3339 UTC time ("2026-11-09T08:00:00Z").
fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn set_error(app: &AppHandle, error: Option<String>) {
    *app.state::<CalendarState>().error.lock().unwrap() = error;
    emit_status(app);
}

pub fn status(app: &AppHandle) -> CalendarStatus {
    let s = app.state::<CalendarState>();
    let settings = app.state::<AppState>().settings.lock().unwrap().calendar.clone();
    let calendars = s
        .calendars
        .lock()
        .unwrap()
        .iter()
        .map(|c| CalendarInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            color: c.color.clone(),
            primary: c.primary,
            on: settings.shown.get(&c.id).copied().unwrap_or(c.selected || c.primary),
        })
        .collect();
    let connected = *s.connected.lock().unwrap();
    let signing_in = s.pending.lock().unwrap().is_some();
    let account = s.account.lock().unwrap().clone();
    let error = s.error.lock().unwrap().clone();
    CalendarStatus {
        connected,
        has_client_id: !settings.client_id.is_empty(),
        has_secret: load(KEY_SECRET).is_some(),
        signing_in,
        account,
        error,
        calendars,
    }
}

pub fn emit_status(app: &AppHandle) {
    let _ = app.emit("calendar-status", status(app));
}

pub fn agenda(app: &AppHandle) -> Agenda {
    app.state::<CalendarState>().agenda.lock().unwrap().clone()
}

/// Fetch again soon (settings changed, the tab was opened, Refresh).
pub fn poke(app: &AppHandle) {
    app.state::<CalendarState>().poke.notify_one();
}

/// Store the OAuth client secret (empty = forget it).
pub fn set_secret(app: &AppHandle, secret: &str) -> Result<(), String> {
    let secret = secret.trim();
    if secret.is_empty() {
        delete(KEY_SECRET);
    } else {
        if secret.len() > 200 || secret.chars().any(|c| c.is_whitespace()) {
            return Err("That doesn't look like a client secret.".into());
        }
        save(KEY_SECRET, secret)?;
    }
    emit_status(app);
    Ok(())
}

// ---------- sign-in ----------

/// Step 1: open Google's consent page in the browser.
pub fn begin_login(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let (client_id, _) = client(app)?;
    let s = app.state::<CalendarState>();
    // Any loopback port works for a Desktop app client: let Windows pick one.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("Couldn't open a sign-in port: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(tokio::sync::Notify::new());
    if let Some(old) = s.callback.lock().unwrap().replace(stop.clone()) {
        old.notify_one();
    }
    crate::server::serve_google_callback(app.clone(), listener, stop);

    let (verifier, challenge) = pkce_pair();
    let state = random_string(24, b"abcdefghijklmnopqrstuvwxyz0123456789");
    let redirect_uri = format!("http://127.0.0.1:{port}/google/callback");
    let url = reqwest::Url::parse_with_params(
        AUTHORIZE_URL,
        &[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("scope", SCOPE),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
            ("state", state.as_str()),
            // A refresh token, every time (Google only sends one on consent).
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )
    .map_err(|e| e.to_string())?;
    *s.pending.lock().unwrap() = Some(Pending { verifier, state, redirect_uri });
    *s.error.lock().unwrap() = None;
    emit_status(app);
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| format!("Couldn't open the browser: {e}"))
}

/// Stop waiting for a sign-in.
pub fn cancel_login(app: &AppHandle) {
    let s = app.state::<CalendarState>();
    s.pending.lock().unwrap().take();
    if let Some(stop) = s.callback.lock().unwrap().take() {
        stop.notify_one();
    }
    emit_status(app);
}

/// Called when the callback listener exits.
pub fn callback_listener_ended(app: &AppHandle, stop: &Arc<tokio::sync::Notify>) {
    let s = app.state::<CalendarState>();
    let mut cb = s.callback.lock().unwrap();
    if cb.as_ref().is_some_and(|current| Arc::ptr_eq(current, stop)) {
        *cb = None;
        // Timed out without an answer: stop showing "signing in".
        if s.pending.lock().unwrap().take().is_some() {
            drop(cb);
            emit_status(app);
        }
    }
}

/// Step 2: Google redirected the browser back to /google/callback.
pub async fn finish_login(app: &AppHandle, code: Option<String>, state: Option<String>, error: Option<String>) -> Result<(), String> {
    let result = exchange(app, code, state, error).await;
    if let Err(e) = &result {
        set_error(app, Some(e.clone()));
    }
    result
}

async fn exchange(app: &AppHandle, code: Option<String>, state: Option<String>, error: Option<String>) -> Result<(), String> {
    let s = app.state::<CalendarState>();
    // Single use: whatever happens, the pending sign-in is consumed.
    let pending = s.pending.lock().unwrap().take().ok_or("No sign-in was started from Atoll.")?;
    if let Some(e) = error {
        return Err(if e == "access_denied" { "Sign-in was cancelled in the browser.".into() } else { format!("Google said: {e}") });
    }
    if state.as_deref() != Some(pending.state.as_str()) {
        return Err("Sign-in check failed (state mismatch). Try again from Atoll.".into());
    }
    let code = code.ok_or("Google did not return a code.")?;
    let (client_id, secret) = client(app)?;
    let resp = s
        .http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", secret.as_str()),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("Token exchange failed: {}", token_error(&body)));
    }
    if body.get("refresh_token").and_then(Value::as_str).is_none() {
        return Err("Google sent no refresh token. Remove Atoll's access at myaccount.google.com/permissions and connect again.".into());
    }
    store_tokens(app, &body)?;
    *s.connected.lock().unwrap() = true;
    *s.error.lock().unwrap() = None;
    emit_status(app);
    poke(app);
    Ok(())
}

fn token_error(body: &Value) -> String {
    let e = body.get("error_description").or(body.get("error")).and_then(Value::as_str).unwrap_or("unknown error");
    match body.get("error").and_then(Value::as_str) {
        Some("invalid_client") => "Google doesn't recognise the client ID and secret. If you set your own under Settings → Calendar → Advanced, check both.".into(),
        _ => e.to_string(),
    }
}

fn store_tokens(app: &AppHandle, body: &Value) -> Result<(), String> {
    let s = app.state::<CalendarState>();
    if let Some(access) = body.get("access_token").and_then(Value::as_str) {
        let secs = body.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
        *s.token.lock().unwrap() = Some((access.to_string(), Instant::now() + Duration::from_secs(secs.saturating_sub(60))));
    }
    if let Some(refresh) = body.get("refresh_token").and_then(Value::as_str) {
        save(KEY_REFRESH, refresh)?;
    }
    Ok(())
}

/// Sign out: revoke the token at Google and forget everything.
pub async fn logout(app: &AppHandle) {
    let s = app.state::<CalendarState>();
    if let Some(refresh) = load(KEY_REFRESH) {
        let _ = s.http.post(REVOKE_URL).form(&[("token", refresh.as_str())]).send().await;
    }
    forget(app);
    emit_status(app);
    let _ = app.emit("calendar-agenda", agenda(app));
}

fn forget(app: &AppHandle) {
    let s = app.state::<CalendarState>();
    delete(KEY_REFRESH);
    *s.connected.lock().unwrap() = false;
    *s.token.lock().unwrap() = None;
    *s.account.lock().unwrap() = None;
    *s.error.lock().unwrap() = None;
    s.calendars.lock().unwrap().clear();
    *s.agenda.lock().unwrap() = Agenda::default();
}

async fn access_token(app: &AppHandle) -> Result<String, String> {
    let s = app.state::<CalendarState>();
    if let Some((tok, exp)) = s.token.lock().unwrap().clone() {
        if Instant::now() < exp {
            return Ok(tok);
        }
    }
    let refresh = load(KEY_REFRESH).ok_or("Not connected to Google.")?;
    let (client_id, secret) = client(app)?;
    let resp = s
        .http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Google: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        if matches!(body.get("error").and_then(Value::as_str), Some("invalid_grant" | "unauthorized_client")) {
            // Revoked, a "Testing" consent screen's 7-day limit ran out, or
            // the client changed (own ↔ built-in) since this sign-in.
            forget(app);
            return Err("Google signed Atoll out. Connect again in Settings → Calendar.".into());
        }
        return Err(format!("Token refresh failed: {}", token_error(&body)));
    }
    store_tokens(app, &body)?;
    let token = s.token.lock().unwrap().clone();
    token.map(|t| t.0).ok_or_else(|| "no access token".to_string())
}

async fn get(app: &AppHandle, url: reqwest::Url) -> Result<Value, String> {
    let s = app.state::<CalendarState>();
    for attempt in 0..2 {
        let token = access_token(app).await?;
        let resp = s.http.get(url.clone()).bearer_auth(&token).send().await.map_err(|e| format!("Couldn't reach Google: {e}"))?;
        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            *s.token.lock().unwrap() = None;
            continue;
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let msg = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("").to_string();
        return Err(if status == 403 && (msg.contains("has not been used") || msg.contains("is disabled")) {
            "The Google Calendar API is off for your Cloud project. Enable it (step 2 in Settings → Calendar), wait a minute and refresh.".into()
        } else {
            format!("Google Calendar answered {status}{}", if msg.is_empty() { String::new() } else { format!(": {msg}") })
        });
    }
    Err("Google keeps refusing the sign-in. Connect again.".into())
}

// ---------- fetching ----------

async fn fetch_calendars(app: &AppHandle) -> Result<Vec<RawCalendar>, String> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut url = reqwest::Url::parse(&format!("{API}/users/me/calendarList")).unwrap();
        url.query_pairs_mut().append_pair("maxResults", "250");
        if let Some(p) = &page {
            url.query_pairs_mut().append_pair("pageToken", p);
        }
        let body = get(app, url).await?;
        for c in body.get("items").and_then(Value::as_array).into_iter().flatten() {
            let str_of = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let name = c.get("summaryOverride").and_then(Value::as_str).map(String::from).unwrap_or_else(|| str_of("summary"));
            out.push(RawCalendar {
                id: str_of("id"),
                name,
                color: c.get("backgroundColor").and_then(Value::as_str).unwrap_or("#7986cb").to_string(),
                primary: c.get("primary").and_then(Value::as_bool).unwrap_or(false),
                selected: c.get("selected").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        page = body.get("nextPageToken").and_then(Value::as_str).map(String::from);
        if page.is_none() {
            break;
        }
    }
    // Primary first, then by name.
    out.sort_by(|a, b| b.primary.cmp(&a.primary).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

fn parse_event(calendar: &RawCalendar, e: &Value) -> Option<Event> {
    if e.get("status").and_then(Value::as_str) == Some("cancelled") {
        return None;
    }
    let when = |k: &str| -> Option<(String, bool)> {
        let t = e.get(k)?;
        if let Some(dt) = t.get("dateTime").and_then(Value::as_str) {
            Some((dt.to_string(), false))
        } else {
            t.get("date").and_then(Value::as_str).map(|d| (d.to_string(), true))
        }
    };
    let (start, all_day) = when("start")?;
    let (end, _) = when("end").unwrap_or_else(|| (start.clone(), all_day));
    let meet = e.get("hangoutLink").and_then(Value::as_str).map(String::from).or_else(|| {
        e.pointer("/conferenceData/entryPoints")?
            .as_array()?
            .iter()
            .find(|p| p.get("entryPointType").and_then(Value::as_str) == Some("video"))?
            .get("uri")?
            .as_str()
            .map(String::from)
    });
    let declined = e
        .get("attendees")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|p| p.get("self").and_then(Value::as_bool) == Some(true)))
        .and_then(|me| me.get("responseStatus").and_then(Value::as_str))
        == Some("declined");
    let text = |k: &str| e.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    Some(Event {
        id: text("id").unwrap_or_default(),
        calendar_id: calendar.id.clone(),
        title: text("summary").unwrap_or_else(|| "(No title)".into()),
        start,
        end,
        all_day,
        location: text("location"),
        meet,
        link: text("htmlLink"),
        color: calendar.color.clone(),
        declined,
    })
}

async fn fetch_events(app: &AppHandle, calendars: &[RawCalendar]) -> Result<Vec<Event>, String> {
    let settings = app.state::<AppState>().settings.lock().unwrap().calendar.clone();
    let now = (now_ms() / 1000) as i64;
    // The island's month grid covers this month and the next, so fetch a month
    // back and two ahead (at least as far as the agenda's last day).
    let time_min = rfc3339(now - FETCH_BACK_DAYS * 86_400);
    let time_max = rfc3339(now + i64::from(settings.days_ahead + 1).max(FETCH_AHEAD_DAYS) * 86_400);
    let mut events = Vec::new();
    for c in calendars.iter().filter(|c| settings.shown.get(&c.id).copied().unwrap_or(c.selected || c.primary)) {
        let mut url = reqwest::Url::parse(&format!("{API}/calendars/")).unwrap();
        url.path_segments_mut().unwrap().pop_if_empty().push(&c.id).push("events");
        url.query_pairs_mut()
            .append_pair("timeMin", &time_min)
            .append_pair("timeMax", &time_max)
            .append_pair("singleEvents", "true")
            .append_pair("orderBy", "startTime")
            .append_pair("maxResults", "250");
        let body = get(app, url).await?;
        events.extend(body.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|e| parse_event(c, e)));
    }
    // RFC 3339 strings with offsets don't sort as text; the island sorts by real time.
    Ok(events)
}

async fn refresh(app: &AppHandle) {
    if !*app.state::<CalendarState>().connected.lock().unwrap() {
        return;
    }
    let result = async {
        let calendars = fetch_calendars(app).await?;
        let events = fetch_events(app, &calendars).await?;
        Ok::<_, String>((calendars, events))
    }
    .await;
    let s = app.state::<CalendarState>();
    match result {
        Ok((calendars, events)) => {
            *s.account.lock().unwrap() = calendars.iter().find(|c| c.primary).map(|c| c.id.clone());
            *s.calendars.lock().unwrap() = calendars;
            *s.agenda.lock().unwrap() = Agenda { events, fetched_at: now_ms() };
            *s.error.lock().unwrap() = None;
        }
        Err(e) => *s.error.lock().unwrap() = Some(e),
    }
    emit_status(app);
    let _ = app.emit("calendar-agenda", agenda(app));
}

/// Background loop: fetch every 5 minutes, or straight away when poked.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let on = app.state::<AppState>().settings.lock().unwrap().modules.calendar;
            if on {
                refresh(&app).await;
            }
            let s = app.state::<CalendarState>();
            tokio::select! {
                _ = s.poke.notified() => {
                    // Let a burst of changes (toggling several calendars) settle.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
                _ = tokio::time::sleep(POLL) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc3339_formats_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_762_675_200), "2025-11-09T08:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn parses_events() {
        let cal = RawCalendar { id: "me@x.com".into(), name: "Me".into(), color: "#123456".into(), primary: true, selected: true };
        let timed = json!({
            "id": "a", "status": "confirmed", "summary": " Standup ",
            "start": { "dateTime": "2026-11-09T09:00:00+08:00" }, "end": { "dateTime": "2026-11-09T09:15:00+08:00" },
            "conferenceData": { "entryPoints": [{ "entryPointType": "phone", "uri": "tel:1" }, { "entryPointType": "video", "uri": "https://zoom.us/j/1" }] },
            "attendees": [{ "email": "o@x.com", "responseStatus": "accepted" }, { "self": true, "responseStatus": "declined" }]
        });
        let e = parse_event(&cal, &timed).unwrap();
        assert_eq!((e.title.as_str(), e.all_day, e.declined), ("Standup", false, true));
        assert_eq!(e.meet.as_deref(), Some("https://zoom.us/j/1"));
        assert_eq!(e.color, "#123456");

        let all_day = json!({ "id": "b", "start": { "date": "2026-11-10" }, "end": { "date": "2026-11-11" } });
        let e = parse_event(&cal, &all_day).unwrap();
        assert!(e.all_day && !e.declined);
        assert_eq!(e.title, "(No title)");

        assert!(parse_event(&cal, &json!({ "status": "cancelled", "start": { "date": "2026-11-10" } })).is_none());
    }
}
