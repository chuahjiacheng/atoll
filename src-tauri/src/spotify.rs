//! Spotify: what's playing, playback controls, devices.
//!
//! Two sources:
//! - **Spotify on this PC** (spotify_local.rs, the main one): read and
//!   controlled through Windows' media session. One click to connect, no
//!   sign-in, no developer app, no rate limits, Free or Premium.
//! - **Linked account** (optional, this file): only for listing your Spotify
//!   devices and moving playback to another one, which Windows can't do.
//!   Spotify allows that only through the Web API with your own developer
//!   app (https://developer.spotify.com/documentation/web-api):
//!   Authorization Code with PKCE (no client secret), redirect to
//!   http://127.0.0.1:<callback port>/spotify/callback. The port is fixed
//!   (Settings → Spotify, default 43117) because Spotify's dashboard rejects a
//!   loopback redirect without a port; Atoll listens on it only while a
//!   sign-in is in progress (at most 10 minutes). The refresh token is kept in
//!   Windows Credential Manager, access tokens (1 h) in memory. The Web API is
//!   called only when you open the device menu or pick a device, never on a
//!   timer. On HTTP 429 it waits out Retry-After before asking again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::{lyrics, AppState};

const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing";
const API: &str = "https://api.spotify.com/v1";
const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const KEYRING_SERVICE: &str = "Crest";
const KEYRING_USER: &str = "spotify-refresh-token";

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub image: Option<String>,
    pub duration_ms: u64,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub is_active: bool,
    pub is_restricted: bool,
    pub volume: Option<u64>,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerView {
    pub is_playing: bool,
    pub track: Option<Track>,
    pub progress_ms: u64,
    /// When this snapshot was taken (ms since epoch), for local progress estimates.
    pub fetched_at: u64,
    pub device: Option<Device>,
    /// Spotify accepts seeking from Windows (else only with a linked account).
    pub can_seek: bool,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyStatus {
    /// Reading Spotify on this PC is switched on (the Connect button).
    pub connected: bool,
    /// The Spotify app is open on this PC.
    pub app_open: bool,
    /// A Spotify window is open but Spotify isn't sharing its playback with
    /// Windows' media controls, so the island can't see or control it.
    pub app_unshared: bool,
    /// An account is linked for device switching (optional).
    pub account: bool,
    pub has_client_id: bool,
    pub user: Option<String>,
    pub error: Option<String>,
    pub redirect_uri: String,
}

struct Pending {
    verifier: String,
    state: String,
    redirect_uri: String,
    client_id: String,
}

pub struct SpotifyState {
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
    pending: Mutex<Option<Pending>>,
    /// A linked account (refresh token stored).
    account: AtomicBool,
    app_open: AtomicBool,
    app_unshared: AtomicBool,
    user: Mutex<Option<String>>,
    /// Latest view of the player, and the one the island last received.
    player: Mutex<Option<PlayerView>>,
    sent: Mutex<Option<PlayerView>>,
    backoff_until: Mutex<Option<Instant>>,
    last_error: Mutex<Option<String>>,
    lyrics_for: Mutex<Option<String>>,
    /// Running sign-in callback listener: (port, stop signal).
    callback: Mutex<Option<(u16, std::sync::Arc<tokio::sync::Notify>)>>,
}

impl Default for SpotifyState {
    fn default() -> Self {
        SpotifyState {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent(lyrics::USER_AGENT)
                .build()
                .expect("http client"),
            token: Mutex::new(None),
            pending: Mutex::new(None),
            account: AtomicBool::new(load_refresh().is_some()),
            app_open: AtomicBool::new(false),
            app_unshared: AtomicBool::new(false),
            user: Mutex::new(None),
            player: Mutex::new(None),
            sent: Mutex::new(None),
            backoff_until: Mutex::new(None),
            last_error: Mutex::new(None),
            lyrics_for: Mutex::new(None),
            callback: Mutex::new(None),
        }
    }
}

// ---------- Credential Manager ----------

fn keyring_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

fn load_refresh() -> Option<String> {
    keyring_entry()?.get_password().ok()
}

fn save_refresh(token: &str) {
    if let Some(e) = keyring_entry() {
        if let Err(err) = e.set_password(token) {
            eprintln!("[crest] could not store Spotify token in Credential Manager: {err}");
        }
    }
}

fn delete_refresh() {
    if let Some(e) = keyring_entry() {
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

fn client_id(app: &AppHandle) -> String {
    app.state::<AppState>().settings.lock().unwrap().spotify.client_id.clone()
}

fn callback_port(app: &AppHandle) -> u16 {
    app.state::<AppState>().settings.lock().unwrap().spotify.callback_port
}

fn redirect_uri(app: &AppHandle) -> String {
    format!("http://127.0.0.1:{}/spotify/callback", callback_port(app))
}

/// Start the short-lived callback listener on the fixed port (or keep the
/// one that is already running there).
fn ensure_callback_listener(app: &AppHandle) -> Result<(), String> {
    let port = callback_port(app);
    let s = app.state::<SpotifyState>();
    let mut cb = s.callback.lock().unwrap();
    if let Some((running, stop)) = cb.as_ref() {
        if *running == port {
            return Ok(());
        }
        stop.notify_one();
    }
    *cb = None;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!(
            "Port {port} is busy ({e}). Choose another callback port in Settings → Music and change the redirect URI in your Spotify app to match."
        )
    })?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    *cb = Some((port, stop.clone()));
    crate::server::serve_spotify_callback(app.clone(), listener, stop);
    Ok(())
}

/// Called when the callback listener exits: forget it if it is still the current one.
pub fn callback_listener_ended(app: &AppHandle, stop: &std::sync::Arc<tokio::sync::Notify>) {
    let s = app.state::<SpotifyState>();
    let mut cb = s.callback.lock().unwrap();
    if cb.as_ref().is_some_and(|(_, current)| std::sync::Arc::ptr_eq(current, stop)) {
        *cb = None;
    }
}

fn set_error(app: &AppHandle, error: Option<String>) {
    *app.state::<SpotifyState>().last_error.lock().unwrap() = error;
    emit_status(app);
}

pub fn status(app: &AppHandle) -> SpotifyStatus {
    let s = app.state::<SpotifyState>();
    let user = s.user.lock().unwrap().clone();
    let error = s.last_error.lock().unwrap().clone();
    let connected = app.state::<AppState>().settings.lock().unwrap().spotify.connected;
    SpotifyStatus {
        connected,
        app_open: s.app_open.load(Ordering::Relaxed),
        app_unshared: s.app_unshared.load(Ordering::Relaxed),
        account: s.account.load(Ordering::Relaxed),
        has_client_id: !client_id(app).is_empty(),
        user,
        error,
        redirect_uri: redirect_uri(app),
    }
}

pub fn emit_status(app: &AppHandle) {
    let _ = app.emit("spotify-status", status(app));
}

pub fn player(app: &AppHandle) -> Option<PlayerView> {
    app.state::<SpotifyState>().player.lock().unwrap().clone()
}

/// Is the island's copy out of date? Between updates it estimates the
/// position itself, so the same track and state is only re-sent after a
/// jump (a seek, or a drift of more than 1.5 s).
fn needs_emit(last: Option<&PlayerView>, next: Option<&PlayerView>) -> bool {
    match (last, next) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            if a.is_playing != b.is_playing || a.can_seek != b.can_seek || a.track != b.track {
                return true;
            }
            let expected = a.progress_ms + if a.is_playing { b.fetched_at.saturating_sub(a.fetched_at) } else { 0 };
            expected.abs_diff(b.progress_ms) > 1_500
        }
        _ => true,
    }
}

/// New player view from the watcher: kept, and sent to the island if it changed.
pub fn publish_player(app: &AppHandle, view: Option<PlayerView>) {
    let s = app.state::<SpotifyState>();
    *s.player.lock().unwrap() = view.clone();
    let mut sent = s.sent.lock().unwrap();
    if needs_emit(sent.as_ref(), view.as_ref()) {
        let _ = app.emit("spotify-player", &view);
        *sent = view;
    }
}

/// `open`: Spotify's media session is there. `unshared`: no session, but a
/// Spotify window is open.
pub fn set_app_state(app: &AppHandle, open: bool, unshared: bool) {
    let s = app.state::<SpotifyState>();
    let changed = s.app_open.swap(open, Ordering::Relaxed) != open;
    let changed = (s.app_unshared.swap(unshared, Ordering::Relaxed) != unshared) || changed;
    if changed {
        emit_status(app);
    }
}

/// Fetch lyrics for a track once, in the background.
pub fn want_lyrics(app: &AppHandle, t: &Track) {
    let s = app.state::<SpotifyState>();
    {
        let mut current = s.lyrics_for.lock().unwrap();
        if current.as_deref() == Some(t.id.as_str()) {
            return;
        }
        *current = Some(t.id.clone());
    }
    let (app2, http, t) = (app.clone(), s.http.clone(), t.clone());
    tauri::async_runtime::spawn(async move {
        let l = lyrics::fetch(&http, &t).await;
        lyrics::remember(&app2, l.clone());
        let _ = app2.emit("lyrics", &l);
    });
}

/// Open the Spotify app (its spotify: link).
pub fn open_app(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url("spotify:", None::<&str>).map_err(|_| "Couldn't open Spotify. Is the Spotify app installed?".to_string())
}

// ---------- sign-in ----------

// ---------- optional account link (device switching) ----------

/// Step 1: open Spotify's consent page in the browser.
pub fn begin_login(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let client_id = client_id(app);
    if client_id.is_empty() {
        return Err("Enter your Spotify app's Client ID first.".into());
    }
    let (verifier, challenge) = pkce_pair();
    let state = random_string(24, b"abcdefghijklmnopqrstuvwxyz0123456789");
    ensure_callback_listener(app)?;
    let redirect_uri = redirect_uri(app);
    let url = reqwest::Url::parse_with_params(
        AUTHORIZE_URL,
        &[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("scope", SCOPES),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
            ("state", state.as_str()),
        ],
    )
    .map_err(|e| e.to_string())?;
    *app.state::<SpotifyState>().pending.lock().unwrap() = Some(Pending { verifier, state, redirect_uri, client_id });
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| format!("could not open the browser: {e}"))
}

/// Step 2: Spotify redirected back to /spotify/callback.
pub async fn finish_login(app: &AppHandle, code: Option<String>, state: Option<String>, error: Option<String>) -> Result<(), String> {
    let spotify = app.state::<SpotifyState>();
    // Single use: whatever happens, the pending login is consumed.
    let pending = spotify.pending.lock().unwrap().take().ok_or("No sign-in was started from Atoll.")?;
    if let Some(e) = error {
        return Err(format!("Spotify said: {e}"));
    }
    if state.as_deref() != Some(pending.state.as_str()) {
        return Err("Sign-in check failed (state mismatch). Try again from Atoll.".into());
    }
    let code = code.ok_or("Spotify did not return a code.")?;
    let resp = spotify
        .http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", pending.client_id.as_str()),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("Token exchange failed: {}", body.get("error_description").or(body.get("error")).unwrap_or(&body)));
    }
    store_tokens(app, &body);
    spotify.account.store(true, Ordering::Relaxed);
    *spotify.last_error.lock().unwrap() = None;
    if let Ok(Some(me)) = api(app, reqwest::Method::GET, "/me", None).await {
        *spotify.user.lock().unwrap() = me.get("display_name").and_then(Value::as_str).map(String::from);
    }
    emit_status(app);
    Ok(())
}

/// Forget the linked account. Spotify on this PC keeps working.
pub fn unlink_account(app: &AppHandle) {
    let s = app.state::<SpotifyState>();
    delete_refresh();
    s.account.store(false, Ordering::Relaxed);
    *s.token.lock().unwrap() = None;
    *s.user.lock().unwrap() = None;
    *s.last_error.lock().unwrap() = None;
    emit_status(app);
}

fn require_account(app: &AppHandle) -> Result<(), String> {
    if app.state::<SpotifyState>().account.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err("Switching devices needs a linked Spotify account (Settings → Spotify → Other devices).".into())
    }
}

fn store_tokens(app: &AppHandle, body: &Value) {
    let s = app.state::<SpotifyState>();
    if let Some(access) = body.get("access_token").and_then(Value::as_str) {
        let secs = body.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
        *s.token.lock().unwrap() = Some((access.to_string(), Instant::now() + Duration::from_secs(secs.saturating_sub(60))));
    }
    // A refresh may omit refresh_token: then keep the existing one.
    if let Some(refresh) = body.get("refresh_token").and_then(Value::as_str) {
        save_refresh(refresh);
    }
}

async fn access_token(app: &AppHandle) -> Result<String, String> {
    let s = app.state::<SpotifyState>();
    if let Some((tok, exp)) = s.token.lock().unwrap().clone() {
        if Instant::now() < exp {
            return Ok(tok);
        }
    }
    let refresh = load_refresh().ok_or("Not connected to Spotify.")?;
    let resp = s
        .http
        .post(TOKEN_URL)
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", client_id(app).as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("");
        // invalid_grant: the sign-in expired or was revoked (refresh tokens last
        // ~6 months). invalid_client: it belongs to a different client ID than the
        // one in Settings now. Either way it can never work again: sign in again.
        if err == "invalid_grant" || err == "invalid_client" {
            unlink_account(app);
            set_error(app, Some("The linked Spotify account signed out. Link it again to switch devices.".into()));
            return Err("The linked Spotify account signed out. Link it again in Settings → Spotify.".into());
        }
        let why = body.get("error_description").and_then(Value::as_str).unwrap_or(err);
        return Err(format!("Token refresh failed ({status}{}{why})", if why.is_empty() { "" } else { ": " }));
    }
    store_tokens(app, &body);
    let token = s.token.lock().unwrap().clone();
    token.map(|t| t.0).ok_or_else(|| "no access token".to_string())
}

/// Call the Web API. Ok(None) = 204 No Content.
async fn api(app: &AppHandle, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Option<Value>, String> {
    let s = app.state::<SpotifyState>();
    if let Some(left) = backoff_left(app) {
        return Err(wait_message(left));
    }
    for attempt in 0..2 {
        let token = access_token(app).await?;
        let mut req = s.http.request(method.clone(), format!("{API}{path}")).bearer_auth(&token);
        req = match &body {
            Some(b) => req.json(b),
            // Spotify's gateway answers 411 (Length Required) unless a bodiless
            // PUT/POST says so explicitly; an empty body alone sends no header.
            None if method != reqwest::Method::GET => req.header(reqwest::header::CONTENT_LENGTH, "0").body(""),
            None => req,
        };
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        match status {
            204 => return Ok(None),
            200..=299 => {
                let text = resp.text().await.unwrap_or_default();
                return Ok(if text.trim().is_empty() { None } else { serde_json::from_str(&text).ok() });
            }
            401 if attempt == 0 => {
                *s.token.lock().unwrap() = None; // expired early: refresh and retry once
                continue;
            }
            429 => {
                let secs = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(5u64);
                eprintln!("[crest] spotify 429 on {path}: retry after {secs} s");
                *s.backoff_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(secs));
                save_wait(crate::sessions::now_ms() + secs * 1000);
                return Err(wait_message(Duration::from_secs(secs)));
            }
            _ => {
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let msg = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("request failed");
                return Err(match status {
                    403 => format!("Spotify refused ({msg}). Playback control needs Premium, and your account must be on the app's user list."),
                    404 => "No active Spotify device. Start playing in Spotify, or pick a device.".into(),
                    _ => format!("Spotify error {status}: {msg}"),
                });
            }
        }
    }
    Err("Spotify authorisation failed.".into())
}

// ---------- Web API helpers ----------

fn parse_device(d: &Value) -> Device {
    Device {
        id: d.get("id").and_then(Value::as_str).map(String::from),
        name: d.get("name").and_then(Value::as_str).unwrap_or("Unknown device").to_string(),
        kind: d.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
        is_active: d.get("is_active").and_then(Value::as_bool).unwrap_or(false),
        is_restricted: d.get("is_restricted").and_then(Value::as_bool).unwrap_or(false),
        volume: d.get("volume_percent").and_then(Value::as_u64),
    }
}

/// A rate-limit wait survives restarts (each restart would otherwise ask
/// again and could lengthen Spotify's penalty). Holds the end time in ms.
fn wait_file() -> std::path::PathBuf {
    crate::paths::local_dir().join("spotify-wait.txt")
}

fn save_wait(until_ms: u64) {
    let _ = std::fs::create_dir_all(crate::paths::local_dir());
    let _ = std::fs::write(wait_file(), until_ms.to_string());
}

fn restore_wait(app: &AppHandle) {
    let until: u64 = std::fs::read_to_string(wait_file()).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let now = crate::sessions::now_ms();
    if until > now {
        // Only device switching waits; Spotify on this PC isn't affected.
        let left = Duration::from_millis(until - now);
        *app.state::<SpotifyState>().backoff_until.lock().unwrap() = Some(Instant::now() + left);
    } else {
        let _ = std::fs::remove_file(wait_file());
    }
}

/// Time left before Spotify allows another request (after a 429), if any.
fn backoff_left(app: &AppHandle) -> Option<Duration> {
    let until = (*app.state::<SpotifyState>().backoff_until.lock().unwrap())?;
    let left = until.saturating_duration_since(Instant::now());
    (!left.is_zero()).then_some(left)
}

fn wait_message(left: Duration) -> String {
    let secs = left.as_secs().max(1);
    let when = if secs < 90 { format!("{secs} s") } else if secs < 5400 { format!("{} min", (secs + 59) / 60) } else { format!("{} h", (secs + 1799) / 3600) };
    format!("Spotify asked Atoll to wait {when} before checking again (too many requests).")
}

pub fn start(app: AppHandle) {
    restore_wait(&app);
    crate::spotify_local::start(app);
}

/// play | pause | toggle | next | previous | seek (value = ms) | transfer (value = device id)
/// | volume (value = 0-100, linked account only).
/// Playback commands go to Spotify on this PC; with a linked account they
/// fall back to the Web API (e.g. when playing on another device).
pub async fn action(app: &AppHandle, action: &str, value: Option<Value>) -> Result<(), String> {
    if action == "transfer" {
        let id = value.and_then(|v| v.as_str().map(String::from)).ok_or("transfer needs a device id")?;
        require_account(app)?;
        api(app, reqwest::Method::PUT, "/me/player", Some(json!({ "device_ids": [id], "play": true }))).await?;
        return Ok(());
    }
    if action == "volume" {
        // Spotify's own volume (the in-app slider), not the Windows mixer.
        // Windows exposes no way to set it, so this is Web API only.
        let pct = value.and_then(|v| v.as_u64()).ok_or("volume needs a percentage")?.min(100);
        require_account(app)?;
        api(app, reqwest::Method::PUT, &format!("/me/player/volume?volume_percent={pct}"), None).await?;
        return Ok(());
    }
    let ms = value.as_ref().and_then(Value::as_u64);
    let name = action.to_string();
    let local = tauri::async_runtime::spawn_blocking(move || crate::spotify_local::command(&name, ms)).await.map_err(|e| e.to_string())?;
    match local {
        Ok(()) => Ok(()),
        Err(e) if app.state::<SpotifyState>().account.load(Ordering::Relaxed) => web_action(app, action, ms).await.map_err(|_| e),
        Err(e) => Err(e),
    }
}

async fn web_action(app: &AppHandle, action: &str, ms: Option<u64>) -> Result<(), String> {
    use reqwest::Method;
    let (method, path) = match action {
        "play" => (Method::PUT, "/me/player/play".to_string()),
        "pause" => (Method::PUT, "/me/player/pause".to_string()),
        "toggle" if player(app).is_some_and(|p| p.is_playing) => (Method::PUT, "/me/player/pause".to_string()),
        "toggle" => (Method::PUT, "/me/player/play".to_string()),
        "next" => (Method::POST, "/me/player/next".to_string()),
        "previous" => (Method::POST, "/me/player/previous".to_string()),
        "seek" => (Method::PUT, format!("/me/player/seek?position_ms={}", ms.ok_or("seek needs a position in ms")?)),
        other => return Err(format!("unknown action {other}")),
    };
    api(app, method, &path, None).await.map(|_| ())
}

/// Spotify's own volume (0-100) as the Spotify app on this PC shows it, read
/// through Windows UI Automation: no account or Premium. Read only; Spotify
/// ignores writes to its slider. None if the app isn't open or doesn't answer.
pub fn read_app_volume() -> Option<u8> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", include_str!("spotify_volume.ps1")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<u8>().ok().map(|v| v.min(100))
}

pub async fn devices(app: &AppHandle) -> Result<Vec<Device>, String> {
    require_account(app)?;
    let v = api(app, reqwest::Method::GET, "/me/player/devices", None).await?.unwrap_or(Value::Null);
    Ok(v.get("devices").and_then(Value::as_array).map(|a| a.iter().map(parse_device).collect()).unwrap_or_default())
}

/// Look up the display name once at startup (if already connected).
pub fn load_user(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if !app.state::<SpotifyState>().account.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(Some(me)) = api(&app, reqwest::Method::GET, "/me", None).await {
            *app.state::<SpotifyState>().user.lock().unwrap() = me.get("display_name").and_then(Value::as_str).map(String::from);
            emit_status(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let (v, c) = pkce_pair();
        assert_eq!(v.len(), 64);
        assert_eq!(c.len(), 43);
    }

    fn view(playing: bool, progress: u64, at: u64) -> PlayerView {
        PlayerView {
            is_playing: playing,
            track: Some(Track { id: "pc-1".into(), name: "Song".into(), artists: "A".into(), album: "Al".into(), image: None, duration_ms: 200_000 }),
            progress_ms: progress,
            fetched_at: at,
            device: None,
            can_seek: true,
        }
    }

    #[test]
    fn island_is_told_only_about_real_changes() {
        let a = view(true, 10_000, 1_000);
        // Steady playback: the island's own estimate is right, nothing to send.
        assert!(!needs_emit(Some(&a), Some(&view(true, 11_000, 2_000))));
        // A seek, a pause, a new track, appearing and disappearing are sent.
        assert!(needs_emit(Some(&a), Some(&view(true, 60_000, 2_000))));
        assert!(needs_emit(Some(&a), Some(&view(false, 11_000, 2_000))));
        let mut other = view(true, 0, 2_000);
        other.track.as_mut().unwrap().id = "pc-2".into();
        assert!(needs_emit(Some(&a), Some(&other)));
        assert!(needs_emit(None, Some(&a)));
        assert!(needs_emit(Some(&a), None));
        assert!(!needs_emit(None, None));
    }
}
