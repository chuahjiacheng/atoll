//! Ask: an AI assistant in the island (module switch: `modules.ask`).
//!
//! Stage 1 talks to Google Gemini (free API key from AI Studio). Replies
//! stream in: every piece updates the open assistant message and is sent to
//! the island as `ask:delta`; the whole chat goes out as `ask` after each
//! change. The API key lives in Windows Credential Manager, never in
//! settings.json. The chat is kept in `%LOCALAPPDATA%\Crest\ask\chat.json`
//! (unless Settings → Ask → Keep chat history is off) and a new chat starts by
//! itself after 30 minutes without use.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

const KEYRING_SERVICE: &str = "Crest";
const GEMINI_KEY: &str = "ask.gemini.apiKey";
const GROQ_KEY: &str = "ask.groq.apiKey";
const API: &str = "https://generativelanguage.googleapis.com/v1beta";
const GROQ_API: &str = "https://api.groq.com/openai/v1";
const OLLAMA_API: &str = "http://127.0.0.1:11434";
/// Groq models to use when none is picked, fastest good one first.
const GROQ_PREFERRED: [&str; 5] =
    ["llama-3.3-70b-versatile", "openai/gpt-oss-120b", "openai/gpt-oss-20b", "qwen/qwen3-32b", "llama-3.1-8b-instant"];

/// Google's alias for its newest Flash-Lite model: the quickest to start
/// answering, and an alias, so the default never goes stale.
pub const DEFAULT_MODEL: &str = "gemini-flash-lite-latest";
/// The default before 2026-11-27 (moved to DEFAULT_MODEL once, see seed_settings).
const OLD_DEFAULT_MODEL: &str = "gemini-flash-latest";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+KeyA";

/// A new chat starts after this long without a message.
const IDLE_NEW_CHAT_MS: u64 = 30 * 60 * 1000;
/// Messages kept in the chat (older ones drop off).
const MAX_KEPT: usize = 60;
/// Messages sent to the model as context (the newest ones).
const MAX_SENT: usize = 24;
/// Longest question accepted, in characters.
pub const MAX_INPUT: usize = 12_000;
/// Give up when the stream is silent this long.
const STREAM_IDLE: Duration = Duration::from_secs(90);

const SYSTEM_PROMPT: &str = "You are Ask, the assistant inside Atoll, a small overlay at the top of the user's Windows screen. \
Answer directly and briefly: lead with the answer, then only the detail that helps. Use Markdown (short paragraphs, lists, \
fenced code blocks with a language). Never pad with greetings or offers of further help.";

// ---------- chat model ----------

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Msg {
    pub id: u64,
    pub role: Role,
    pub text: String,
    /// ms since epoch.
    pub at: u64,
    /// Why the reply failed or stopped early (shown under it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Model that wrote an assistant message.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// "gemini", "groq" or "ollama".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub messages: Vec<Msg>,
    pub next_id: u64,
    pub updated_at: u64,
}

impl Chat {
    fn push(&mut self, role: Role, text: String, now: u64) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.messages.push(Msg { id, role, text, at: now, error: None, model: String::new(), provider: String::new() });
        if self.messages.len() > MAX_KEPT {
            let extra = self.messages.len() - MAX_KEPT;
            self.messages.drain(..extra);
        }
        self.updated_at = now;
        id
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Msg> {
        self.messages.iter_mut().find(|m| m.id == id)
    }

    /// Too long since the last message: time for a fresh chat.
    pub fn is_stale(&self, now: u64) -> bool {
        !self.messages.is_empty() && now.saturating_sub(self.updated_at) > IDLE_NEW_CHAT_MS
    }

    fn clear(&mut self) {
        self.messages.clear();
        self.updated_at = 0;
    }
}

/// Gemini `contents` for the chat: the newest messages, without failed or
/// empty replies, starting with a user turn (Gemini requires that).
pub fn gemini_contents(messages: &[Msg]) -> Vec<Value> {
    let usable: Vec<&Msg> = messages
        .iter()
        .filter(|m| !m.text.trim().is_empty() && !(m.role == Role::Assistant && m.error.is_some() && m.text.trim().is_empty()))
        .collect();
    let mut tail: &[&Msg] = &usable[usable.len().saturating_sub(MAX_SENT)..];
    while let Some(first) = tail.first() {
        if first.role == Role::User {
            break;
        }
        tail = &tail[1..];
    }
    // Merge neighbours with the same role (a failed reply leaves two user turns in a row).
    let mut out: Vec<(Role, String)> = Vec::new();
    for m in tail {
        match out.last_mut() {
            Some((r, text)) if *r == m.role => {
                text.push_str("\n\n");
                text.push_str(&m.text);
            }
            _ => out.push((m.role, m.text.clone())),
        }
    }
    out.into_iter()
        .map(|(role, text)| {
            json!({ "role": if role == Role::User { "user" } else { "model" }, "parts": [{ "text": text }] })
        })
        .collect()
}

// ---------- streaming (Server-Sent Events) ----------

/// Splits a byte stream into SSE `data` payloads. Bytes may arrive cut
/// anywhere, even inside a UTF-8 character.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data: Vec<String>,
}

impl SseParser {
    /// Feed bytes; returns every event completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(rest) = line.strip_prefix("data:") {
                self.data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            }
            // Comments (":") and other fields (event, id, retry) are not used.
        }
        events
    }

    /// End of stream: an event not followed by a blank line still counts.
    pub fn finish(&mut self) -> Option<String> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&rest);
            let line = line.trim_end_matches('\r');
            if let Some(d) = line.strip_prefix("data:") {
                self.data.push(d.strip_prefix(' ').unwrap_or(d).to_string());
            }
        }
        if self.data.is_empty() {
            None
        } else {
            let out = self.data.join("\n");
            self.data.clear();
            Some(out)
        }
    }
}

/// Text in one Gemini stream chunk (thought summaries left out), or why
/// Gemini refused to answer.
pub fn chunk_text(payload: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(payload).map_err(|_| "Gemini sent a reply Atoll couldn't read.".to_string())?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
        return Err(format!("Gemini error: {msg}"));
    }
    if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        return Err(format!("Gemini wouldn't answer this ({}).", reason.to_lowercase().replace('_', " ")));
    }
    let mut text = String::new();
    if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(Value::as_array) {
        for p in parts {
            if p.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                text.push_str(t);
            }
        }
    }
    match v.pointer("/candidates/0/finishReason").and_then(Value::as_str) {
        Some("SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII") if text.is_empty() => {
            Err("Gemini stopped this answer for safety reasons.".into())
        }
        _ => Ok(text),
    }
}

/// A plain-English message for a failed Gemini request.
pub fn friendly_error(status: u16, body: &str, model: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let low = format!("{body} {detail}").to_lowercase();
    if low.contains("api_key_invalid") || low.contains("api key not valid") || low.contains("api key expired") {
        return "Gemini didn't accept the API key. Check it in Settings → Ask.".into();
    }
    match status {
        401 | 403 => "This API key isn't allowed to use Gemini. Make a new key in Google AI Studio.".into(),
        404 => format!("Gemini doesn't offer the model \u{201c}{model}\u{201d}. Pick another in Settings → Ask."),
        429 => "You've used Gemini's free limit for now. Try again in a minute, or tomorrow if the daily limit is used up.".into(),
        400 if low.contains("location") || low.contains("region") => {
            "Gemini's free API isn't available in your region.".into()
        }
        503 if low.contains("high demand") || low.contains("overloaded") => {
            "Gemini is busy right now (high demand). Try again in a moment.".into()
        }
        500..=599 => format!("Gemini is having trouble (error {status}). Try again in a moment."),
        _ if !detail.is_empty() => format!("Gemini: {detail}"),
        _ => format!("Gemini returned error {status}."),
    }
}

/// Model ids that can chat, from a `models.list` reply (newest names first).
pub fn chat_models(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list
        .get("models")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(|x| x.as_str() == Some("generateContent")))
                })
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .filter_map(|n| n.strip_prefix("models/"))
                .filter(|n| n.starts_with("gemini") && !n.contains("embedding") && !n.contains("image") && !n.contains("tts"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

/// "AIza…wXyZ": enough to recognise a key without showing it.
pub fn mask_key(key: &str) -> String {
    let k: Vec<char> = key.chars().collect();
    if k.len() <= 8 {
        return "••••".into();
    }
    format!("{}…{}", k[..4].iter().collect::<String>(), k[k.len() - 4..].iter().collect::<String>())
}

/// A failed start that another model may not have: busy or overloaded (503,
/// 500), this model's own free limit used up (429), or a model Google no
/// longer offers to this key (404).
pub fn worth_another_model(status: u16) -> bool {
    matches!(status, 404 | 429 | 500 | 503)
}

/// Version numbers in a model id, for newest-first sorting ("gemini-3.10-flash" → [3, 10]).
fn version(id: &str) -> Vec<u32> {
    id.split('-').nth(1).map(|v| v.split('.').filter_map(|p| p.parse().ok()).collect()).unwrap_or_default()
}

/// Models to try, in order: the chosen one, Google's "latest" aliases, then
/// the four newest stable (non-preview) Flash models this key has, then
/// Flash-Lite. Only models in `available` are added after the chosen one.
pub fn fallback_order(chosen: &str, available: &[String]) -> Vec<String> {
    const MAX: usize = 8;
    /// Older Flash models are often retired for new keys; don't spend tries on them.
    const NEWEST_FLASH: usize = 4;
    let has = |m: &str| available.iter().any(|a| a == m);
    let stable = |suffix: &str| {
        let mut v: Vec<&String> = available
            .iter()
            .filter(|m| {
                let mut parts = m.split('-');
                parts.next() == Some("gemini")
                    && parts.next().is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                    && parts.collect::<Vec<_>>().join("-") == suffix
            })
            .collect();
        v.sort_by(|a, b| version(b).cmp(&version(a)));
        v.into_iter().cloned().collect::<Vec<_>>()
    };
    let mut out = vec![chosen.to_string()];
    let mut add = |m: String| {
        if !out.contains(&m) {
            out.push(m);
        }
    };
    // Fastest first: the Flash-Lite alias, then the Flash alias and newest Flash models.
    if has("gemini-flash-lite-latest") {
        add("gemini-flash-lite-latest".into());
    }
    if has("gemini-flash-latest") {
        add("gemini-flash-latest".into());
    }
    for m in stable("flash").into_iter().take(NEWEST_FLASH) {
        add(m);
    }
    for m in stable("flash-lite") {
        add(m);
    }
    out.truncate(MAX);
    out
}

/// After a fallback works, keep starting with it for this long.
const STICKY_MS: u64 = 30 * 60 * 1000;
/// How long the first model gets to start answering before the others are asked too.
const HEDGE: Duration = Duration::from_millis(2500);
/// Pro models think before they start, so they get longer.
const HEDGE_PRO: Duration = Duration::from_secs(8);

pub fn hedge_after(model: &str) -> Duration {
    if model.contains("-pro") {
        HEDGE_PRO
    } else {
        HEDGE
    }
}

/// The request: the chat, and as little thinking as the model allows
/// (thinking first makes answers several times slower to start).
pub fn request_body(contents: &[Value], minimal_thinking: bool) -> Value {
    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
        "contents": contents,
    });
    if minimal_thinking {
        body["generationConfig"] = json!({ "thinkingConfig": { "thinkingLevel": "minimal" } });
    }
    body
}

/// Google turned down the thinking setting (some models can't think less).
pub fn rejects_thinking_setting(status: u16, body: &str) -> bool {
    let low = body.to_lowercase();
    status == 400 && (low.contains("thinking") || low.contains("invalid argument"))
}

fn route_file() -> PathBuf {
    crate::paths::local_dir().join("ask").join("route.json")
}

/// The model to start with: a recent stand-in for `chosen`, else `chosen`.
fn sticky_start(state: &AskState, chosen: &str) -> String {
    if !state.sticky_loaded.swap(true, Ordering::SeqCst) {
        let saved = std::fs::read(route_file()).ok().and_then(|b| serde_json::from_slice::<(String, String, u64)>(&b).ok());
        let mut s = state.sticky.lock().unwrap();
        if s.is_none() {
            *s = saved;
        }
    }
    match state.sticky.lock().unwrap().as_ref() {
        Some((c, f, until)) if c == chosen && *until > now_ms() => f.clone(),
        _ => chosen.to_string(),
    }
}

/// Remember (on disk too) which model answered for `chosen`.
fn set_sticky(state: &AskState, chosen: &str, model: &str) {
    let next = (model != chosen).then(|| (chosen.to_string(), model.to_string(), now_ms() + STICKY_MS));
    let mut s = state.sticky.lock().unwrap();
    if *s == next || (next.is_none() && s.is_none()) {
        return;
    }
    *s = next.clone();
    {
        let path = route_file();
        match next {
            Some(v) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, serde_json::to_vec(&v).unwrap_or_default());
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

// ---------- state ----------

#[derive(Default)]
pub struct AskState {
    chat: Mutex<Option<Chat>>,
    /// (chosen model, model that answered instead, until ms): skip the busy one for a while.
    sticky: Mutex<Option<(String, String, u64)>>,
    /// The key's chat models, fetched the first time a fallback is needed.
    models: Mutex<Vec<String>>,
    /// Models that refused minimal thinking (asked with their default instead).
    full_thinking: Mutex<Vec<String>>,
    /// The saved stand-in has been read from disk.
    sticky_loaded: std::sync::atomic::AtomicBool,
    /// Groq chat models for the saved key (fetched when first needed).
    groq_models: Mutex<Vec<String>>,
    /// Today's request counts.
    usage: Mutex<Usage>,
    /// Id of the reply being written (0 = none).
    busy: AtomicU64,
    /// Bumped by every send and stop; a stream whose number is old stops writing.
    run: AtomicU64,
    cancel: tokio::sync::Notify,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AskView {
    pub messages: Vec<Msg>,
    /// Id of the reply still being written, if any.
    pub busy: Option<u64>,
    /// The main provider can be used (has its key, or is local).
    pub has_key: bool,
    /// Main provider and the model it is set to ("" = picked automatically).
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Delta {
    id: u64,
    text: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub has_key: bool,
    pub masked: String,
}

fn chat_file() -> PathBuf {
    crate::paths::local_dir().join("ask").join("chat.json")
}

fn now_ms() -> u64 {
    crate::sessions::now_ms()
}

fn keep_history(app: &AppHandle) -> bool {
    app.state::<crate::AppState>().settings.lock().unwrap().ask.keep_history
}

fn model(app: &AppHandle) -> String {
    app.state::<crate::AppState>().settings.lock().unwrap().ask.model.clone()
}

/// Run `f` on the chat, loading it from disk the first time.
fn with_chat<R>(app: &AppHandle, f: impl FnOnce(&mut Chat) -> R) -> R {
    let state = app.state::<AskState>();
    let mut guard = state.chat.lock().unwrap();
    let chat = guard.get_or_insert_with(|| {
        if keep_history(app) {
            std::fs::read(chat_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
        } else {
            Chat::default()
        }
    });
    f(chat)
}

fn save(app: &AppHandle) {
    let path = chat_file();
    if !keep_history(app) {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let bytes = with_chat(app, |c| serde_json::to_vec(c).unwrap_or_default());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn view(app: &AppHandle) -> AskView {
    let busy = app.state::<AskState>().busy.load(Ordering::SeqCst);
    let now = now_ms();
    let messages = with_chat(app, |c| {
        if busy == 0 && c.is_stale(now) {
            c.clear();
        }
        c.messages.clone()
    });
    let (provider, model, _) = choice(app);
    let has_key = provider_plan(&provider, true, load_key().is_some(), load_named(GROQ_KEY).is_some()).is_ok();
    AskView { messages, busy: (busy != 0).then_some(busy), has_key, provider, model }
}

fn emit(app: &AppHandle) {
    let _ = app.emit("ask", view(app));
}

// ---------- Credential Manager ----------

fn entry() -> Option<keyring::Entry> {
    named_entry(GEMINI_KEY)
}

fn named_entry(name: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, name).ok()
}

fn load_named(name: &str) -> Option<String> {
    named_entry(name)?.get_password().ok().filter(|s| !s.trim().is_empty())
}

fn load_key() -> Option<String> {
    load_named(GEMINI_KEY)
}

pub fn key_status() -> KeyStatus {
    match load_key() {
        Some(k) => KeyStatus { has_key: true, masked: mask_key(&k) },
        None => KeyStatus { has_key: false, masked: String::new() },
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Couldn't start a web request: {e}"))
}

/// Models this key can chat with; also proves the key works.
async fn list_models(key: &str) -> Result<Vec<String>, String> {
    let resp = client()?
        .get(format!("{API}/models?pageSize=1000"))
        .header("x-goog-api-key", key)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| offline(&e))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(friendly_error(status, &body, ""));
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| "Gemini sent a reply Atoll couldn't read.".to_string())?;
    Ok(chat_models(&v))
}

fn offline(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "Gemini took too long to answer. Check your connection and try again.".into()
    } else {
        "Couldn't reach Gemini. Check your internet connection.".into()
    }
}

/// Test a key with Google and keep it in Credential Manager if it works.
pub async fn set_key(app: &AppHandle, key: &str) -> Result<Vec<String>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your Gemini API key first.".into());
    }
    if key.len() > 200 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like an API key.".into());
    }
    let models = list_models(key).await?;
    entry()
        .ok_or("Windows Credential Manager is unavailable.")?
        .set_password(key)
        .map_err(|e| format!("Couldn't save to Windows Credential Manager: {e}"))?;
    *app.state::<AskState>().models.lock().unwrap() = models.clone();
    *app.state::<AskState>().sticky.lock().unwrap() = None;
    let _ = std::fs::remove_file(route_file());
    emit(app);
    Ok(models)
}

pub fn forget_key(app: &AppHandle) {
    if let Some(e) = entry() {
        let _ = e.delete_credential();
    }
    emit(app);
}

/// Models the saved key can use.
pub async fn models() -> Result<Vec<String>, String> {
    let key = load_key().ok_or("Add a Gemini API key first.")?;
    list_models(&key).await
}

// ---------- asking ----------

pub fn send(app: &AppHandle, text: String) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Type a question first.".into());
    }
    if text.chars().count() > MAX_INPUT {
        return Err(format!("That's too long. Keep it under {MAX_INPUT} characters."));
    }
    let state = app.state::<AskState>();
    if state.busy.load(Ordering::SeqCst) != 0 {
        return Err("Still answering. Stop it first.".into());
    }
    let (main, _, backup) = choice(app);
    let plan = provider_plan(&main, backup, load_key().is_some(), load_named(GROQ_KEY).is_some())?;
    let now = now_ms();

    let (reply_id, contents) = with_chat(app, |c| {
        if c.is_stale(now) {
            c.clear();
        }
        c.push(Role::User, text, now);
        let contents = gemini_contents(&c.messages);
        let id = c.push(Role::Assistant, String::new(), now);
        (id, contents)
    });
    state.busy.store(reply_id, Ordering::SeqCst);
    let run = state.run.fetch_add(1, Ordering::SeqCst) + 1;
    save(app);
    emit(app);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = answer(&app, &plan, contents, reply_id, run).await;
        let state = app.state::<AskState>();
        if state.run.load(Ordering::SeqCst) != run {
            return; // stopped: `stop` already tidied up
        }
        with_chat(&app, |c| {
            if let Some(m) = c.get_mut(reply_id) {
                match result {
                    Ok(()) if m.text.trim().is_empty() => m.error = Some("The answer came back empty. Try asking again.".into()),
                    Ok(()) => {}
                    Err(e) => m.error = Some(e),
                }
            }
            c.updated_at = now_ms();
        });
        state.busy.store(0, Ordering::SeqCst);
        save(&app);
        emit(&app);
    });
    Ok(())
}

/// Stream one answer from Gemini into message `id`.
enum Attempt {
    /// Answering; `bool` = asked with minimal thinking.
    Ok(reqwest::Response, bool),
    /// Status (0 = no answer in time) and Google's reply.
    Failed(u16, String),
    Offline(String),
}

/// Ask one model to start answering. A model that won't take minimal
/// thinking is asked again with its default.
async fn try_model(key: String, model: String, contents: Vec<Value>, minimal: bool) -> (String, Attempt) {
    let client = match client() {
        Ok(c) => c,
        Err(e) => return (model, Attempt::Offline(e)),
    };
    let mut minimal = minimal;
    loop {
        let url = format!("{API}/models/{model}:streamGenerateContent?alt=sse");
        let sent = client.post(url).header("x-goog-api-key", &key).json(&request_body(&contents, minimal)).send().await;
        let r = match sent {
            Ok(r) => r,
            Err(e) => return (model, Attempt::Offline(offline(&e))),
        };
        let status = r.status().as_u16();
        if status == 200 {
            return (model, Attempt::Ok(r, minimal));
        }
        let text = r.text().await.unwrap_or_default();
        if minimal && rejects_thinking_setting(status, &text) {
            minimal = false;
            continue;
        }
        return (model, Attempt::Failed(status, text));
    }
}

// ---------- providers (stage 2: Groq and Ollama beside Gemini) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    Groq,
    Ollama,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::Groq => "groq",
            Provider::Ollama => "ollama",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Gemini => "Gemini",
            Provider::Groq => "Groq",
            Provider::Ollama => "Ollama",
        }
    }
}

/// Settings → Ask: (main provider, its model, use the other cloud provider as backup).
fn choice(app: &AppHandle) -> (String, String, bool) {
    let s = app.state::<crate::AppState>();
    let s = s.settings.lock().unwrap();
    let a = &s.ask;
    let model = match a.provider.as_str() {
        "groq" => a.groq_model.clone(),
        "ollama" => a.ollama_model.clone(),
        _ => a.model.clone(),
    };
    (a.provider.clone(), model, a.backup)
}

/// Which providers to try, in order. Local (Ollama) never falls back to the
/// cloud: questions meant to stay on this PC must not leave it.
pub fn provider_plan(main: &str, backup: bool, gemini_key: bool, groq_key: bool) -> Result<Vec<Provider>, String> {
    let mut out = Vec::new();
    match main {
        "ollama" => out.push(Provider::Ollama),
        "groq" => {
            if groq_key {
                out.push(Provider::Groq);
            }
            if backup && gemini_key {
                out.push(Provider::Gemini);
            }
            if out.is_empty() {
                return Err("Add a Groq API key in Settings → Ask.".into());
            }
        }
        _ => {
            if gemini_key {
                out.push(Provider::Gemini);
            }
            if backup && groq_key {
                out.push(Provider::Groq);
            }
            if out.is_empty() {
                return Err("Add a Gemini API key in Settings → Ask.".into());
            }
        }
    }
    Ok(out)
}

/// Chat in the OpenAI format (Groq, Ollama), from the Gemini-format contents.
pub fn openai_messages(contents: &[Value]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    for c in contents {
        let role = if c["role"] == "model" { "assistant" } else { "user" };
        let text: String = c["parts"].as_array().map(|ps| ps.iter().filter_map(|p| p["text"].as_str()).collect()).unwrap_or_default();
        out.push(json!({ "role": role, "content": text }));
    }
    out
}

/// Ask for little or no thinking where the model supports the setting
/// (answers start many times sooner); `None` = send nothing extra.
pub fn reasoning_setting(provider: Provider, model: &str) -> Option<&'static str> {
    match provider {
        Provider::Ollama => Some("none"),
        Provider::Groq if model.contains("gpt-oss") => Some("low"),
        Provider::Groq if model.contains("qwen") => Some("none"),
        _ => None,
    }
}

pub fn openai_body(model: &str, messages: &[Value], reasoning: Option<&str>) -> Value {
    let mut body = json!({ "model": model, "messages": messages, "stream": true });
    if let Some(r) = reasoning {
        body["reasoning_effort"] = json!(r);
    }
    body
}

/// Text in one OpenAI-format stream chunk; `None` at the end (`[DONE]`).
/// Reasoning text, when a model sends it, is left out.
pub fn openai_chunk_text(payload: &str) -> Result<Option<String>, String> {
    if payload.trim() == "[DONE]" {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(payload).map_err(|_| "Got a reply Atoll couldn't read.".to_string())?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).or_else(|| err.as_str()).unwrap_or("unknown error");
        return Err(format!("Error: {msg}"));
    }
    Ok(Some(v.pointer("/choices/0/delta/content").and_then(Value::as_str).unwrap_or("").to_string()))
}

/// A plain-English message for a failed Groq or Ollama request.
pub fn openai_error(provider: Provider, status: u16, body: &str, model: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message").and_then(Value::as_str).or_else(|| v.get("error").and_then(Value::as_str)).map(str::to_string)
        })
        .unwrap_or_default();
    let low = format!("{body} {detail}").to_lowercase();
    let who = provider.label();
    match (provider, status) {
        (Provider::Groq, 401) => "Groq didn't accept the API key. Check it in Settings → Ask.".into(),
        (Provider::Groq, 429) => {
            "You've used Groq's free limit for now. Try again in a minute, or tomorrow if the daily limit is used up.".into()
        }
        (Provider::Ollama, 404) | (_, 404) if low.contains("not found") || low.contains("model_not_found") || low.contains("does not exist") => {
            if provider == Provider::Ollama {
                format!("Ollama doesn't have \u{201c}{model}\u{201d}. Run \u{201c}ollama pull {model}\u{201d}, or pick another model in Settings → Ask.")
            } else {
                format!("{who} doesn't offer the model \u{201c}{model}\u{201d}. Pick another in Settings → Ask.")
            }
        }
        (_, 500..=599) => format!("{who} is having trouble (error {status}). Try again in a moment."),
        _ if !detail.is_empty() => format!("{who}: {detail}"),
        _ => format!("{who} returned error {status}."),
    }
}

/// The Groq model to use: the one picked, else the first preferred one this
/// key has, else any chat model it has.
pub fn pick_groq_model(picked: &str, available: &[String]) -> Option<String> {
    if !picked.is_empty() {
        return Some(picked.to_string());
    }
    GROQ_PREFERRED
        .iter()
        .find(|m| available.iter().any(|a| a == *m))
        .map(|m| m.to_string())
        .or_else(|| available.first().cloned())
}

/// Chat models in a Groq `/models` reply (speech, guard and TTS models left out).
pub fn groq_chat_models(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .filter(|m| m.get("active").and_then(Value::as_bool) != Some(false))
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .filter(|id| {
                    let id = id.to_lowercase();
                    !["whisper", "tts", "guard", "playai", "orpheus", "distil", "embed"].iter().any(|x| id.contains(x))
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Models installed in Ollama (`/api/tags`).
pub fn ollama_models_from(list: &Value) -> Vec<String> {
    list.get("models")
        .and_then(Value::as_array)
        .map(|ms| ms.iter().filter_map(|m| m.get("name").and_then(Value::as_str)).map(str::to_string).collect())
        .unwrap_or_default()
}

pub async fn groq_models_with(key: &str) -> Result<Vec<String>, String> {
    let resp = client()?
        .get(format!("{GROQ_API}/models"))
        .bearer_auth(key)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "Couldn't reach Groq. Check your internet connection.".to_string())?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(openai_error(Provider::Groq, status, &body, ""));
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| "Groq sent a reply Atoll couldn't read.".to_string())?;
    Ok(groq_chat_models(&v))
}

pub async fn ollama_models() -> Result<Vec<String>, String> {
    let resp = client()?
        .get(format!("{OLLAMA_API}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "Ollama isn't running. Start Ollama, or pick another provider in Settings → Ask.".to_string())?;
    let v: Value = resp.json().await.map_err(|_| "Ollama sent a reply Atoll couldn't read.".to_string())?;
    Ok(ollama_models_from(&v))
}

/// Start a Groq or Ollama reply.
async fn start_openai(app: &AppHandle, provider: Provider, contents: &[Value], run: u64) -> Result<Option<Started>, String> {
    let state = app.state::<AskState>();
    let picked = {
        let s = app.state::<crate::AppState>();
        let s = s.settings.lock().unwrap();
        if provider == Provider::Groq { s.ask.groq_model.clone() } else { s.ask.ollama_model.clone() }
    };
    let (url, key, model) = match provider {
        Provider::Groq => {
            let key = load_named(GROQ_KEY).ok_or("Add a Groq API key in Settings → Ask.")?;
            let model = if picked.is_empty() {
                let cached = state.groq_models.lock().unwrap().clone();
                let available = if cached.is_empty() { groq_models_with(&key).await? } else { cached };
                *state.groq_models.lock().unwrap() = available.clone();
                pick_groq_model("", &available).ok_or("Groq has no chat models for this key.")?
            } else {
                picked
            };
            (format!("{GROQ_API}/chat/completions"), Some(key), model)
        }
        _ => {
            let model = if picked.is_empty() {
                ollama_models().await?.into_iter().next().ok_or("Ollama has no models yet. Run \u{201c}ollama pull qwen3.5:9b\u{201d} (or another model) first.")?
            } else {
                picked
            };
            (format!("{OLLAMA_API}/v1/chat/completions"), None, model)
        }
    };
    let messages = openai_messages(contents);
    let mut reasoning = reasoning_setting(provider, &model);
    let c = client()?;
    loop {
        let mut req = c.post(&url).json(&openai_body(&model, &messages, reasoning));
        if let Some(k) = &key {
            req = req.bearer_auth(k);
        }
        let sent = tokio::select! {
            r = tokio::time::timeout(STREAM_IDLE, req.send()) => r,
            _ = state.cancel.notified() => return Ok(None),
        };
        if state.run.load(Ordering::SeqCst) != run {
            return Ok(None);
        }
        let resp = match sent {
            Ok(Ok(r)) => r,
            Ok(Err(e)) if provider == Provider::Ollama && e.is_connect() => {
                return Err("Ollama isn't running. Start Ollama, or pick another provider in Settings → Ask.".into())
            }
            Ok(Err(e)) => return Err(offline(&e).replace("Gemini", provider.label())),
            Err(_) => return Err(format!("{} took too long to answer. Try again.", provider.label())),
        };
        let status = resp.status().as_u16();
        if status == 200 {
            return Ok(Some(Started { resp, provider, model }));
        }
        let body = resp.text().await.unwrap_or_default();
        // A model that doesn't take the thinking setting: ask again without it.
        if reasoning.is_some() && status == 400 && body.to_lowercase().contains("reasoning") {
            reasoning = None;
            continue;
        }
        if provider == Provider::Groq && status == 404 {
            state.groq_models.lock().unwrap().clear();
        }
        return Err(openai_error(provider, status, &body, &model));
    }
}

// ---------- usage counters ----------

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Usage {
    /// Local date the counts are for (YYYY-MM-DD).
    pub day: String,
    pub gemini: u32,
    pub groq: u32,
    pub ollama: u32,
    /// From Groq's own rate-limit headers on the last answer (requests per day).
    pub groq_limit: Option<u32>,
    pub groq_remaining: Option<u32>,
}

impl Usage {
    /// Counts start again on a new day.
    pub fn roll(&mut self, today: &str) {
        if self.day != today {
            *self = Usage { day: today.to_string(), ..Default::default() };
        }
    }

    pub fn count(&mut self, today: &str, provider: Provider, limit: Option<u32>, remaining: Option<u32>) {
        self.roll(today);
        match provider {
            Provider::Gemini => self.gemini += 1,
            Provider::Groq => {
                self.groq += 1;
                if limit.is_some() {
                    self.groq_limit = limit;
                    self.groq_remaining = remaining;
                }
            }
            Provider::Ollama => self.ollama += 1,
        }
    }
}

fn usage_file() -> PathBuf {
    crate::paths::local_dir().join("ask").join("usage.json")
}

fn header_u32(h: &reqwest::header::HeaderMap, name: &str) -> Option<u32> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn record_use(app: &AppHandle, provider: Provider, headers: &reqwest::header::HeaderMap) {
    let state = app.state::<AskState>();
    let mut u = state.usage.lock().unwrap();
    if u.day.is_empty() {
        *u = std::fs::read(usage_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    }
    let limit = header_u32(headers, "x-ratelimit-limit-requests");
    let remaining = header_u32(headers, "x-ratelimit-remaining-requests");
    u.count(&crate::focus::today_key(), provider, limit, remaining);
    let path = usage_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec(&*u).unwrap_or_default());
}

/// Today's counts (Settings → Ask).
pub fn usage(app: &AppHandle) -> Usage {
    let state = app.state::<AskState>();
    let mut u = state.usage.lock().unwrap();
    if u.day.is_empty() {
        *u = std::fs::read(usage_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    }
    u.roll(&crate::focus::today_key());
    u.clone()
}

// ---------- Groq key ----------

pub fn groq_key_status() -> KeyStatus {
    match load_named(GROQ_KEY) {
        Some(k) => KeyStatus { has_key: true, masked: mask_key(&k) },
        None => KeyStatus { has_key: false, masked: String::new() },
    }
}

/// Test a Groq key and keep it in Credential Manager if it works.
pub async fn set_groq_key(app: &AppHandle, key: &str) -> Result<Vec<String>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your Groq API key first.".into());
    }
    if key.len() > 200 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like an API key.".into());
    }
    let models = groq_models_with(key).await?;
    named_entry(GROQ_KEY)
        .ok_or("Windows Credential Manager is unavailable.")?
        .set_password(key)
        .map_err(|e| format!("Couldn't save to Windows Credential Manager: {e}"))?;
    *app.state::<AskState>().groq_models.lock().unwrap() = models.clone();
    emit(app);
    Ok(models)
}

pub fn forget_groq_key(app: &AppHandle) {
    if let Some(e) = named_entry(GROQ_KEY) {
        let _ = e.delete_credential();
    }
    app.state::<AskState>().groq_models.lock().unwrap().clear();
    emit(app);
}

pub async fn groq_models() -> Result<Vec<String>, String> {
    let key = load_named(GROQ_KEY).ok_or("Add a Groq API key first.")?;
    groq_models_with(&key).await
}

/// A reply that has started: the response to read and how to read it.
struct Started {
    resp: reqwest::Response,
    provider: Provider,
    model: String,
}

/// Answer with the first provider in `plan` that starts; a provider that
/// fails before writing anything hands over to the next.
async fn answer(app: &AppHandle, plan: &[Provider], contents: Vec<Value>, id: u64, run: u64) -> Result<(), String> {
    let mut first_err: Option<String> = None;
    for p in plan {
        let started = match p {
            Provider::Gemini => {
                let key = load_key().unwrap_or_default();
                let model = model(app);
                start_gemini(app, &key, &model, contents.clone(), run).await
            }
            Provider::Groq | Provider::Ollama => start_openai(app, *p, &contents, run).await,
        };
        match started {
            Ok(Some(st)) => {
                record_use(app, st.provider, st.resp.headers());
                let (provider, model) = (st.provider.name().to_string(), st.model.clone());
                with_chat(app, |c| {
                    if let Some(m) = c.get_mut(id) {
                        m.model = model;
                        m.provider = provider;
                    }
                });
                emit(app);
                return pump(app, st, id, run).await;
            }
            Ok(None) => return Ok(()), // stopped
            Err(e) => {
                eprintln!("[crest] ask: {} could not answer: {e}", p.name());
                first_err.get_or_insert(e);
            }
        }
        if app.state::<AskState>().run.load(Ordering::SeqCst) != run {
            return Ok(());
        }
    }
    Err(first_err.unwrap_or_else(|| "No provider could answer.".into()))
}

/// Start a Gemini reply: the chosen model, else the fastest free stand-in.
async fn start_gemini(app: &AppHandle, key: &str, model: &str, contents: Vec<Value>, run: u64) -> Result<Option<Started>, String> {
    let state = app.state::<AskState>();
    let chosen = model;

    // Fetch the model list in the background so a fallback doesn't wait for it.
    if state.models.lock().unwrap().is_empty() {
        let (app2, key2) = (app.clone(), key.to_string());
        tauri::async_runtime::spawn(async move {
            if let Ok(m) = list_models(&key2).await {
                *app2.state::<AskState>().models.lock().unwrap() = m;
            }
        });
    }

    // The chosen model (or a recent stand-in) goes first. If it hasn't started
    // answering within a moment, or fails in a way another model may not,
    // every other candidate is asked too and the first to start wins (Google's
    // load moves between models from hour to hour).
    let first = sticky_start(&state, chosen);
    let full: Vec<String> = state.full_thinking.lock().unwrap().clone();
    let hedge = hedge_after(&first);
    let mut set = tokio::task::JoinSet::new();
    set.spawn(try_model(key.to_string(), first.clone(), contents.clone(), !full.contains(&first)));
    let mut widened = false;
    let mut failures: Vec<(String, u16, String)> = Vec::new();
    let deadline = tokio::time::Instant::now() + STREAM_IDLE;
    let won = loop {
        let wait = if widened { deadline.saturating_duration_since(tokio::time::Instant::now()) } else { hedge };
        let next = tokio::select! {
            r = tokio::time::timeout(wait, set.join_next()) => r,
            _ = state.cancel.notified() => return Ok(None),
        };
        let mut widen = false;
        match next {
            // Nothing yet, or every request has failed.
            Err(_) | Ok(None) => {
                if widened {
                    break None;
                }
                widen = true;
            }
            Ok(Some(Err(_))) => {}
            Ok(Some(Ok((m, outcome)))) => match outcome {
                Attempt::Ok(r, minimal) => break Some((m, r, minimal)),
                Attempt::Failed(status, text) => {
                    eprintln!("[crest] ask: {m} returned {status}");
                    if !widened && status != 0 && !worth_another_model(status) {
                        return Err(friendly_error(status, &text, &m));
                    }
                    failures.push((m, status, text));
                    widen = !widened;
                }
                Attempt::Offline(e) => {
                    if !widened {
                        return Err(e);
                    }
                    failures.push((m, 0, String::new()));
                }
            },
        }
        if widen {
            widened = true;
            let cached = state.models.lock().unwrap().clone();
            let available = if cached.is_empty() {
                let fetched = list_models(key).await.unwrap_or_default();
                *state.models.lock().unwrap() = fetched.clone();
                fetched
            } else {
                cached
            };
            for m in fallback_order(chosen, &available).into_iter().filter(|m| *m != first) {
                set.spawn(try_model(key.to_string(), m.clone(), contents.clone(), !full.contains(&m)));
            }
        }
        if state.run.load(Ordering::SeqCst) != run {
            return Ok(None);
        }
    };
    // Dropping the set cancels the slower requests.
    drop(set);
    let (model, resp, minimal) = match won {
        Some(w) => w,
        None => {
            let busy = failures.iter().filter(|f| matches!(f.1, 0 | 500 | 503)).count();
            return Err(if busy == failures.len() && failures.len() > 1 {
                format!("Every Gemini model tried is busy right now ({} tried). Try again in a minute.", failures.len())
            } else {
                failures
                    .iter()
                    .rev()
                    .find(|f| f.1 != 0)
                    .map(|f| friendly_error(f.1, &f.2, &f.0))
                    .unwrap_or_else(|| "Gemini took too long to answer. Try again.".into())
            });
        }
    };
    if state.run.load(Ordering::SeqCst) != run {
        return Ok(None);
    }
    if !minimal {
        let mut f = state.full_thinking.lock().unwrap();
        if !f.contains(&model) {
            f.push(model.clone());
        }
    }
    set_sticky(&state, chosen, &model);
    Ok(Some(Started { resp, provider: Provider::Gemini, model }))
}

/// Read a started reply into message `id` as it streams.
async fn pump(app: &AppHandle, st: Started, id: u64, run: u64) -> Result<(), String> {
    let state = app.state::<AskState>();
    let who = st.provider.label();
    let mut resp = st.resp;
    let mut parser = SseParser::default();
    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(STREAM_IDLE, resp.chunk()) => match c {
                Ok(Ok(c)) => c,
                Ok(Err(_)) => return Err(format!("The connection to {who} dropped mid-answer.")),
                Err(_) => return Err(format!("{who} stopped sending. Try again.")),
            },
            _ = state.cancel.notified() => return Ok(()),
        };
        let ended = chunk.is_none();
        let events = match chunk {
            Some(bytes) => parser.push(&bytes),
            None => parser.finish().into_iter().collect(),
        };
        for ev in events {
            let piece = match st.provider {
                Provider::Gemini => chunk_text(&ev)?,
                _ => match openai_chunk_text(&ev)? {
                    Some(t) => t,
                    None => return Ok(()), // [DONE]
                },
            };
            if piece.is_empty() {
                continue;
            }
            if state.run.load(Ordering::SeqCst) != run {
                return Ok(());
            }
            let text = with_chat(app, |c| {
                c.get_mut(id).map(|m| {
                    m.text.push_str(&piece);
                    m.text.clone()
                })
            });
            if let Some(text) = text {
                let _ = app.emit("ask:delta", Delta { id, text });
            }
        }
        if ended {
            return Ok(());
        }
    }
}

/// Stop the answer being written; what arrived so far stays.
pub fn stop(app: &AppHandle) {
    let state = app.state::<AskState>();
    let id = state.busy.swap(0, Ordering::SeqCst);
    state.run.fetch_add(1, Ordering::SeqCst);
    state.cancel.notify_waiters();
    if id != 0 {
        with_chat(app, |c| {
            let empty = c.get_mut(id).map(|m| m.text.trim().is_empty());
            match empty {
                Some(true) => c.messages.retain(|m| m.id != id),
                Some(false) => {
                    if let Some(m) = c.get_mut(id) {
                        m.error = Some("Stopped.".into());
                    }
                }
                None => {}
            }
        });
    }
    save(app);
    emit(app);
}

/// Start a new chat.
pub fn clear(app: &AppHandle) {
    stop(app);
    with_chat(app, Chat::clear);
    save(app);
    emit(app);
}

/// Settings → Privacy → Clear all data: chat and key go.
pub fn clear_everything(app: &AppHandle) {
    clear(app);
    let _ = std::fs::remove_file(chat_file());
    let _ = std::fs::remove_file(route_file());
    let _ = std::fs::remove_file(usage_file());
    *app.state::<AskState>().usage.lock().unwrap() = Usage::default();
    forget_key(app);
    forget_groq_key(app);
}

/// Settings changed (history switch or model): tidy and tell the island.
pub fn settings_changed(app: &AppHandle) {
    save(app);
    emit(app);
}

// ---------- settings ----------

/// One-time defaults kept out of the shared settings version: the Ask
/// hotkey (Ctrl+Alt+A) unless that combination is already yours.
pub fn seed_settings(s: &mut crate::settings::Settings) {
    if s.ask.seed_version < 2 {
        s.ask.seed_version = 2;
        if s.ask.model == OLD_DEFAULT_MODEL {
            s.ask.model = DEFAULT_MODEL.into();
        }
    }
    if s.ask.seeded {
        return;
    }
    s.ask.seeded = true;
    let taken = s
        .hotkeys
        .modules
        .values()
        .chain([&s.hotkeys.toggle_island, &s.hotkeys.expand_island, &s.hotkeys.open_settings])
        .any(|v| v.eq_ignore_ascii_case(DEFAULT_HOTKEY));
    if !taken {
        s.hotkeys.modules.entry("ask".into()).or_insert_with(|| DEFAULT_HOTKEY.into());
    }
}

/// Keep a model id to the characters Google uses.
pub fn clean_model(m: &str) -> String {
    let m: String = m.trim().trim_start_matches("models/").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).take(80).collect();
    if m.is_empty() {
        DEFAULT_MODEL.into()
    } else {
        m
    }
}

/// Keep a Groq or Ollama model id to safe characters ("openai/gpt-oss-120b", "qwen3.5:9b"); "" = automatic.
pub fn clean_model_id(m: &str) -> String {
    m.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':')).take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: u64, role: Role, text: &str) -> Msg {
        Msg { id, role, text: text.into(), at: 0, error: None, model: String::new(), provider: String::new() }
    }

    #[test]
    fn sse_handles_split_lines_crlf_and_utf8() {
        let mut p = SseParser::default();
        let full = "data: {\"a\":\"caf\u{e9}\"}\r\n\r\ndata: {\"b\":1}\n\n".as_bytes();
        // Cut inside the two-byte "é" and inside the CRLF.
        let cut1 = full.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let mut events = p.push(&full[..cut1]);
        assert!(events.is_empty());
        events.extend(p.push(&full[cut1..cut1 + 6]));
        events.extend(p.push(&full[cut1 + 6..]));
        assert_eq!(events, vec!["{\"a\":\"caf\u{e9}\"}".to_string(), "{\"b\":1}".to_string()]);
        assert_eq!(p.finish(), None);
    }

    #[test]
    fn sse_joins_multiline_data_and_ignores_comments() {
        let mut p = SseParser::default();
        let ev = p.push(b": keep-alive\nevent: x\ndata: one\ndata:two\n\n");
        assert_eq!(ev, vec!["one\ntwo".to_string()]);
    }

    #[test]
    fn sse_finish_flushes_unterminated_event() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: last").is_empty());
        assert_eq!(p.finish().as_deref(), Some("last"));
    }

    #[test]
    fn chunk_text_reads_parts_and_skips_thoughts() {
        let c = r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"hmm","thought":true},{"text":"Hello"},{"text":" there"}]}}]}"#;
        assert_eq!(chunk_text(c).unwrap(), "Hello there");
        // Usage-only chunk at the end of a stream.
        assert_eq!(chunk_text(r#"{"usageMetadata":{"totalTokenCount":5}}"#).unwrap(), "");
    }

    #[test]
    fn chunk_text_reports_blocks_and_errors() {
        let blocked = r#"{"promptFeedback":{"blockReason":"PROHIBITED_CONTENT"}}"#;
        assert!(chunk_text(blocked).unwrap_err().contains("prohibited content"));
        let safety = r#"{"candidates":[{"finishReason":"SAFETY"}]}"#;
        assert!(chunk_text(safety).unwrap_err().contains("safety"));
        let err = r#"{"error":{"code":500,"message":"Internal"}}"#;
        assert_eq!(chunk_text(err).unwrap_err(), "Gemini error: Internal");
        assert!(chunk_text("not json").is_err());
    }

    #[test]
    fn friendly_errors() {
        let bad_key = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert!(friendly_error(400, bad_key, "m").contains("didn't accept the API key"));
        assert!(friendly_error(429, r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#, "m").contains("free limit"));
        assert!(friendly_error(404, "{}", "gemini-x").contains("gemini-x"));
        assert!(friendly_error(503, "", "m").contains("503"));
        assert_eq!(friendly_error(400, r#"{"error":{"message":"Bad thing"}}"#, "m"), "Gemini: Bad thing");
        assert!(friendly_error(400, r#"{"error":{"message":"User location is not supported for the API use."}}"#, "m").contains("region"));
    }

    #[test]
    fn contents_start_with_user_merge_roles_and_skip_failures() {
        let mut failed = msg(3, Role::Assistant, "");
        failed.error = Some("429".into());
        let msgs = vec![
            msg(1, Role::Assistant, "orphan reply"),
            msg(2, Role::User, "first"),
            failed,
            msg(4, Role::User, "second"),
            msg(5, Role::Assistant, "answer"),
            msg(6, Role::User, "third"),
        ];
        let c = gemini_contents(&msgs);
        let roles: Vec<&str> = c.iter().map(|v| v["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "model", "user"]);
        assert_eq!(c[0]["parts"][0]["text"], "first\n\nsecond");
        assert_eq!(c[2]["parts"][0]["text"], "third");
    }

    #[test]
    fn contents_keep_only_newest() {
        let msgs: Vec<Msg> = (0..100)
            .map(|i| msg(i, if i % 2 == 0 { Role::User } else { Role::Assistant }, &format!("m{i}")))
            .collect();
        let c = gemini_contents(&msgs);
        assert!(c.len() <= MAX_SENT);
        assert_eq!(c[0]["role"], "user");
        assert_eq!(c.last().unwrap()["parts"][0]["text"], "m99");
    }

    #[test]
    fn chat_caps_length_and_goes_stale() {
        let mut chat = Chat::default();
        for i in 0..(MAX_KEPT + 5) {
            chat.push(Role::User, format!("q{i}"), 1_000);
        }
        assert_eq!(chat.messages.len(), MAX_KEPT);
        assert_eq!(chat.messages[0].text, "q5");
        assert!(!chat.is_stale(1_000 + IDLE_NEW_CHAT_MS));
        assert!(chat.is_stale(1_001 + IDLE_NEW_CHAT_MS));
        assert!(!Chat::default().is_stale(u64::MAX));
    }

    #[test]
    fn models_filtered_and_sorted() {
        let v = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-embedding-001", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemma-3-27b-it", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-flash-latest", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"] }
        ]});
        assert_eq!(chat_models(&v), vec!["gemini-flash-latest", "gemini-2.5-flash"]);
    }

    #[test]
    fn masks_and_cleans() {
        assert_eq!(mask_key("AIzaSyABCDEFGHwxyz"), "AIza…wxyz");
        assert_eq!(mask_key("short"), "••••");
        assert_eq!(clean_model(" models/gemini-2.5-flash "), "gemini-2.5-flash");
        assert_eq!(clean_model("gemini/../x?y"), "gemini..xy");
        assert_eq!(clean_model(""), DEFAULT_MODEL);
    }

    #[test]
    fn fallback_order_from_real_model_list() {
        // The list a free key got on 2026-11-25.
        let available: Vec<String> = "gemini-robotics-er-2-preview, gemini-pro-latest, gemini-omni-flash-preview, gemini-omni-1.2-flash, gemini-omni-1.1-flash, gemini-nano-banana-2.1, gemini-flash-lite-latest, gemini-flash-latest, gemini-4-neon, gemini-4-hydrogen, gemini-4-carbon, gemini-4-argon, gemini-3.8-flash, gemini-3.7-flash, gemini-3.6-flash, gemini-3.5-transcribe, gemini-3.5-flash-lite, gemini-3.5-flash, gemini-3.1-pro-preview-customtools, gemini-3.1-pro-preview, gemini-3.1-flash-lite-preview, gemini-3.1-flash-lite, gemini-3-flash-preview, gemini-2.5-pro, gemini-2.5-flash-lite, gemini-2.5-flash, gemini-2.5-computer-use-preview-10-2025"
            .split(", ")
            .map(String::from)
            .collect();
        assert_eq!(
            fallback_order("gemini-flash-latest", &available),
            [
                "gemini-flash-latest",
                "gemini-flash-lite-latest",
                "gemini-3.8-flash",
                "gemini-3.7-flash",
                "gemini-3.6-flash",
                "gemini-3.5-flash",
                "gemini-3.5-flash-lite",
                "gemini-3.1-flash-lite",
            ]
        );
        // A Pro choice falls back to Flash; previews, omni, transcribe are never picked.
        let pro = fallback_order("gemini-2.5-pro", &available);
        assert_eq!(pro[..3], ["gemini-2.5-pro", "gemini-flash-lite-latest", "gemini-flash-latest"]);
        assert!(pro.iter().all(|m| !m.contains("preview") && !m.contains("omni") && !m.contains("transcribe")));
        // Version-aware: 3.10 is newer than 3.9.
        let v: Vec<String> = ["gemini-3.9-flash", "gemini-3.10-flash"].map(String::from).to_vec();
        assert_eq!(fallback_order("x", &v), ["x", "gemini-3.10-flash", "gemini-3.9-flash"]);
        // No list (fetch failed): only the chosen model.
        assert_eq!(fallback_order("gemini-flash-latest", &[]), ["gemini-flash-latest"]);
    }

    #[test]
    fn hedges_sooner_for_flash_than_pro() {
        assert_eq!(hedge_after("gemini-flash-lite-latest"), HEDGE);
        assert_eq!(hedge_after("gemini-3.5-flash"), HEDGE);
        assert_eq!(hedge_after("gemini-2.5-pro"), HEDGE_PRO);
        assert_eq!(hedge_after("gemini-pro-latest"), HEDGE_PRO);
    }

    #[test]
    fn old_default_model_moves_to_flash_lite_once() {
        // A settings file from before the change: no seedVersion, old default model.
        let old: crate::settings::Settings =
            serde_json::from_value(json!({ "ask": { "model": "gemini-flash-latest", "seeded": true } })).unwrap();
        assert_eq!(old.ask.seed_version, 0);
        let mut s = old.clone();
        seed_settings(&mut s);
        assert_eq!(s.ask.model, DEFAULT_MODEL);
        assert_eq!(s.ask.seed_version, 2);
        // Picked again by the user later: left alone.
        s.ask.model = "gemini-flash-latest".into();
        seed_settings(&mut s);
        assert_eq!(s.ask.model, "gemini-flash-latest");
        // Someone who chose another model keeps it.
        let mut t: crate::settings::Settings = serde_json::from_value(json!({ "ask": { "model": "gemini-3.5-flash" } })).unwrap();
        seed_settings(&mut t);
        assert_eq!(t.ask.model, "gemini-3.5-flash");
        assert_eq!(crate::settings::Settings::default().ask.model, DEFAULT_MODEL);
    }

    #[test]
    fn which_failures_try_another_model() {
        for s in [404, 429, 500, 503] {
            assert!(worth_another_model(s), "{s}");
        }
        for s in [400, 401, 403] {
            assert!(!worth_another_model(s), "{s}");
        }
        let busy = r#"{"error":{"code":503,"message":"This model is currently experiencing high demand. Spikes in demand are usually temporary. Please try again later.","status":"UNAVAILABLE"}}"#;
        assert_eq!(friendly_error(503, busy, "m"), "Gemini is busy right now (high demand). Try again in a moment.");
    }

    #[test]
    fn provider_plan_orders_and_explains() {
        use Provider::*;
        assert_eq!(provider_plan("gemini", true, true, true).unwrap(), [Gemini, Groq]);
        assert_eq!(provider_plan("gemini", false, true, true).unwrap(), [Gemini]);
        assert_eq!(provider_plan("gemini", true, false, true).unwrap(), [Groq], "no Gemini key: Groq backup still answers");
        assert!(provider_plan("gemini", true, false, false).unwrap_err().contains("Gemini API key"));
        assert!(provider_plan("gemini", false, false, true).unwrap_err().contains("Gemini API key"));
        assert_eq!(provider_plan("groq", true, true, true).unwrap(), [Groq, Gemini]);
        assert!(provider_plan("groq", false, true, false).unwrap_err().contains("Groq API key"));
        // Local never falls back to the cloud, whatever keys exist.
        assert_eq!(provider_plan("ollama", true, true, true).unwrap(), [Ollama]);
        assert_eq!(provider_plan("ollama", true, false, false).unwrap(), [Ollama]);
    }

    #[test]
    fn openai_format_messages_and_body() {
        let contents = gemini_contents(&[msg(1, Role::User, "hi"), msg(2, Role::Assistant, "hello"), msg(3, Role::User, "more")]);
        let m = openai_messages(&contents);
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert_eq!(m[2]["content"], "hello");
        let b = openai_body("qwen3.5:9b", &m, Some("none"));
        assert_eq!(b["stream"], true);
        assert_eq!(b["reasoning_effort"], "none");
        assert!(openai_body("x", &m, None).get("reasoning_effort").is_none());
        assert_eq!(reasoning_setting(Provider::Ollama, "anything"), Some("none"));
        assert_eq!(reasoning_setting(Provider::Groq, "openai/gpt-oss-120b"), Some("low"));
        assert_eq!(reasoning_setting(Provider::Groq, "llama-3.3-70b-versatile"), None);
    }

    #[test]
    fn openai_chunks() {
        let c = r#"{"id":"x","choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"}}]}"#;
        assert_eq!(openai_chunk_text(c).unwrap().as_deref(), Some("Hel"));
        let reasoning = r#"{"choices":[{"delta":{"reasoning":"thinking..."}}]}"#;
        assert_eq!(openai_chunk_text(reasoning).unwrap().as_deref(), Some(""));
        assert_eq!(openai_chunk_text("[DONE]").unwrap(), None);
        assert_eq!(openai_chunk_text(r#"{"error":{"message":"boom"}}"#).unwrap_err(), "Error: boom");
        assert_eq!(openai_chunk_text(r#"{"error":"model unloaded"}"#).unwrap_err(), "Error: model unloaded");
    }

    #[test]
    fn openai_errors() {
        // Real reply to a bad key on 2026-11-25.
        let bad = r#"{"error":{"message":"Invalid API Key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert!(openai_error(Provider::Groq, 401, bad, "m").contains("Groq didn't accept the API key"));
        assert!(openai_error(Provider::Groq, 429, "{}", "m").contains("Groq's free limit"));
        let gone = r#"{"error":{"message":"The model `x` does not exist","code":"model_not_found"}}"#;
        assert!(openai_error(Provider::Groq, 404, gone, "x").contains("doesn't offer the model"));
        let missing = r#"{"error":{"message":"model \"llama9\" not found, try pulling it first"}}"#;
        assert!(openai_error(Provider::Ollama, 404, missing, "llama9").contains("ollama pull llama9"));
        assert!(openai_error(Provider::Groq, 503, "", "m").contains("Groq is having trouble"));
        assert_eq!(openai_error(Provider::Ollama, 400, r#"{"error":"bad"}"#, "m"), "Ollama: bad");
    }

    #[test]
    fn model_lists_and_picks() {
        let groq = json!({ "object": "list", "data": [
            { "id": "whisper-large-v3", "active": true },
            { "id": "openai/gpt-oss-120b", "active": true },
            { "id": "meta-llama/llama-guard-4-12b", "active": true },
            { "id": "llama-3.3-70b-versatile", "active": true },
            { "id": "old-model", "active": false },
            { "id": "playai-tts", "active": true }
        ]});
        let ids = groq_chat_models(&groq);
        assert_eq!(ids, ["llama-3.3-70b-versatile", "openai/gpt-oss-120b"]);
        assert_eq!(pick_groq_model("", &ids).as_deref(), Some("llama-3.3-70b-versatile"));
        assert_eq!(pick_groq_model("mine", &ids).as_deref(), Some("mine"));
        assert_eq!(pick_groq_model("", &["new-model".to_string()]).as_deref(), Some("new-model"));
        assert_eq!(pick_groq_model("", &[]), None);
        let tags = json!({ "models": [{ "name": "qwen3.5:9b", "model": "qwen3.5:9b" }, { "name": "llama3.2:3b" }] });
        assert_eq!(ollama_models_from(&tags), ["qwen3.5:9b", "llama3.2:3b"]);
        assert_eq!(clean_model_id(" openai/gpt-oss-120b "), "openai/gpt-oss-120b");
        assert_eq!(clean_model_id("qwen3.5:9b\"<x>"), "qwen3.5:9bx");
    }

    #[test]
    fn usage_counts_per_day() {
        let mut u = Usage::default();
        u.count("2026-11-25", Provider::Gemini, None, None);
        u.count("2026-11-25", Provider::Groq, Some(1000), Some(999));
        u.count("2026-11-25", Provider::Groq, None, None);
        u.count("2026-11-25", Provider::Ollama, None, None);
        assert_eq!((u.gemini, u.groq, u.ollama), (1, 2, 1));
        assert_eq!((u.groq_limit, u.groq_remaining), (Some(1000), Some(999)), "a reply without headers keeps the last known");
        u.roll("2026-11-26");
        assert_eq!(u, Usage { day: "2026-11-26".into(), ..Default::default() });
    }

    /// Local model end to end: `cargo test -p crest --lib probe_ollama -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_ollama() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let models = ollama_models().await.expect("Ollama not running");
            let model = models.first().expect("no models").clone();
            let contents = gemini_contents(&[msg(1, Role::User, "Explain what a mutex is in two sentences.")]);
            let t = std::time::Instant::now();
            let mut r = client()
                .unwrap()
                .post(format!("{OLLAMA_API}/v1/chat/completions"))
                .json(&openai_body(&model, &openai_messages(&contents), reasoning_setting(Provider::Ollama, &model)))
                .send()
                .await
                .unwrap();
            println!("{model}: status {} after {} ms", r.status(), t.elapsed().as_millis());
            let mut p = SseParser::default();
            let (mut text, mut first) = (String::new(), None);
            'outer: while let Ok(Some(b)) = r.chunk().await {
                for ev in p.push(&b) {
                    match openai_chunk_text(&ev).unwrap() {
                        Some(piece) => {
                            if first.is_none() && !piece.is_empty() {
                                first = Some(t.elapsed().as_millis());
                            }
                            text.push_str(&piece);
                        }
                        None => break 'outer,
                    }
                }
            }
            println!("first words {} ms, done {} ms: {}", first.unwrap_or(0), t.elapsed().as_millis(), text.trim());
        });
    }

    /// Live check with the key saved in Settings → Ask (never printed):
    /// `cargo test -p crest --lib probe_gemini -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_gemini() {
        let key = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            match list_models(&key).await {
                Ok(m) => println!("models ({}): {}", m.len(), m.join(", ")),
                Err(e) => println!("models: {e}"),
            }
            for model in ["gemini-flash-latest", "gemini-3.8-flash", "gemini-3.7-flash", "gemini-3.5-flash", "gemini-flash-lite-latest", "gemini-3.5-flash-lite"] {
                let body = json!({ "contents": [{ "role": "user", "parts": [{ "text": "Say hi in three words." }] }] });
                let r = client().unwrap()
                    .post(format!("{API}/models/{model}:generateContent"))
                    .header("x-goog-api-key", &key)
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                let status = r.status().as_u16();
                let text = r.text().await.unwrap_or_default();
                let short: String = text.chars().take(300).collect();
                println!("{model}: {status} {}", short.replace('\n', " "));
            }
        });
    }

    #[test]
    fn asks_for_minimal_thinking_and_spots_refusals() {
        let c = vec![json!({ "role": "user", "parts": [{ "text": "hi" }] })];
        let fast = request_body(&c, true);
        assert_eq!(fast["generationConfig"]["thinkingConfig"]["thinkingLevel"], "minimal");
        assert_eq!(fast["contents"][0]["parts"][0]["text"], "hi");
        assert!(request_body(&c, false).get("generationConfig").is_none());
        // Real refusals seen on 2026-11-25.
        assert!(rejects_thinking_setting(400, r#"{"error":{"code":400,"message":"Thinking level MINIMAL is not supported for this model."}}"#));
        assert!(rejects_thinking_setting(400, r#"{"error":{"code":400,"message":"Request contains an invalid argument."}}"#));
        assert!(!rejects_thinking_setting(400, r#"{"error":{"message":"API key not valid"}}"#));
        assert!(!rejects_thinking_setting(503, "thinking"));
    }

    /// New flow timing: `cargo test -p crest --lib probe_route -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_route() {
        let key = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let contents = vec![json!({ "role": "user", "parts": [{ "text": "Explain what a mutex is in two sentences." }] })];
            for round in 0..2 {
                let t = std::time::Instant::now();
                let first = tokio::time::timeout(HEDGE_PRO, try_model(key.clone(), "gemini-flash-latest".into(), contents.clone(), true)).await;
                let label = match &first {
                    Ok((_, Attempt::Ok(_, m))) => format!("answered (minimal={m})"),
                    Ok((_, Attempt::Failed(s, _))) => format!("failed {s}"),
                    Ok((_, Attempt::Offline(e))) => e.clone(),
                    Err(_) => "timed out".into(),
                };
                println!("round {round}: chosen model {label} after {} ms", t.elapsed().as_millis());
                let available = list_models(&key).await.unwrap_or_default();
                let t2 = std::time::Instant::now();
                let mut set = tokio::task::JoinSet::new();
                for m in fallback_order("gemini-flash-latest", &available).into_iter().skip(1) {
                    set.spawn(try_model(key.clone(), m, contents.clone(), true));
                }
                while let Some(Ok((m, o))) = set.join_next().await {
                    if let Attempt::Ok(mut r, minimal) = o {
                        let head = t2.elapsed().as_millis();
                        let _ = r.chunk().await;
                        println!("round {round}: race won by {m} (minimal={minimal}): headers {head} ms, first bytes {} ms; total {} ms", t2.elapsed().as_millis(), t.elapsed().as_millis());
                        break;
                    }
                }
            }
        });
    }

    /// The default model, three times: `cargo test -p crest --lib probe_default -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_default() {
        let key = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let contents = vec![json!({ "role": "user", "parts": [{ "text": "Explain what a mutex is in two sentences." }] })];
            for i in 0..3 {
                let t = std::time::Instant::now();
                let (m, o) = try_model(key.clone(), DEFAULT_MODEL.into(), contents.clone(), true).await;
                match o {
                    Attempt::Ok(mut r, minimal) => {
                        let head = t.elapsed().as_millis();
                        let mut p = SseParser::default();
                        let mut first = None;
                        while let Ok(Some(b)) = r.chunk().await {
                            for ev in p.push(&b) {
                                if first.is_none() && chunk_text(&ev).map(|x| !x.is_empty()).unwrap_or(false) {
                                    first = Some(t.elapsed().as_millis());
                                }
                            }
                        }
                        println!("{i}: {m} minimal={minimal} headers {head} ms, first words {} ms, done {} ms", first.unwrap_or(0), t.elapsed().as_millis());
                    }
                    Attempt::Failed(st, _) => println!("{i}: {m} failed {st} after {} ms", t.elapsed().as_millis()),
                    Attempt::Offline(e) => println!("{i}: {e}"),
                }
            }
        });
    }

    /// Timing: `cargo test -p crest --lib probe_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_speed() {
        let key = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let c = client().unwrap();
            let t = std::time::Instant::now();
            let _ = list_models(&key).await;
            println!("list models: {} ms", t.elapsed().as_millis());
            let configs: [(&str, Value); 1] = [("level=minimal", json!({ "thinkingLevel": "minimal" }))];
            for model in ["gemini-flash-latest", "gemini-3.8-flash", "gemini-3.7-flash", "gemini-3.6-flash", "gemini-3.5-flash", "gemini-flash-lite-latest", "gemini-3.5-flash-lite", "gemini-3.1-flash-lite"] {
                for (name, cfg) in &configs {
                    let mut body = json!({
                        "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
                        "contents": [{ "role": "user", "parts": [{ "text": "Explain what a mutex is in two sentences." }] }],
                    });
                    if !cfg.is_null() {
                        body["generationConfig"] = json!({ "thinkingConfig": cfg });
                    }
                    let t = std::time::Instant::now();
                    let mut r = c
                        .post(format!("{API}/models/{model}:streamGenerateContent?alt=sse"))
                        .header("x-goog-api-key", &key)
                        .json(&body)
                        .send()
                        .await
                        .unwrap();
                    let status = r.status().as_u16();
                    let head = t.elapsed().as_millis();
                    if status != 200 {
                        let txt: String = r.text().await.unwrap_or_default().chars().take(90).collect();
                        println!("{model:28} {name:14} {status} after {head} ms {}", txt.replace('\n', " "));
                        continue;
                    }
                    let mut p = SseParser::default();
                    let mut first = None;
                    while let Ok(Some(b)) = r.chunk().await {
                        for ev in p.push(&b) {
                            if first.is_none() && chunk_text(&ev).map(|s| !s.is_empty()).unwrap_or(false) {
                                first = Some(t.elapsed().as_millis());
                            }
                        }
                    }
                    println!("{model:28} {name:14} 200 headers {head} ms, first text {} ms, done {} ms", first.unwrap_or(0), t.elapsed().as_millis());
                }
            }
        });
    }

    #[test]
    fn seeds_hotkey_once_unless_taken() {
        let mut s = crate::settings::Settings::default();
        seed_settings(&mut s);
        assert_eq!(s.hotkeys.modules.get("ask").map(String::as_str), Some(DEFAULT_HOTKEY));
        s.hotkeys.modules.remove("ask");
        seed_settings(&mut s);
        assert!(s.hotkeys.modules.get("ask").is_none(), "removed by the user: not added back");

        let mut t = crate::settings::Settings::default();
        t.hotkeys.open_settings = "ctrl+alt+keya".into();
        seed_settings(&mut t);
        assert!(t.hotkeys.modules.get("ask").is_none());
    }
}
