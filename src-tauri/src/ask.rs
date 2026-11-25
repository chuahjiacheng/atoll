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
const API: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google's alias for its newest Flash model, so the default never goes stale.
pub const DEFAULT_MODEL: &str = "gemini-flash-latest";
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
        self.messages.push(Msg { id, role, text, at: now, error: None, model: String::new() });
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

// ---------- state ----------

#[derive(Default)]
pub struct AskState {
    chat: Mutex<Option<Chat>>,
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
    pub has_key: bool,
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
    AskView { messages, busy: (busy != 0).then_some(busy), has_key: load_key().is_some(), model: model(app) }
}

fn emit(app: &AppHandle) {
    let _ = app.emit("ask", view(app));
}

// ---------- Credential Manager ----------

fn entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, GEMINI_KEY).ok()
}

fn load_key() -> Option<String> {
    entry()?.get_password().ok().filter(|s| !s.trim().is_empty())
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
    let key = load_key().ok_or("Add a Gemini API key in Settings → Ask.")?;
    let model = model(app);
    let now = now_ms();

    let (reply_id, contents) = with_chat(app, |c| {
        if c.is_stale(now) {
            c.clear();
        }
        c.push(Role::User, text, now);
        let contents = gemini_contents(&c.messages);
        let id = c.push(Role::Assistant, String::new(), now);
        if let Some(m) = c.get_mut(id) {
            m.model = model.clone();
        }
        (id, contents)
    });
    state.busy.store(reply_id, Ordering::SeqCst);
    let run = state.run.fetch_add(1, Ordering::SeqCst) + 1;
    save(app);
    emit(app);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = stream(&app, &key, &model, contents, reply_id, run).await;
        let state = app.state::<AskState>();
        if state.run.load(Ordering::SeqCst) != run {
            return; // stopped: `stop` already tidied up
        }
        with_chat(&app, |c| {
            if let Some(m) = c.get_mut(reply_id) {
                match result {
                    Ok(()) if m.text.trim().is_empty() => m.error = Some("Gemini sent an empty answer. Try asking again.".into()),
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
async fn stream(app: &AppHandle, key: &str, model: &str, contents: Vec<Value>, id: u64, run: u64) -> Result<(), String> {
    let body = json!({
        "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
        "contents": contents,
    });
    let state = app.state::<AskState>();
    let url = format!("{API}/models/{model}:streamGenerateContent?alt=sse");
    let send = client()?.post(url).header("x-goog-api-key", key).json(&body).send();
    let mut resp = tokio::select! {
        r = tokio::time::timeout(STREAM_IDLE, send) => match r {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(offline(&e)),
            Err(_) => return Err("Gemini took too long to answer. Try again.".into()),
        },
        _ = state.cancel.notified() => return Ok(()),
    };
    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(friendly_error(status, &body, model));
    }

    let mut parser = SseParser::default();
    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(STREAM_IDLE, resp.chunk()) => match c {
                Ok(Ok(c)) => c,
                Ok(Err(_)) => return Err("The connection to Gemini dropped mid-answer.".into()),
                Err(_) => return Err("Gemini stopped sending. Try again.".into()),
            },
            _ = state.cancel.notified() => return Ok(()),
        };
        let ended = chunk.is_none();
        let events = match chunk {
            Some(bytes) => parser.push(&bytes),
            None => parser.finish().into_iter().collect(),
        };
        for ev in events {
            let piece = chunk_text(&ev)?;
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
    forget_key(app);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: u64, role: Role, text: &str) -> Msg {
        Msg { id, role, text: text.into(), at: 0, error: None, model: String::new() }
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
