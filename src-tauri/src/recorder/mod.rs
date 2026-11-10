//! Flight recorder: every Claude Code hook event and every usage reading,
//! redacted, in one local SQLite database (WAL mode).
//! Contract: docs/contracts/event-store.md.
//!
//! - One writer thread owns the only write connection. `record_*` only queues
//!   (bounded, `try_send`), so a hook response never waits on the disk; if the
//!   queue is ever full the event is dropped and counted, never blocked on.
//! - The writer sleeps on the channel when there is nothing to do (no polling,
//!   no idle CPU). It wakes once a second only while a change notice is owed.
//! - Reads use their own read-only connection; WAL lets them run during writes.

pub mod breakdown;
pub mod commands;
pub mod redact;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use breakdown::Breakdown;

pub type EventId = i64;

pub const SCHEMA_VERSION: i64 = 1;
/// Pending writes before new ones are dropped (hook bursts are far smaller).
const QUEUE: usize = 4096;
/// Most jobs written in one transaction.
const BATCH: usize = 256;
/// A usage reading is kept if it changed, or this long after the last one.
const USAGE_EVERY_MS: i64 = 10 * 60 * 1000;
/// `recorder:changed` at most this often per session.
const NOTIFY_EVERY: Duration = Duration::from_secs(1);
const SUMMARY_CHARS: usize = 200;
const DETAIL_CHARS: usize = 4000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `%LOCALAPPDATA%\Crest\recorder\events.db`
pub fn default_path() -> PathBuf {
    crate::paths::local_dir().join("recorder").join("events.db")
}

// ---------------------------------------------------------------- types

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    FiveHour,
    SevenDay,
}

impl Window {
    pub fn as_str(self) -> &'static str {
        match self {
            Window::FiveHour => "five_hour",
            Window::SevenDay => "seven_day",
        }
    }
    fn parse(s: &str) -> Option<Window> {
        match s {
            "five_hour" => Some(Window::FiveHour),
            "seven_day" => Some(Window::SevenDay),
            _ => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSample {
    pub ts: i64,
    pub window: Window,
    pub used: f64,
    /// Unix SECONDS, 0 = no window running.
    pub resets_at: u64,
    /// "statusline" | "signin" | "log"
    pub source: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionQuery {
    pub since_ms: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventQuery {
    pub session_id: Option<String>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub kinds: Option<Vec<String>>,
    pub limit: Option<u32>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub session_id: String,
    pub project: String,
    pub entrypoint: String,
    pub started_at: i64,
    pub last_event_at: i64,
    pub ended_at: Option<i64>,
    pub event_count: i64,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EventRow {
    pub id: EventId,
    pub ts: i64,
    pub session_id: String,
    pub kind: String,
    pub tool: Option<String>,
    pub agent_id: Option<String>,
    pub summary: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EventDetail {
    #[serde(flatten)]
    pub row: EventRow,
    pub tool_use_id: Option<String>,
    pub agent_type: Option<String>,
    pub detail: String,
    pub payload: Value,
    pub diff: Option<Vec<DiffHunk>>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub path: String,
    pub old: String,
    pub new: String,
    /// The bridge cut a string at 4,000 characters (it ends in "…").
    pub truncated: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AgentNode {
    pub agent_id: String,
    pub agent_type: Option<String>,
    /// Hook inputs name no parent agent, so this is always None for now.
    pub parent: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub event_count: i64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoreStats {
    pub events: i64,
    pub sessions: i64,
    pub usage_samples: i64,
    /// Database file plus its write-ahead log.
    pub file_bytes: i64,
    pub oldest_ts: Option<i64>,
    /// Events dropped because the write queue was full (should stay 0).
    pub dropped: u64,
}

// ---------------------------------------------------------------- schema

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  session_id    TEXT PRIMARY KEY,
  project       TEXT NOT NULL DEFAULT '',
  cwd           TEXT NOT NULL DEFAULT '',
  entrypoint    TEXT NOT NULL DEFAULT '',
  started_at    INTEGER NOT NULL,
  last_event_at INTEGER NOT NULL,
  ended_at      INTEGER,
  event_count   INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS events (
  id          INTEGER PRIMARY KEY,
  ts          INTEGER NOT NULL,
  session_id  TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,
  tool        TEXT,
  tool_use_id TEXT,
  agent_id    TEXT,
  agent_type  TEXT,
  summary     TEXT NOT NULL,
  detail      TEXT NOT NULL DEFAULT '',
  payload     TEXT NOT NULL,
  bytes       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS events_by_session ON events(session_id, ts);
CREATE INDEX IF NOT EXISTS events_by_time ON events(ts);
CREATE INDEX IF NOT EXISTS events_by_tool_use ON events(tool_use_id) WHERE tool_use_id IS NOT NULL;
CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
  summary, detail, content='events', content_rowid='id', tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS events_ai AFTER INSERT ON events BEGIN
  INSERT INTO events_fts(rowid, summary, detail) VALUES (new.id, new.summary, new.detail);
END;
CREATE TRIGGER IF NOT EXISTS events_ad AFTER DELETE ON events BEGIN
  INSERT INTO events_fts(events_fts, rowid, summary, detail) VALUES ('delete', old.id, old.summary, old.detail);
END;
CREATE TABLE IF NOT EXISTS usage_samples (
  ts        INTEGER NOT NULL,
  window    TEXT NOT NULL,
  used      REAL NOT NULL,
  resets_at INTEGER NOT NULL,
  source    TEXT NOT NULL,
  PRIMARY KEY (window, ts)
) WITHOUT ROWID;
INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', '1');
"#;

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_millis(2000))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn open_writer(path: &Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    let fresh: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if fresh == 0 {
        // Only takes effect before the first table exists.
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL").map_err(|e| e.to_string())?;
    }
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(format!("could not switch the recorder database to WAL (got {mode})"));
    }
    conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
    configure(&conn).map_err(|e| e.to_string())?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let has_meta: bool = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='meta'", [], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        > 0;
    let version: i64 = if has_meta {
        conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get::<_, String>(0))
            .optional()
            .map_err(|e| e.to_string())?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    if version > SCHEMA_VERSION {
        return Err(format!("recorder database is from a newer Atoll (schema {version})"));
    }
    if version < 1 {
        conn.execute_batch(SCHEMA_V1).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- describing events

fn one_line(s: &str, max: usize) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    cap(line, max)
}

fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The most telling field of a tool's input.
fn key_field(input: &Value) -> &str {
    for k in ["command", "file_path", "notebook_path", "pattern", "url", "query", "description", "prompt", "skill"] {
        let s = str_at(input, k);
        if !s.is_empty() {
            return s;
        }
    }
    ""
}

/// (summary, detail) for one redacted event.
fn describe(ev: &Value) -> (String, String) {
    let kind = str_at(ev, "hook_event_name");
    let tool = str_at(ev, "tool_name");
    let input = ev.get("tool_input").unwrap_or(&Value::Null);
    let key = key_field(input);
    let with = |label: &str, text: &str| if text.is_empty() { label.to_string() } else { format!("{label}: {}", one_line(text, SUMMARY_CHARS)) };
    let summary = match kind {
        "SessionStart" => with("Session started", str_at(ev, "source")),
        "SessionEnd" => with("Session ended", str_at(ev, "reason")),
        "UserPromptSubmit" => with("You", str_at(ev, "prompt")),
        "PreToolUse" => with(tool, key),
        "PostToolUse" => with(&format!("{tool} done"), key),
        "PostToolUseFailure" => with(&format!("{tool} failed"), str_at(ev, "error")),
        "PermissionRequest" => with(&format!("Asked to use {tool}"), key),
        "Notification" => with("Notice", str_at(ev, "message")),
        "Stop" => with("Finished", str_at(ev, "last_assistant_message")),
        "StopFailure" => with("Stopped", if str_at(ev, "error_details").is_empty() { str_at(ev, "error") } else { str_at(ev, "error_details") }),
        "SubagentStart" => format!("Subagent started: {}", str_at(ev, "agent_type")),
        "SubagentStop" => with(&format!("Subagent finished: {}", str_at(ev, "agent_type")), str_at(ev, "last_assistant_message")),
        "" => "Event".to_string(),
        other => other.to_string(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for s in [str_at(ev, "prompt"), key, str_at(input, "old_string"), str_at(input, "new_string"), str_at(input, "content")] {
        parts.push(s);
    }
    for k in ["message", "last_assistant_message", "error", "error_details"] {
        parts.push(str_at(ev, k));
    }
    let mut seen = HashSet::new();
    let detail = parts.into_iter().filter(|p| !p.is_empty() && seen.insert(*p)).collect::<Vec<_>>().join("\n");
    (cap(&summary, SUMMARY_CHARS), cap(&detail, DETAIL_CHARS))
}

/// Edit / MultiEdit / Write as old/new pairs.
fn diff_of(payload: &Value) -> Option<Vec<DiffHunk>> {
    let tool = str_at(payload, "tool_name");
    let input = payload.get("tool_input")?;
    let path = str_at(input, "file_path").to_string();
    let cut = |s: &str| s.ends_with('…') && s.chars().count() >= 4000;
    let hunk = |old: &str, new: &str| DiffHunk { path: path.clone(), old: old.into(), new: new.into(), truncated: cut(old) || cut(new) };
    match tool {
        "Edit" => Some(vec![hunk(str_at(input, "old_string"), str_at(input, "new_string"))]),
        "Write" => Some(vec![hunk("", str_at(input, "content"))]),
        "MultiEdit" => Some(
            input
                .get("edits")?
                .as_array()?
                .iter()
                .map(|e| hunk(str_at(e, "old_string"), str_at(e, "new_string")))
                .collect(),
        ),
        _ => None,
    }
}

/// Plain words → an FTS5 query: each word quoted (no syntax errors from user
/// text) and prefix-matched.
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

// ---------------------------------------------------------------- writer

enum Job {
    Hook(Value, i64),
    Usage(UsageSample),
    Prune(u32, i64),
    DeleteAll(mpsc::Sender<Result<(), String>>),
    Flush(mpsc::Sender<()>),
}

type Notifier = Arc<dyn Fn(&str) + Send + Sync>;

struct Writer {
    conn: Connection,
    last_usage: HashMap<Window, (i64, f64, u64)>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    owed: HashSet<String>,
    last_notice: HashMap<String, Instant>,
}

impl Writer {
    fn load_last_usage(&mut self) {
        self.last_usage.clear();
        let Ok(mut st) = self.conn.prepare(
            "SELECT window, ts, used, resets_at FROM usage_samples u WHERE ts = (SELECT max(ts) FROM usage_samples WHERE window = u.window)",
        ) else {
            return;
        };
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?)));
        if let Ok(rows) = rows {
            for (w, ts, used, resets) in rows.flatten() {
                if let Some(w) = Window::parse(&w) {
                    self.last_usage.insert(w, (ts, used, resets.max(0) as u64));
                }
            }
        }
    }

    fn insert_hook(&mut self, mut ev: Value, ts: i64) -> rusqlite::Result<String> {
        redact::redact_value(&mut ev);
        let session = match str_at(&ev, "session_id") {
            "" => "unknown".to_string(),
            s => s.to_string(),
        };
        let kind = match str_at(&ev, "hook_event_name") {
            "" => "Unknown",
            k => k,
        }
        .to_string();
        let opt = |k: &str| Some(str_at(&ev, k)).filter(|s| !s.is_empty()).map(String::from);
        let (summary, detail) = describe(&ev);
        let cwd = str_at(&ev, "cwd").to_string();
        let project = Path::new(&cwd).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let entry = str_at(&ev, "crest_entrypoint");
        let entrypoint = if entry.contains("desktop") { "desktop" } else if entry.is_empty() { "" } else { "cli" };
        let ended = (kind == "SessionEnd").then_some(ts);
        let restarted = kind == "SessionStart";
        let payload = ev.to_string();

        self.conn.execute(
            "INSERT INTO sessions(session_id, project, cwd, entrypoint, started_at, last_event_at, ended_at, event_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, 1)
             ON CONFLICT(session_id) DO UPDATE SET
               project = CASE WHEN excluded.project <> '' THEN excluded.project ELSE project END,
               cwd = CASE WHEN excluded.cwd <> '' THEN excluded.cwd ELSE cwd END,
               entrypoint = CASE WHEN excluded.entrypoint <> '' THEN excluded.entrypoint ELSE entrypoint END,
               last_event_at = max(last_event_at, excluded.last_event_at),
               ended_at = CASE WHEN ?7 THEN NULL ELSE coalesce(excluded.ended_at, ended_at) END,
               event_count = event_count + 1",
            params![session, project, cwd, entrypoint, ts, ended, restarted],
        )?;
        self.conn.execute(
            "INSERT INTO events(ts, session_id, kind, tool, tool_use_id, agent_id, agent_type, summary, detail, payload, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![ts, session, kind, opt("tool_name"), opt("tool_use_id"), opt("agent_id"), opt("agent_type"), summary, detail, payload, payload.len() as i64],
        )?;
        Ok(session)
    }

    fn insert_usage(&mut self, s: &UsageSample) -> rusqlite::Result<()> {
        if let Some(&(ts, used, resets)) = self.last_usage.get(&s.window) {
            if (used - s.used).abs() < 0.05 && resets == s.resets_at && s.ts - ts < USAGE_EVERY_MS && s.ts >= ts {
                return Ok(());
            }
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO usage_samples(ts, window, used, resets_at, source) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![s.ts, s.window.as_str(), s.used.clamp(0.0, 100.0), s.resets_at as i64, s.source],
        )?;
        self.last_usage.insert(s.window, (s.ts, s.used, s.resets_at));
        Ok(())
    }

    fn prune(&mut self, days: u32, now: i64) -> rusqlite::Result<()> {
        if days == 0 {
            return Ok(());
        }
        let cutoff = now - days as i64 * 86_400_000;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM events WHERE ts < ?1", [cutoff])?;
        tx.execute(
            "DELETE FROM sessions WHERE last_event_at < ?1 AND NOT EXISTS (SELECT 1 FROM events e WHERE e.session_id = sessions.session_id)",
            [cutoff],
        )?;
        tx.execute("DELETE FROM usage_samples WHERE ts < ?1", [cutoff])?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA wal_checkpoint(PASSIVE);")?;
        self.load_last_usage();
        Ok(())
    }

    fn delete_all(&mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "BEGIN;
             DELETE FROM events;
             DELETE FROM sessions;
             DELETE FROM usage_samples;
             INSERT INTO events_fts(events_fts) VALUES ('rebuild');
             COMMIT;
             VACUUM;
             PRAGMA wal_checkpoint(TRUNCATE);",
        )?;
        self.last_usage.clear();
        self.owed.clear();
        Ok(())
    }

    /// Write a run of Hook/Usage jobs in one transaction.
    fn write_run(&mut self, run: Vec<Job>) {
        let mut touched = Vec::new();
        let result: rusqlite::Result<()> = (|| {
            self.conn.execute_batch("BEGIN")?;
            for job in run {
                match job {
                    Job::Hook(ev, ts) => match self.insert_hook(ev, ts) {
                        Ok(s) => touched.push(s),
                        Err(e) => eprintln!("[crest] recorder: event not saved: {e}"),
                    },
                    Job::Usage(s) => {
                        if let Err(e) = self.insert_usage(&s) {
                            eprintln!("[crest] recorder: usage not saved: {e}");
                        }
                    }
                    _ => {}
                }
            }
            self.conn.execute_batch("COMMIT")
        })();
        if let Err(e) = result {
            eprintln!("[crest] recorder: write failed: {e}");
            let _ = self.conn.execute_batch("ROLLBACK");
            touched.clear();
        }
        self.owed.extend(touched);
    }

    fn send_notices(&mut self) {
        if self.owed.is_empty() {
            return;
        }
        let notifier = self.notifier.lock().unwrap().clone();
        let Some(notify) = notifier else {
            self.owed.clear();
            return;
        };
        let due: Vec<String> = self
            .owed
            .iter()
            .filter(|s| self.last_notice.get(*s).is_none_or(|t| t.elapsed() >= NOTIFY_EVERY))
            .cloned()
            .collect();
        for s in due {
            self.owed.remove(&s);
            self.last_notice.insert(s.clone(), Instant::now());
            notify(&s);
        }
        self.last_notice.retain(|_, t| t.elapsed() < NOTIFY_EVERY * 10);
    }

    fn run(mut self, rx: mpsc::Receiver<Job>) {
        loop {
            // Block with no timeout unless a change notice is still owed.
            let first = if self.owed.is_empty() {
                match rx.recv() {
                    Ok(j) => Some(j),
                    Err(_) => return,
                }
            } else {
                match rx.recv_timeout(NOTIFY_EVERY) {
                    Ok(j) => Some(j),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            };
            let mut batch: Vec<Job> = first.into_iter().collect();
            while batch.len() < BATCH {
                match rx.try_recv() {
                    Ok(j) => batch.push(j),
                    Err(_) => break,
                }
            }
            let mut run: Vec<Job> = Vec::new();
            for job in batch {
                match job {
                    Job::Hook(..) | Job::Usage(_) => run.push(job),
                    control => {
                        if !run.is_empty() {
                            self.write_run(std::mem::take(&mut run));
                        }
                        match control {
                            Job::Prune(days, now) => {
                                if let Err(e) = self.prune(days, now) {
                                    eprintln!("[crest] recorder: clean-up failed: {e}");
                                }
                            }
                            Job::DeleteAll(reply) => {
                                let _ = reply.send(self.delete_all().map_err(|e| e.to_string()));
                            }
                            Job::Flush(reply) => {
                                let _ = reply.send(());
                            }
                            _ => {}
                        }
                    }
                }
            }
            if !run.is_empty() {
                self.write_run(run);
            }
            self.send_notices();
        }
    }
}

// ---------------------------------------------------------------- store

#[derive(Clone)]
pub struct Store {
    tx: mpsc::SyncSender<Job>,
    path: Arc<PathBuf>,
    reader: Arc<Mutex<Option<Connection>>>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    dropped: Arc<AtomicU64>,
}

impl Store {
    /// Opens (creating and migrating if needed) and starts the writer thread.
    pub fn open(path: &Path) -> Result<Store, String> {
        let conn = open_writer(path)?;
        let notifier: Arc<Mutex<Option<Notifier>>> = Arc::new(Mutex::new(None));
        let mut writer = Writer { conn, last_usage: HashMap::new(), notifier: notifier.clone(), owed: HashSet::new(), last_notice: HashMap::new() };
        writer.load_last_usage();
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        std::thread::Builder::new()
            .name("recorder-writer".into())
            .spawn(move || writer.run(rx))
            .map_err(|e| e.to_string())?;
        Ok(Store { tx, path: Arc::new(path.to_path_buf()), reader: Arc::new(Mutex::new(None)), notifier, dropped: Arc::new(AtomicU64::new(0)) })
    }

    /// Called with a session id after its events were written (at most once a second per session).
    pub fn on_change(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        *self.notifier.lock().unwrap() = Some(Arc::new(f));
    }

    fn queue(&self, job: Job) {
        if self.tx.try_send(job).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    // ---- writes: queue only, never block, never error to the caller ----

    pub fn record_hook(&self, event: &Value) {
        self.record_hook_at(event.clone(), now_ms());
    }

    /// Same as `record_hook` with an explicit receive time (simulator, tests).
    pub fn record_hook_at(&self, event: Value, ts: i64) {
        self.queue(Job::Hook(event, ts));
    }

    pub fn record_usage(&self, sample: UsageSample) {
        self.queue(Job::Usage(sample));
    }

    pub fn prune(&self, days: u32) {
        self.queue(Job::Prune(days, now_ms()));
    }

    /// Prune relative to a given "now" (simulator, tests).
    pub fn prune_at(&self, days: u32, now: i64) {
        self.queue(Job::Prune(days, now));
    }

    /// Delete everything and VACUUM. Waits until done.
    pub fn delete_all(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Job::DeleteAll(tx)).map_err(|_| "recorder is not running".to_string())?;
        rx.recv().map_err(|_| "recorder stopped".to_string())?
    }

    /// Wait until everything queued so far is written (tests, benchmarks, shutdown).
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    // ---- reads: synchronous, call from spawn_blocking ----

    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut guard = self.reader.lock().unwrap();
        if guard.is_none() {
            let conn = Connection::open_with_flags(&*self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .map_err(|e| e.to_string())?;
            configure(&conn).map_err(|e| e.to_string())?;
            *guard = Some(conn);
        }
        f(guard.as_ref().unwrap()).map_err(|e| e.to_string())
    }

    pub fn sessions(&self, q: &SessionQuery) -> Result<Vec<SessionRow>, String> {
        let since = q.since_ms.unwrap_or(0);
        let limit = q.limit.unwrap_or(50).min(1000);
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT session_id, project, entrypoint, started_at, last_event_at, ended_at, event_count
                 FROM sessions WHERE last_event_at >= ?1 ORDER BY last_event_at DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![since, limit], |r| {
                Ok(SessionRow {
                    session_id: r.get(0)?,
                    project: r.get(1)?,
                    entrypoint: r.get(2)?,
                    started_at: r.get(3)?,
                    last_event_at: r.get(4)?,
                    ended_at: r.get(5)?,
                    event_count: r.get(6)?,
                })
            })?;
            rows.collect()
        })
    }

    fn event_row(r: &rusqlite::Row) -> rusqlite::Result<EventRow> {
        Ok(EventRow { id: r.get(0)?, ts: r.get(1)?, session_id: r.get(2)?, kind: r.get(3)?, tool: r.get(4)?, agent_id: r.get(5)?, summary: r.get(6)? })
    }

    pub fn events(&self, q: &EventQuery) -> Result<Vec<EventRow>, String> {
        let mut sql = String::from("SELECT id, ts, session_id, kind, tool, agent_id, summary FROM events WHERE 1=1");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(s) = &q.session_id {
            sql.push_str(" AND session_id = ?");
            args.push(s.clone().into());
        }
        if let Some(t) = q.since_ms {
            sql.push_str(" AND ts >= ?");
            args.push(t.into());
        }
        if let Some(t) = q.until_ms {
            sql.push_str(" AND ts <= ?");
            args.push(t.into());
        }
        if let Some(kinds) = q.kinds.as_ref().filter(|k| !k.is_empty()) {
            sql.push_str(" AND kind IN (");
            sql.push_str(&vec!["?"; kinds.len()].join(","));
            sql.push(')');
            args.extend(kinds.iter().map(|k| k.clone().into()));
        }
        sql.push_str(" ORDER BY ts, id LIMIT ?");
        args.push((q.limit.unwrap_or(500).min(5000) as i64).into());
        self.read(|c| {
            let mut st = c.prepare(&sql)?;
            let rows = st.query_map(params_from_iter(args), Self::event_row)?;
            rows.collect()
        })
    }

    pub fn event(&self, id: EventId) -> Result<Option<EventDetail>, String> {
        self.read(|c| {
            c.query_row(
                "SELECT id, ts, session_id, kind, tool, agent_id, summary, tool_use_id, agent_type, detail, payload FROM events WHERE id = ?1",
                [id],
                |r| {
                    let payload: Value = serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Null);
                    Ok(EventDetail {
                        row: Self::event_row(r)?,
                        tool_use_id: r.get(7)?,
                        agent_type: r.get(8)?,
                        detail: r.get(9)?,
                        diff: diff_of(&payload),
                        payload,
                    })
                },
            )
            .optional()
        })
    }

    pub fn search(&self, text: &str, limit: u32) -> Result<Vec<EventRow>, String> {
        let Some(q) = fts_query(text) else { return Ok(vec![]) };
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT e.id, e.ts, e.session_id, e.kind, e.tool, e.agent_id, e.summary
                 FROM events_fts f JOIN events e ON e.id = f.rowid
                 WHERE events_fts MATCH ?1 ORDER BY e.ts DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![q, limit.clamp(1, 500)], Self::event_row)?;
            rows.collect()
        })
    }

    pub fn breakdown(&self, session_id: &str) -> Result<Breakdown, String> {
        type Raw = (i64, String, Option<String>, Option<String>, Option<String>, Option<String>);
        let raw: Vec<Raw> = self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT ts, kind, tool, tool_use_id, agent_id, json_extract(payload, '$.notification_type')
                 FROM events WHERE session_id = ?1 ORDER BY ts, id",
            )?;
            let rows = st.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
            rows.collect()
        })?;
        let evs: Vec<breakdown::Ev> = raw
            .iter()
            .map(|(ts, kind, tool, id, agent, nt)| breakdown::Ev {
                ts: *ts,
                kind,
                tool: tool.as_deref(),
                tool_use_id: id.as_deref(),
                agent_id: agent.as_deref(),
                notification_type: nt.as_deref(),
            })
            .collect();
        Ok(breakdown::compute(&evs))
    }

    pub fn agent_tree(&self, session_id: &str) -> Result<Vec<AgentNode>, String> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT agent_id, max(agent_type), min(ts), max(CASE WHEN kind = 'SubagentStop' THEN ts END), count(*)
                 FROM events WHERE session_id = ?1 AND agent_id IS NOT NULL
                 GROUP BY agent_id ORDER BY min(ts)",
            )?;
            let rows = st.query_map([session_id], |r| {
                Ok(AgentNode { agent_id: r.get(0)?, agent_type: r.get(1)?, parent: None, started_at: r.get(2)?, ended_at: r.get(3)?, event_count: r.get(4)? })
            })?;
            rows.collect()
        })
    }

    pub fn usage_samples(&self, window: Window, since_ms: i64) -> Result<Vec<UsageSample>, String> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT ts, used, resets_at, source FROM usage_samples WHERE window = ?1 AND ts >= ?2 ORDER BY ts",
            )?;
            let rows = st.query_map(params![window.as_str(), since_ms], |r| {
                Ok(UsageSample { ts: r.get(0)?, window, used: r.get(1)?, resets_at: r.get::<_, i64>(2)?.max(0) as u64, source: r.get(3)? })
            })?;
            rows.collect()
        })
    }

    pub fn stats(&self) -> Result<StoreStats, String> {
        let file_bytes = [self.path.to_path_buf(), PathBuf::from(format!("{}-wal", self.path.display()))]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len() as i64)
            .sum();
        let dropped = self.dropped.load(Ordering::Relaxed);
        self.read(|c| {
            let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
            Ok(StoreStats {
                events: n("SELECT count(*) FROM events")?,
                sessions: n("SELECT count(*) FROM sessions")?,
                usage_samples: n("SELECT count(*) FROM usage_samples")?,
                file_bytes,
                oldest_ts: c.query_row("SELECT min(ts) FROM events", [], |r| r.get(0))?,
                dropped,
            })
        })
    }
}

#[cfg(test)]
mod tests;
