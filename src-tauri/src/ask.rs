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
/// Snip part of the screen and ask about it.
pub const SNIP_HOTKEY: &str = "Ctrl+Alt+KeyQ";

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
    /// What was attached to a question (clipboard, sessions, screen).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attached: Vec<Attached>,
}

/// Context sent along with one question.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Attached {
    /// "clipboard" | "sessions" | "screen"
    pub kind: String,
    /// Shown on the question, e.g. "Clipboard · const a = 1…".
    pub label: String,
    /// Text given to the model.
    pub text: String,
    /// A picture for the model (path to a PNG or JPEG on this PC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Web results behind the answer, numbered as the model cites them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub title: String,
    pub url: String,
}

/// Longest context text sent with one question, in characters.
pub const MAX_CONTEXT: usize = 20_000;

/// The question as the model sees it: attached context first, then the question.
pub fn prompt_text(m: &Msg) -> String {
    if m.attached.is_empty() {
        return m.text.clone();
    }
    let mut out = String::new();
    for a in &m.attached {
        out.push_str(&format!("<context source=\"{}\">\n{}\n</context>\n\n", a.kind, a.text.trim()));
    }
    out.push_str(&m.text);
    out
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
        self.messages.push(Msg { id, role, text, at: now, error: None, model: String::new(), provider: String::new(), attached: Vec::new() });
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
                text.push_str(&prompt_text(m));
            }
            _ => out.push((m.role, prompt_text(m))),
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
    /// A web search is running for the answer in progress.
    searching: std::sync::atomic::AtomicBool,
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
    /// Context you allowed in Settings → Ask ("web", "clipboard", "sessions", "screen").
    pub context: Vec<String>,
    /// The answer in progress is waiting on a web search.
    pub searching: bool,
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
    let context = allowed_context(app);
    let searching = app.state::<AskState>().searching.load(Ordering::SeqCst);
    AskView { messages, busy: (busy != 0).then_some(busy), has_key, provider, model, context, searching }
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

pub fn send(app: &AppHandle, text: String, attach: Vec<String>, pictures: Vec<String>) -> Result<(), String> {
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
    let mut plan = provider_plan(&main, backup, load_key().is_some(), load_named(GROQ_KEY).is_some())?;
    let mut attached = gather(app, &attach)?;
    if pictures.len() > 6 {
        return Err("Send at most 6 pictures with one question.".into());
    }
    for p in &pictures {
        attached.push(picture_context(p)?);
    }
    let web = attach.iter().any(|k| k == "web");
    if web && !allowed_context(app).iter().any(|k| k == "web") {
        return Err("Add a web search key in Settings → Ask first.".into());
    }
    // Pictures go to Gemini, or to a local model that can see (checked when it
    // starts); Groq's fast models can't.
    if attached.iter().any(|a| a.image.is_some()) {
        plan.retain(|p| *p != Provider::Groq);
        if plan.is_empty() {
            return Err("Pictures need Gemini or a local model that can see. Add a Gemini key in Settings → Ask, or ask without the picture.".into());
        }
    }
    let now = now_ms();

    let question = text.clone();
    let (uid, reply_id) = with_chat(app, |c| {
        if c.is_stale(now) {
            c.clear();
        }
        let uid = c.push(Role::User, text, now);
        if let Some(m) = c.get_mut(uid) {
            m.attached = attached;
        }
        (uid, c.push(Role::Assistant, String::new(), now))
    });
    state.busy.store(reply_id, Ordering::SeqCst);
    state.searching.store(web, Ordering::SeqCst);
    let run = state.run.fetch_add(1, Ordering::SeqCst) + 1;
    save(app);
    emit(app);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            if web {
                let found = web_search(&app, &question).await;
                app.state::<AskState>().searching.store(false, Ordering::SeqCst);
                if app.state::<AskState>().run.load(Ordering::SeqCst) != run {
                    return Ok(());
                }
                let a = found?;
                with_chat(&app, |c| {
                    if let Some(m) = c.get_mut(uid) {
                        m.attached.insert(0, a);
                    }
                });
                save(&app);
                emit(&app);
            }
            let contents = with_chat(&app, |c| build_contents(c, uid));
            answer(&app, &plan, contents, reply_id, run).await
        }
        .await;
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
        state.searching.store(false, Ordering::SeqCst);
        save(&app);
        emit(&app);
    });
    Ok(())
}

/// What the model is sent: the chat, plus the pictures attached to question
/// `uid` (older pictures are not re-sent).
fn build_contents(c: &Chat, uid: u64) -> Vec<Value> {
    let mut contents = gemini_contents(&c.messages);
    if let (Some(last), Some(m)) = (contents.last_mut(), c.messages.iter().find(|m| m.id == uid)) {
        for a in &m.attached {
            if let Some(part) = a.image.as_deref().and_then(image_part) {
                if let Some(parts) = last["parts"].as_array_mut() {
                    parts.push(part);
                }
            }
        }
    }
    contents
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
/// Pictures in a Gemini-format message become `image_url` parts (data URLs),
/// which Ollama's vision models read.
pub fn openai_messages(contents: &[Value]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    for c in contents {
        let role = if c["role"] == "model" { "assistant" } else { "user" };
        let parts = c["parts"].as_array().cloned().unwrap_or_default();
        let text: String = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        let images: Vec<Value> = parts
            .iter()
            .filter_map(|p| {
                let d = p.get("inline_data")?;
                Some(json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", d["mime_type"].as_str()?, d["data"].as_str()?) } }))
            })
            .collect();
        if images.is_empty() {
            out.push(json!({ "role": role, "content": text }));
        } else {
            let mut content = vec![json!({ "type": "text", "text": text })];
            content.extend(images);
            out.push(json!({ "role": role, "content": content }));
        }
    }
    out
}

/// A model turned down the thinking setting: one with no thinking mode
/// ("does not support thinking") or that doesn't know the field.
pub fn rejects_reasoning_setting(status: u16, body: &str) -> bool {
    let low = body.to_lowercase();
    status == 400 && (low.contains("reasoning") || low.contains("think"))
}

/// The chat has a picture in it.
pub fn has_picture(contents: &[Value]) -> bool {
    contents.iter().any(|c| c["parts"].as_array().is_some_and(|ps| ps.iter().any(|p| p.get("inline_data").is_some())))
}

/// What an Ollama model can do, from `/api/show` ("vision", "tools", "thinking"…).
pub fn ollama_capabilities_from(show: &Value) -> Vec<String> {
    show.get("capabilities").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// How long Ollama keeps the local model loaded after Ask uses it.
const OLLAMA_KEEP_ALIVE: &str = "30m";

/// Load the local model ahead of the question (opening the Ask tab), so the
/// first answer doesn't wait for it: a cold start took 31 s, a warm one 0.3 s.
pub async fn warm_local(app: &AppHandle) {
    let (provider, picked) = {
        let s = app.state::<crate::AppState>();
        let s = s.settings.lock().unwrap();
        (s.ask.provider.clone(), s.ask.ollama_model.clone())
    };
    if provider != "ollama" {
        return;
    }
    let model = if picked.is_empty() {
        match ollama_models().await.ok().and_then(|m| m.into_iter().next()) {
            Some(m) => m,
            None => return,
        }
    } else {
        picked
    };
    if let Ok(c) = client() {
        // An empty prompt only loads the model.
        let _ = c
            .post(format!("{OLLAMA_API}/api/generate"))
            .json(&json!({ "model": model, "keep_alive": OLLAMA_KEEP_ALIVE }))
            .timeout(Duration::from_secs(120))
            .send()
            .await;
    }
}

async fn ollama_can_see(model: &str) -> Result<bool, String> {
    let resp = client()?
        .post(format!("{OLLAMA_API}/api/show"))
        .json(&json!({ "model": model }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "Ollama isn't running. Start Ollama, or pick another provider in Settings → Ask.".to_string())?;
    let v: Value = resp.json().await.map_err(|_| "Ollama sent a reply Atoll couldn't read.".to_string())?;
    Ok(ollama_capabilities_from(&v).iter().any(|c| c == "vision"))
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
            if has_picture(contents) && !ollama_can_see(&model).await? {
                return Err(format!(
                    "{model} can't look at pictures. Pick a local model that can (for example qwen3.5:4b or gemma4:e4b) in Settings → Ask, or ask without the picture."
                ));
            }
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
        if reasoning.is_some() && rejects_reasoning_setting(status, &body) {
            reasoning = None;
            continue;
        }
        if provider == Provider::Groq && status == 404 {
            state.groq_models.lock().unwrap().clear();
        }
        return Err(openai_error(provider, status, &body, &model));
    }
}

// ---------- context (stage 3) ----------

fn allowed_context(app: &AppHandle) -> Vec<String> {
    let s = app.state::<crate::AppState>();
    let s = s.settings.lock().unwrap();
    let mut out = Vec::new();
    if load_named(search_key_name(&s.ask.search_provider)).is_some() {
        out.push("web".to_string());
    }
    if s.ask.context_clipboard {
        out.push("clipboard".to_string());
    }
    if s.ask.context_sessions && s.modules.claude {
        out.push("sessions".to_string());
    }
    if s.ask.context_screen {
        out.push("screen".to_string());
    }
    out
}

/// Build what was asked for, refusing kinds switched off in Settings.
fn gather(app: &AppHandle, kinds: &[String]) -> Result<Vec<Attached>, String> {
    let allowed = allowed_context(app);
    let mut out = Vec::new();
    for k in kinds.iter().filter(|k| k.as_str() != "web") {
        if !allowed.contains(k) {
            return Err("That context is switched off in Settings → Ask.".into());
        }
        if out.iter().any(|a: &Attached| &a.kind == k) {
            continue;
        }
        out.push(match k.as_str() {
            "clipboard" => clipboard_context(&crate::clipboard::list(app))?,
            "sessions" => {
                let list = app.state::<crate::AppState>().sessions.lock().unwrap().snapshot();
                sessions_context(&list)?
            }
            "screen" => screen_context()?,
            _ => return Err("Unknown context.".into()),
        });
    }
    Ok(out)
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_CONTEXT {
        text.to_string()
    } else {
        let mut t: String = text.chars().take(MAX_CONTEXT).collect();
        t.push_str("\n[cut: too long]");
        t
    }
}

fn snippet(text: &str, n: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n {
        one
    } else {
        format!("{}…", one.chars().take(n).collect::<String>())
    }
}

/// Your latest copy. A copy Atoll flagged as a secret is never sent.
pub fn clipboard_context(items: &[crate::clipboard::Clip]) -> Result<Attached, String> {
    let c = items.iter().max_by_key(|c| c.at).ok_or("Your clipboard history is empty. Copy something first.")?;
    if let Some(kind) = &c.secret {
        return Err(format!("Your latest copy looks like a secret ({kind}), so Ask won't send it."));
    }
    let base = Attached { kind: "clipboard".into(), ..Default::default() };
    Ok(match c.kind.as_str() {
        "image" => {
            let path = c.image.clone().ok_or("That copied picture is no longer saved.")?;
            let mut text = format!("The user copied a picture ({}×{}); it is attached.", c.width, c.height);
            if let Some(o) = c.ocr.as_deref().filter(|o| !o.trim().is_empty()) {
                text.push_str(&format!(" Text found in it:\n{}", cap(o)));
            }
            Attached { label: format!("Clipboard · picture {}×{}", c.width, c.height), text, image: Some(path), ..base }
        }
        "files" => {
            let names: Vec<String> = c
                .files
                .iter()
                .map(|f| std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone()))
                .collect();
            Attached {
                label: format!("Clipboard · {} {}", names.len(), if names.len() == 1 { "file" } else { "files" }),
                text: format!("The user copied these files:\n{}", c.files.join("\n")),
                ..base
            }
        }
        _ => {
            let text = c.text.clone().unwrap_or_default();
            if text.trim().is_empty() {
                return Err("Your latest copy has no text.".into());
            }
            Attached { label: format!("Clipboard · {}", snippet(&text, 40)), text: cap(&text), ..base }
        }
    })
}

/// What your Claude Code sessions are doing.
pub fn sessions_context(list: &[crate::sessions::Session]) -> Result<Attached, String> {
    if list.is_empty() {
        return Err("No Claude Code sessions right now.".into());
    }
    let mut text = String::from("The user's Claude Code sessions, newest first:\n");
    for s in list.iter().take(6) {
        text.push_str(&format!("\n- {} ({}): {:?}", if s.project.is_empty() { "unnamed" } else { &s.project }, s.source, s.status));
        if let Some(p) = &s.prompt {
            text.push_str(&format!("\n  Asked: {}", snippet(p, 300)));
        }
        if !s.plan.is_empty() {
            let done = s.plan.iter().filter(|p| p.status == "completed").count();
            text.push_str(&format!("\n  Task list: {done}/{} done", s.plan.len()));
            if let Some(cur) = s.plan.iter().find(|p| p.status == "in_progress") {
                text.push_str(&format!("; now: {}", snippet(&cur.content, 200)));
            }
        }
        if let Some(a) = &s.activity {
            text.push_str(&format!("\n  Doing: {} {}", a.label, snippet(&a.detail, 200)));
        }
        if let Some(m) = s.message.as_ref().or(s.answer.as_ref()) {
            text.push_str(&format!("\n  Last message: {}", snippet(m, 600)));
        }
    }
    let n = list.len();
    Ok(Attached {
        kind: "sessions".into(),
        label: format!("Claude · {n} {}", if n == 1 { "session" } else { "sessions" }),
        text: cap(&text),
        ..Default::default()
    })
}

fn shots_dir() -> PathBuf {
    crate::paths::local_dir().join("ask").join("shots")
}

/// The window you were using before the island (picture, saved as JPEG).
fn screen_context() -> Result<Attached, String> {
    let hwnd = crate::paste::previous();
    if hwnd.is_null() {
        return Err("Atoll doesn't know which window you were using. Click into it, then open Ask with its hotkey.".into());
    }
    let title = window_title(hwnd);
    let jpeg = capture_window(hwnd)?;
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the picture: {e}"))?;
    // Keep only the newest few pictures.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        while files.len() >= 10 {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }
    let path = dir.join(format!("{}.jpg", now_ms()));
    std::fs::write(&path, jpeg).map_err(|e| format!("Couldn't save the picture: {e}"))?;
    let name = if title.is_empty() { "the window".to_string() } else { format!("\u{201c}{}\u{201d}", snippet(&title, 80)) };
    Ok(Attached {
        kind: "screen".into(),
        label: format!("Screen · {}", if title.is_empty() { "window".into() } else { snippet(&title, 40) }),
        text: format!("A picture of {name}, the window the user was looking at, is attached."),
        image: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    })
}

/// Title of the window Screen would capture (shown on the button).
pub fn screen_title() -> String {
    let hwnd = crate::paste::previous();
    if hwnd.is_null() {
        String::new()
    } else {
        window_title(hwnd)
    }
}

fn window_title(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Picture of one window, even when other windows (or the island) cover it,
/// scaled to at most 1600 px wide and saved as JPEG.
fn capture_window(hwnd: windows_sys::Win32::Foundation::HWND) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic};
    const PW_RENDERFULLCONTENT: u32 = 2;
    unsafe {
        if IsIconic(hwnd) != 0 {
            return Err("That window is minimised. Bring it back, then try again.".into());
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return Err("Couldn't find that window.".into());
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
            return Err("That window has no size to capture.".into());
        }
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp as _);
        let ok = PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) != 0;
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h; // top-down rows
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut px = vec![0u8; (w * h * 4) as usize];
        let rows = GetDIBits(dc, bmp, 0, h as u32, px.as_mut_ptr() as _, &mut info, DIB_RGB_COLORS);
        SelectObject(dc, old);
        DeleteObject(bmp as _);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        if !ok || rows == 0 {
            return Err("Windows wouldn't give a picture of that window.".into());
        }
        encode_bgra_jpeg(&px, w as u32, h as u32)
    }
}

/// BGRA pixels → JPEG, scaled down to 1600 px wide at most.
pub fn encode_bgra_jpeg(bgra: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for p in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    encode_jpeg(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, rgb).ok_or("Bad picture size.")?))
}

/// Any picture → JPEG (transparent parts on white), at most 1600 px on its long side.
pub fn encode_jpeg(img: image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut img = img;
    if img.width().max(img.height()) > 1600 {
        img = img.resize(1600, 1600, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p.0[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([mix(p.0[0]), mix(p.0[1]), mix(p.0[2])]));
    }
    let img = image::DynamicImage::ImageRgb8(rgb);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
        .encode_image(&img)
        .map_err(|e| format!("Couldn't save the picture: {e}"))?;
    Ok(out)
}

/// A Gemini part for a picture file (PNG or JPEG).
fn image_part(path: &str) -> Option<Value> {
    use base64::Engine;
    let bytes = std::fs::read(path).ok()?;
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
    Some(json!({ "inline_data": { "mime_type": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) } }))
}

// ---------- web search (Tavily or Ollama's web search; both free with a key) ----------

const TAVILY_KEY: &str = "ask.tavily.apiKey";
const OLLAMA_SEARCH_KEY: &str = "ask.ollamaSearch.apiKey";
/// Results given to the model per question.
const SEARCH_RESULTS: usize = 5;
/// Text kept from each result.
const RESULT_CHARS: usize = 1500;

fn search_key_name(provider: &str) -> &'static str {
    if provider == "ollama" {
        OLLAMA_SEARCH_KEY
    } else {
        TAVILY_KEY
    }
}

fn search_label(provider: &str) -> &'static str {
    if provider == "ollama" {
        "Ollama web search"
    } else {
        "Tavily"
    }
}

fn search_url(provider: &str) -> &'static str {
    if provider == "ollama" {
        "https://ollama.com/api/web_search"
    } else {
        "https://api.tavily.com/search"
    }
}

pub fn search_body(provider: &str, query: &str) -> Value {
    if provider == "ollama" {
        json!({ "query": query, "max_results": SEARCH_RESULTS })
    } else {
        json!({ "query": query, "max_results": SEARCH_RESULTS, "search_depth": "basic", "include_answer": false })
    }
}

/// Results from either service (both answer `{ "results": [{ title, url, content }] }`),
/// as context for the model and numbered sources for the page.
pub fn search_context(query: &str, reply: &Value) -> Result<Attached, String> {
    let results: Vec<&Value> = reply.get("results").and_then(Value::as_array).map(|r| r.iter().collect()).unwrap_or_default();
    let mut sources = Vec::new();
    let mut text = format!(
        "Web search results for \u{201c}{}\u{201d}. Use them when they help, and cite them inline as [n](url) with the number shown. Say so if they don't answer the question.\n",
        snippet(query, 200)
    );
    for r in results.iter().take(SEARCH_RESULTS) {
        let url = r.get("url").and_then(Value::as_str).unwrap_or("").trim();
        if !url.starts_with("http") {
            continue;
        }
        let title = r.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).unwrap_or(url);
        let content: String = r.get("content").and_then(Value::as_str).unwrap_or("").chars().take(RESULT_CHARS).collect();
        sources.push(Source { title: snippet(title, 120), url: url.to_string() });
        text.push_str(&format!("\n[{}] {}\n{}\n{}\n", sources.len(), title, url, content.trim()));
    }
    if sources.is_empty() {
        return Err("The web search found nothing for that. Try other words, or ask without Web.".into());
    }
    let n = sources.len();
    Ok(Attached { kind: "web".into(), label: format!("Web · {n} {}", if n == 1 { "result" } else { "results" }), text, image: None, sources })
}

pub fn search_error(provider: &str, status: u16, body: &str) -> String {
    let who = search_label(provider);
    let low = body.to_lowercase();
    match status {
        401 | 403 => format!("{who} didn't accept the API key. Check it in Settings → Ask."),
        429 | 432 | 433 => format!("You've used {who}'s free searches for now. Try again later."),
        500..=599 => format!("{who} is having trouble (error {status}). Try again in a moment."),
        _ if low.contains("invalid") && low.contains("key") => format!("{who} didn't accept the API key. Check it in Settings → Ask."),
        _ => format!("{who} search failed (error {status})."),
    }
}

async fn search_with(provider: &str, key: &str, query: &str) -> Result<Value, String> {
    let who = search_label(provider);
    let resp = client()?
        .post(search_url(provider))
        .bearer_auth(key)
        .json(&search_body(provider, query))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { format!("{who} took too long to search.") } else { format!("Couldn't reach {who}. Check your internet connection.") })?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(search_error(provider, status, &body));
    }
    serde_json::from_str(&body).map_err(|_| format!("{who} sent a reply Atoll couldn't read."))
}

/// Search the web for `query` with the service picked in Settings → Ask.
async fn web_search(app: &AppHandle, query: &str) -> Result<Attached, String> {
    let provider = app.state::<crate::AppState>().settings.lock().unwrap().ask.search_provider.clone();
    let key = load_named(search_key_name(&provider)).ok_or("Add a web search key in Settings → Ask first.")?;
    let reply = search_with(&provider, &key, query).await?;
    search_context(query, &reply)
}

pub fn search_key_status(provider: &str) -> KeyStatus {
    match load_named(search_key_name(provider)) {
        Some(k) => KeyStatus { has_key: true, masked: mask_key(&k) },
        None => KeyStatus { has_key: false, masked: String::new() },
    }
}

/// Check a search key with one small search, then keep it in Credential Manager.
pub async fn set_search_key(app: &AppHandle, provider: &str, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your API key first.".into());
    }
    if key.len() > 300 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like an API key.".into());
    }
    search_with(provider, key, "weather today").await?;
    named_entry(search_key_name(provider))
        .ok_or("Windows Credential Manager is unavailable.")?
        .set_password(key)
        .map_err(|e| format!("Couldn't save to Windows Credential Manager: {e}"))?;
    emit(app);
    Ok(())
}

pub fn forget_search_key(app: &AppHandle, provider: &str) {
    if let Some(e) = named_entry(search_key_name(provider)) {
        let _ = e.delete_credential();
    }
    emit(app);
}

/// Test the saved key with one small search.
pub async fn test_search_key(provider: &str) -> Result<(), String> {
    let key = load_named(search_key_name(provider)).ok_or("Add a key first.")?;
    search_with(provider, &key, "weather today").await.map(|_| ())
}

// ---------- pictures you paste or snip ----------

/// A picture waiting to go with the next question.
#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Picture {
    /// Saved copy on this PC (sent with the question).
    pub path: String,
    /// Small PNG preview as a data URL.
    pub thumb: String,
    pub width: u32,
    pub height: u32,
}

/// Biggest picture accepted, encoded.
const MAX_PICTURE_BYTES: usize = 25 * 1024 * 1024;

/// Save a picture for Ask (as JPEG in the shots folder) with a preview.
pub fn save_picture(img: image::DynamicImage) -> Result<Picture, String> {
    use base64::Engine;
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 {
        return Err("That picture is empty.".into());
    }
    let mut thumb = Vec::new();
    img.thumbnail(160, 160)
        .write_to(&mut std::io::Cursor::new(&mut thumb), image::ImageFormat::Png)
        .map_err(|e| format!("Couldn't read the picture: {e}"))?;
    let jpeg = encode_jpeg(img)?;
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the picture: {e}"))?;
    prune_shots(&dir);
    let path = dir.join(format!("{}-{}.jpg", now_ms(), random_tag()));
    std::fs::write(&path, jpeg).map_err(|e| format!("Couldn't save the picture: {e}"))?;
    Ok(Picture {
        path: path.to_string_lossy().into_owned(),
        thumb: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(thumb)),
        width,
        height,
    })
}

fn random_tag() -> String {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Keep only the newest pictures (questions keep their own path; old ones go).
fn prune_shots(dir: &std::path::Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        while files.len() >= 30 {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }
}

/// A picture pasted into the question box (base64 of the image file).
pub fn paste_picture(data: &str) -> Result<Picture, String> {
    use base64::Engine;
    let data = data.split_once("base64,").map(|(_, d)| d).unwrap_or(data);
    if data.len() > MAX_PICTURE_BYTES * 4 / 3 + 16 {
        return Err("That picture is too big (25 MB at most).".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| "Couldn't read that picture.".to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|_| "That isn't a picture Ask can read (PNG, JPEG, GIF, WebP or BMP).".to_string())?;
    save_picture(img)
}

/// A picture to send: only ones Ask saved itself (in its shots folder).
pub fn picture_context(path: &str) -> Result<Attached, String> {
    let dir = std::fs::canonicalize(shots_dir()).map_err(|_| "That picture is no longer saved.".to_string())?;
    let file = std::fs::canonicalize(path).map_err(|_| "That picture is no longer saved.".to_string())?;
    if !file.starts_with(&dir) {
        return Err("Ask can only send pictures you pasted or snipped.".into());
    }
    Ok(Attached {
        kind: "picture".into(),
        label: "Picture".into(),
        text: "The user attached a picture; it is attached.".into(),
        image: Some(file.to_string_lossy().into_owned()),
        ..Default::default()
    })
}

/// Snip: open Windows' own region snipping, then put the picture in Ask.
/// Windows copies a finished snip to the clipboard; Ask waits for that
/// (up to two minutes, so a cancelled snip just ends quietly).
pub fn snip(app: &AppHandle) {
    use std::sync::atomic::AtomicBool;
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    // The island steps out of the way of the snipping overlay.
    let _ = app.emit("ask:snipping", ());
    let app = app.clone();
    std::thread::spawn(move || {
        let seq = || unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
        let start = seq();
        std::thread::sleep(Duration::from_millis(250));
        let launched = std::process::Command::new("explorer.exe").arg("ms-screenclip:").spawn();
        if launched.is_err() {
            let _ = app.emit("ask:picture-error", "Couldn't open Windows' snipping. Press Win+Shift+S, then paste the picture into Ask.");
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut last = start;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
            let now = seq();
            if now == last {
                continue;
            }
            last = now;
            let img = match crate::winclip::read_with(false) {
                Some(crate::winclip::Content::Image(crate::winclip::Image::Png(b))) => image::load_from_memory(&b).ok(),
                Some(crate::winclip::Content::Image(crate::winclip::Image::Rgba { width, height, pixels })) => {
                    image::RgbaImage::from_raw(width, height, pixels).map(image::DynamicImage::ImageRgba8)
                }
                _ => None,
            };
            if let Some(img) = img {
                match save_picture(img) {
                    Ok(p) => {
                        crate::windows::open_tab(&app, "ask");
                        let _ = app.emit("ask:picture", p);
                    }
                    Err(e) => {
                        let _ = app.emit("ask:picture-error", e);
                    }
                }
                break;
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
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
    state.searching.store(false, Ordering::SeqCst);
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
    let _ = std::fs::remove_dir_all(shots_dir());
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
    forget_search_key(app, "tavily");
    forget_search_key(app, "ollama");
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
    if s.ask.seed_version < 3 {
        s.ask.seed_version = 3;
        let taken = s
            .hotkeys
            .modules
            .values()
            .chain([&s.hotkeys.toggle_island, &s.hotkeys.expand_island, &s.hotkeys.open_settings])
            .any(|v| v.eq_ignore_ascii_case(SNIP_HOTKEY));
        if !taken {
            s.hotkeys.modules.entry("askSnip".into()).or_insert_with(|| SNIP_HOTKEY.into());
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
        Msg { id, role, text: text.into(), at: 0, error: None, model: String::new(), provider: String::new(), attached: Vec::new() }
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
        assert_eq!(s.ask.seed_version, 3, "every later one-time change ran too");
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

    fn clip(v: Value) -> crate::clipboard::Clip {
        let mut base = json!({ "id": "c", "kind": "text", "at": 1 });
        for (k, val) in v.as_object().unwrap() {
            base[k] = val.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn clipboard_context_takes_newest_and_never_sends_secrets() {
        let items = vec![
            clip(json!({ "id": "old", "text": "older copy", "at": 1 })),
            clip(json!({ "id": "new", "text": "fn main() {\n    println!(\"hi\");\n}", "at": 5 })),
        ];
        let a = clipboard_context(&items).unwrap();
        assert_eq!(a.kind, "clipboard");
        assert!(a.text.contains("println!"));
        assert_eq!(a.label, "Clipboard · fn main() { println!(\"hi\"); }");
        assert!(a.image.is_none());

        let secret = vec![clip(json!({ "text": "ghp_••••", "secret": "GitHub token", "at": 9 }))];
        assert!(clipboard_context(&secret).unwrap_err().contains("secret (GitHub token)"));
        assert!(clipboard_context(&[]).unwrap_err().contains("empty"));

        let pic = vec![clip(json!({ "kind": "image", "image": "C:\\x\\a.png", "width": 800, "height": 600, "ocr": "Total 42", "at": 2 }))];
        let p = clipboard_context(&pic).unwrap();
        assert_eq!(p.image.as_deref(), Some("C:\\x\\a.png"));
        assert!(p.text.contains("Total 42"));
        assert_eq!(p.label, "Clipboard · picture 800×600");

        let files = vec![clip(json!({ "kind": "files", "files": ["C:\\a\\report.pdf", "C:\\a\\b.txt"], "at": 2 }))];
        let f = clipboard_context(&files).unwrap();
        assert_eq!(f.label, "Clipboard · 2 files");
        assert!(f.text.contains("report.pdf"));

        let long = vec![clip(json!({ "text": "x".repeat(MAX_CONTEXT + 50), "at": 3 }))];
        assert!(clipboard_context(&long).unwrap().text.ends_with("[cut: too long]"));
    }

    #[test]
    fn sessions_context_summarises_each_session() {
        use crate::sessions::{Activity, PlanStep, Session, Status};
        let step = |c: &str, st: &str| PlanStep { content: c.into(), status: st.into(), id: String::new() };
        let s = Session {
            id: "1".into(),
            project: "twocodeclone".into(),
            cwd: "C:\\x".into(),
            source: "desktop".into(),
            status: Status::Working,
            activity: Some(Activity { kind: "edit".into(), label: "Editing".into(), detail: "ask.rs".into(), active: true }),
            plan: vec![step("Study code", "completed"), step("Write context", "in_progress"), step("Test", "pending")],
            message: None,
            prompt: Some("start stage 3".into()),
            answer: None,
            tool_count: 4,
            ask: None,
            started_at: 0,
            updated_at: 0,
        };
        let a = sessions_context(&[s]).unwrap();
        assert_eq!(a.label, "Claude · 1 session");
        for want in ["twocodeclone (desktop): Working", "Asked: start stage 3", "Task list: 1/3 done; now: Write context", "Doing: Editing ask.rs"] {
            assert!(a.text.contains(want), "{want} missing in:\n{}", a.text);
        }
        assert!(sessions_context(&[]).unwrap_err().contains("No Claude Code sessions"));
    }

    #[test]
    fn context_goes_before_the_question_in_every_format() {
        let mut q = msg(1, Role::User, "what does this do?");
        q.attached = vec![Attached { kind: "clipboard".into(), label: "x".into(), text: "let a = 1;".into(), ..Default::default() }];
        let t = prompt_text(&q);
        assert_eq!(t, "<context source=\"clipboard\">\nlet a = 1;\n</context>\n\nwhat does this do?");
        let g = gemini_contents(&[q.clone()]);
        assert_eq!(g[0]["parts"][0]["text"], t.as_str());
        let o = openai_messages(&g);
        assert_eq!(o[1]["content"], t.as_str());
        // Without context the question is sent as typed.
        assert_eq!(prompt_text(&msg(2, Role::User, "hi")), "hi");
    }

    #[test]
    fn pictures_scale_down_and_become_jpeg() {
        // 2000×10 blue pixels (BGRA).
        let bgra: Vec<u8> = std::iter::repeat([255u8, 0, 0, 255]).take(2000 * 10).flatten().collect();
        let jpeg = encode_bgra_jpeg(&bgra, 2000, 10).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8]), "JPEG header");
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(img.width(), 1600);
        let px = img.to_rgb8().get_pixel(800, 4).0;
        assert!(px[2] > 200 && px[0] < 60, "blue stays blue (BGRA order handled): {px:?}");
        let dir = std::env::temp_dir().join(format!("ask-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("p.jpg");
        std::fs::write(&f, &jpeg).unwrap();
        let part = image_part(f.to_str().unwrap()).unwrap();
        assert_eq!(part["inline_data"]["mime_type"], "image/jpeg");
        assert!(part["inline_data"]["data"].as_str().unwrap().len() > 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Picture of the foreground window (kept on this PC, checked it isn't
    /// blank), then a real Gemini answer about a made-up picture:
    /// `cargo test -p crest --lib probe_screen -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_screen() {
        let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        let t = std::time::Instant::now();
        let jpeg = capture_window(hwnd).expect("capture failed");
        let img = image::load_from_memory(&jpeg).unwrap().to_luma8();
        let (mut lo, mut hi) = (255u8, 0u8);
        for p in img.pixels() {
            lo = lo.min(p.0[0]);
            hi = hi.max(p.0[0]);
        }
        println!("captured the foreground window: {}x{}, {} KB in {} ms, brightness range {lo}..{hi} (blank would be one value)", img.width(), img.height(), jpeg.len() / 1024, t.elapsed().as_millis());

        // A made-up 64x64 picture: solid orange.
        let bgra: Vec<u8> = std::iter::repeat([0u8, 140, 255, 255]).take(64 * 64).flatten().collect();
        let dir = std::env::temp_dir().join("ask-probe");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("orange.jpg");
        std::fs::write(&f, encode_bgra_jpeg(&bgra, 64, 64).unwrap()).unwrap();
        let key = load_key().expect("no Gemini key saved");
        let mut contents = vec![json!({ "role": "user", "parts": [{ "text": "What single colour fills this picture? One word." }] })];
        contents[0]["parts"].as_array_mut().unwrap().push(image_part(f.to_str().unwrap()).unwrap());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let t = std::time::Instant::now();
            let (m, o) = try_model(key, DEFAULT_MODEL.into(), contents, true).await;
            match o {
                Attempt::Ok(mut r, _) => {
                    let mut p = SseParser::default();
                    let mut text = String::new();
                    while let Ok(Some(b)) = r.chunk().await {
                        for ev in p.push(&b) {
                            text.push_str(&chunk_text(&ev).unwrap_or_default());
                        }
                    }
                    println!("{m} in {} ms: {}", t.elapsed().as_millis(), text.trim());
                }
                Attempt::Failed(s, b) => println!("{m} failed {s}: {}", b.chars().take(200).collect::<String>()),
                Attempt::Offline(e) => println!("{e}"),
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Web search: `cargo test -p crest --lib probe_search -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_search() {
        let key = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for (model, search) in [("gemini-flash-lite-latest", false), ("gemini-flash-lite-latest", true)] {
                let mut body = json!({
                    "contents": [{ "role": "user", "parts": [{ "text": "What is today's top news headline in Malaysia? One sentence." }] }],
                    "generationConfig": { "thinkingConfig": { "thinkingLevel": "minimal" } },
                });
                if search {
                    body["tools"] = json!([{ "google_search": {} }]);
                }
                let t = std::time::Instant::now();
                let r = client().unwrap()
                    .post(format!("{API}/models/{model}:streamGenerateContent?alt=sse"))
                    .header("x-goog-api-key", &key).json(&body).send().await.unwrap();
                let status = r.status().as_u16();
                let head = t.elapsed().as_millis();
                let text = r.text().await.unwrap_or_default();
                let mut p = SseParser::default();
                let evs = p.push(text.as_bytes());
                let mut answer = String::new();
                let mut meta = String::new();
                for e in &evs {
                    answer.push_str(&chunk_text(e).unwrap_or_default());
                    if e.contains("groundingMetadata") {
                        let v: Value = serde_json::from_str(e).unwrap_or_default();
                        meta = v.pointer("/candidates/0/groundingMetadata").map(|m| m.to_string()).unwrap_or_default();
                    }
                }
                let short: String = if status == 200 { answer.trim().chars().take(200).collect() } else { text.chars().take(1500).collect() };
                println!("{model} search={search}: {status} headers {head} ms, done {} ms: {}", t.elapsed().as_millis(), short.replace('\n', " "));
                println!("   meta: {}", meta.chars().take(700).collect::<String>());
            }
        });
    }

    #[test]
    fn web_results_become_numbered_sources() {
        // Tavily and Ollama both answer in this shape.
        let reply = json!({ "query": "q", "results": [
            { "title": "Malaysia budget 2027 announced", "url": "https://www.thestar.com.my/news/budget", "content": "The budget was tabled on Friday.", "score": 0.9 },
            { "title": "", "url": "https://example.org/page", "content": "x".repeat(RESULT_CHARS + 500) },
            { "title": "No link", "url": "javascript:alert(1)", "content": "skipped" },
            { "title": "Fourth", "url": "https://four.example", "content": "four" }
        ]});
        let a = search_context("malaysia budget", &reply).unwrap();
        assert_eq!(a.kind, "web");
        assert_eq!(a.label, "Web · 3 results");
        assert_eq!(a.sources.len(), 3);
        assert_eq!(a.sources[0], Source { title: "Malaysia budget 2027 announced".into(), url: "https://www.thestar.com.my/news/budget".into() });
        assert_eq!(a.sources[1].title, "https://example.org/page", "no title: the link stands in");
        assert_eq!(a.sources[2].url, "https://four.example", "a non-web link is skipped and numbering stays in order");
        assert!(a.text.contains("[1] Malaysia budget 2027 announced\nhttps://www.thestar.com.my/news/budget\nThe budget was tabled on Friday."));
        assert!(a.text.contains("[3] Fourth"));
        assert!(!a.text.contains("javascript:"));
        assert!(!a.text.contains(&"x".repeat(RESULT_CHARS + 1)), "each result is cut to RESULT_CHARS");
        assert!(a.text.contains("cite them inline as [n](url)"));
        assert!(search_context("q", &json!({ "results": [] })).unwrap_err().contains("found nothing"));
        assert!(search_context("q", &json!({ "detail": "x" })).is_err());
    }

    #[test]
    fn search_requests_and_errors() {
        assert_eq!(search_body("tavily", "q")["search_depth"], "basic");
        assert_eq!(search_body("tavily", "q")["max_results"], SEARCH_RESULTS);
        assert!(search_body("ollama", "q").get("search_depth").is_none());
        assert_eq!(search_url("ollama"), "https://ollama.com/api/web_search");
        assert_eq!(search_key_name("ollama"), OLLAMA_SEARCH_KEY);
        assert_eq!(search_key_name("anything else"), TAVILY_KEY);
        // Real replies to a bad key on 2026-11-27.
        assert!(search_error("tavily", 401, r#"{"detail":{"error":"Unauthorized: missing or invalid API key."}}"#).contains("Tavily didn't accept the API key"));
        assert!(search_error("ollama", 401, r#"{"error":"Unauthorized"}"#).contains("Ollama web search didn't accept the API key"));
        assert!(search_error("tavily", 432, "").contains("free searches"));
        assert!(search_error("ollama", 503, "").contains("having trouble"));
    }

    #[test]
    fn web_results_reach_the_model_before_the_question() {
        let mut chat = Chat::default();
        let uid = chat.push(Role::User, "who won?".into(), 1);
        let reply = json!({ "results": [{ "title": "Final score", "url": "https://news.example/final", "content": "Team A won 2-1." }] });
        chat.get_mut(uid).unwrap().attached.insert(0, search_context("who won?", &reply).unwrap());
        chat.push(Role::Assistant, String::new(), 1); // the reply being written
        let contents = build_contents(&chat, uid);
        assert_eq!(contents.len(), 1, "the empty reply isn't sent");
        let text = contents[0]["parts"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("<context source=\"web\">"));
        assert!(text.contains("Team A won 2-1."));
        assert!(text.ends_with("who won?"));
    }

    /// Citations: `cargo test -p crest --lib probe_cite -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_cite() {
        let key = load_key().expect("no Gemini key saved");
        let mut chat = Chat::default();
        let uid = chat.push(Role::User, "When does the Atoll test library close on Fridays?".into(), 1);
        let reply = json!({ "results": [
            { "title": "Atoll Test Library opening hours", "url": "https://library.example/hours", "content": "Monday to Thursday 9am-8pm. Friday 9am-5pm. Closed on weekends." },
            { "title": "Library news", "url": "https://library.example/news", "content": "New reading room opens in March." }
        ]});
        chat.get_mut(uid).unwrap().attached.insert(0, search_context("library friday hours", &reply).unwrap());
        let mut contents = build_contents(&chat, uid);
        let body_contents = std::mem::take(&mut contents);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let t = std::time::Instant::now();
            let (m, o) = try_model(key, DEFAULT_MODEL.into(), body_contents, true).await;
            if let Attempt::Ok(mut r, _) = o {
                let mut p = SseParser::default();
                let mut text = String::new();
                while let Ok(Some(b)) = r.chunk().await {
                    for ev in p.push(&b) {
                        text.push_str(&chunk_text(&ev).unwrap_or_default());
                    }
                }
                println!("{m} in {} ms: {}", t.elapsed().as_millis(), text.trim());
            } else {
                println!("{m}: did not answer");
            }
        });
    }

    /// Real web search + answer: `cargo test -p crest --lib probe_web -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_web() {
        let skey = load_named(TAVILY_KEY).expect("no Tavily key saved");
        let gkey = load_key().expect("no Gemini key saved");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let q = "What is the latest stable version of the Rust programming language?";
            let t = std::time::Instant::now();
            let reply = search_with("tavily", &skey, q).await;
            let reply = match reply {
                Ok(r) => r,
                Err(e) => return println!("search failed: {e}"),
            };
            let a = search_context(q, &reply).unwrap();
            println!("search {} ms, {}: {:?}", t.elapsed().as_millis(), a.label, a.sources.iter().map(|s| s.url.as_str()).collect::<Vec<_>>());
            let mut chat = Chat::default();
            let uid = chat.push(Role::User, q.into(), 1);
            chat.get_mut(uid).unwrap().attached.insert(0, a);
            let t = std::time::Instant::now();
            let (m, o) = try_model(gkey, DEFAULT_MODEL.into(), build_contents(&chat, uid), true).await;
            if let Attempt::Ok(mut r, _) = o {
                let mut p = SseParser::default();
                let mut text = String::new();
                while let Ok(Some(b)) = r.chunk().await {
                    for ev in p.push(&b) {
                        text.push_str(&chunk_text(&ev).unwrap_or_default());
                    }
                }
                println!("{m} {} ms: {}", t.elapsed().as_millis(), text.trim());
            } else {
                println!("{m}: did not answer");
            }
        });
    }

    #[test]
    fn pictures_go_to_local_models_as_data_urls() {
        let contents = vec![
            json!({ "role": "user", "parts": [{ "text": "earlier" }] }),
            json!({ "role": "model", "parts": [{ "text": "ok" }] }),
            json!({ "role": "user", "parts": [{ "text": "what is this?" }, { "inline_data": { "mime_type": "image/jpeg", "data": "QUJD" } }] }),
        ];
        assert!(has_picture(&contents));
        assert!(!has_picture(&contents[..2]));
        let m = openai_messages(&contents);
        assert_eq!(m[1]["content"], "earlier", "text-only turns stay plain strings");
        let last = &m[3]["content"];
        assert_eq!(last[0], json!({ "type": "text", "text": "what is this?" }));
        assert_eq!(last[1]["type"], "image_url");
        assert_eq!(last[1]["image_url"]["url"], "data:image/jpeg;base64,QUJD");
        // The real /api/show reply for qwen3.5:9b on this PC.
        let show = json!({ "capabilities": ["completion", "vision", "tools", "thinking"] });
        assert!(ollama_capabilities_from(&show).contains(&"vision".to_string()));
        assert!(ollama_capabilities_from(&json!({})).is_empty());
    }

    #[test]
    fn models_without_thinking_are_asked_again_plainly() {
        assert!(rejects_reasoning_setting(400, r#"{"error":"\"gemma3:4b\" does not support thinking"}"#));
        assert!(rejects_reasoning_setting(400, r#"{"error":{"message":"reasoning_effort is not supported"}}"#));
        assert!(!rejects_reasoning_setting(400, r#"{"error":"model not found"}"#));
        assert!(!rejects_reasoning_setting(500, "thinking"));
    }

    /// A picture to the local model: `cargo test -p crest --lib probe_local_vision -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_local_vision() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let model = ollama_models().await.expect("Ollama not running").into_iter().next().expect("no models");
            println!("{model} can see: {}", ollama_can_see(&model).await.unwrap());
            // A made-up 64x64 picture: solid green.
            let bgra: Vec<u8> = std::iter::repeat([40u8, 200, 40, 255]).take(64 * 64).flatten().collect();
            let dir = std::env::temp_dir().join("ask-probe-local");
            std::fs::create_dir_all(&dir).unwrap();
            let f = dir.join("green.jpg");
            std::fs::write(&f, encode_bgra_jpeg(&bgra, 64, 64).unwrap()).unwrap();
            let mut contents = vec![json!({ "role": "user", "parts": [{ "text": "What single colour fills this picture? One word." }] })];
            contents[0]["parts"].as_array_mut().unwrap().push(image_part(f.to_str().unwrap()).unwrap());
            let t = std::time::Instant::now();
            let mut r = client()
                .unwrap()
                .post(format!("{OLLAMA_API}/v1/chat/completions"))
                .json(&openai_body(&model, &openai_messages(&contents), reasoning_setting(Provider::Ollama, &model)))
                .send()
                .await
                .unwrap();
            println!("status {}", r.status());
            let mut p = SseParser::default();
            let (mut text, mut first) = (String::new(), None);
            'outer: while let Ok(Some(b)) = r.chunk().await {
                for ev in p.push(&b) {
                    match openai_chunk_text(&ev) {
                        Ok(Some(piece)) => {
                            if first.is_none() && !piece.is_empty() {
                                first = Some(t.elapsed().as_millis());
                            }
                            text.push_str(&piece);
                        }
                        Ok(None) => break 'outer,
                        Err(e) => {
                            println!("error {e}");
                            break 'outer;
                        }
                    }
                }
            }
            println!("first words {} ms, done {} ms: {}", first.unwrap_or(0), t.elapsed().as_millis(), text.trim());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn pasted_pictures_are_saved_checked_and_sent() {
        use base64::Engine;
        // A 40x30 red PNG whose top-left 16x16 corner is transparent.
        let mut img = image::RgbaImage::from_pixel(40, 30, image::Rgba([255, 0, 0, 255]));
        for x in 0..16 {
            for y in 0..16 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let p = paste_picture(&format!("data:image/png;base64,{b64}")).unwrap();
        assert_eq!((p.width, p.height), (40, 30));
        assert!(p.thumb.starts_with("data:image/png;base64,"));
        let saved = image::load_from_memory(&std::fs::read(&p.path).unwrap()).unwrap().to_rgb8();
        let c = saved.get_pixel(4, 4).0;
        assert!(c.iter().all(|v| *v > 240), "transparent becomes white, not black: {c:?}");
        let r = saved.get_pixel(30, 20).0;
        assert!(r[0] > 230 && r[1] < 30, "red stays red: {r:?}");
        let a = picture_context(&p.path).unwrap();
        assert_eq!(a.kind, "picture");
        assert!(a.image.is_some());
        let _ = std::fs::remove_file(&p.path);

        assert!(paste_picture("not base64 at all!!").is_err());
        assert!(paste_picture(&base64::engine::general_purpose::STANDARD.encode(b"hello")).unwrap_err().contains("isn't a picture"));
        // Only Ask's own pictures can be sent: nothing else on the PC.
        let outside = std::env::temp_dir().join("ask-outside.jpg");
        std::fs::write(&outside, &png).unwrap();
        assert!(picture_context(outside.to_str().unwrap()).unwrap_err().contains("only send pictures you pasted or snipped"));
        let _ = std::fs::remove_file(&outside);
        assert!(picture_context("C:\\does\\not\\exist.jpg").is_err());
    }

    #[test]
    fn big_pictures_shrink_to_1600_on_the_long_side() {
        let tall = image::DynamicImage::ImageRgb8(image::RgbImage::new(900, 3200));
        let j = image::load_from_memory(&encode_jpeg(tall).unwrap()).unwrap();
        assert_eq!((j.width(), j.height()), (450, 1600));
    }

    #[test]
    fn snip_hotkey_is_added_once() {
        let mut s: crate::settings::Settings = serde_json::from_value(json!({ "ask": { "seeded": true, "seedVersion": 2 } })).unwrap();
        seed_settings(&mut s);
        assert_eq!(s.hotkeys.modules.get("askSnip").map(String::as_str), Some(SNIP_HOTKEY));
        s.hotkeys.modules.remove("askSnip");
        seed_settings(&mut s);
        assert!(s.hotkeys.modules.get("askSnip").is_none(), "removed by you: not added back");
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
