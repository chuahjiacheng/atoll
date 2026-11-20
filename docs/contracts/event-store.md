# Contract: event store (flight recorder)

Owner: `recorder-forecast` session. Status: **v1 draft, published for other
sessions to code against.** Changes after this point are additive only
(new columns with defaults, new functions); anything breaking bumps
`schema_version` and is announced in this file's changelog.

## 1. Rules for other sessions

- Never open the database file yourself. Read and write only through the
  Rust functions in section 4 (or the Tauri commands in section 5).
- Never block a hook response on the store. `record_*` calls only queue
  work; they return immediately and never fail the caller.
- Everything stored has already been redacted (section 3). Do not store
  raw payloads anywhere else to get around that.
- If you need a field or query that is not here, write the request in
  `docs/requests/<your-branch>.md`. Until it lands, code against a stub.

## 2. Storage

- File: `%LOCALAPPDATA%\Crest\recorder\events.db` (SQLite 3, via
  `rusqlite` with the `bundled` feature, FTS5 on).
- Pragmas on every connection: `journal_mode=WAL`, `synchronous=NORMAL`,
  `foreign_keys=ON`, `busy_timeout=2000`.
- One writer thread owns the only write connection. Readers each open
  their own read-only connection (WAL lets them run while it writes).
- Times are **milliseconds since the Unix epoch, UTC**, as `INTEGER`,
  except `resets_at`, which keeps Claude Code's unit (seconds).

### Schema v1

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);                                    -- ('schema_version','1')

CREATE TABLE sessions (
  session_id    TEXT PRIMARY KEY,
  project       TEXT NOT NULL DEFAULT '',   -- folder name of cwd
  cwd           TEXT NOT NULL DEFAULT '',   -- redacted path
  entrypoint    TEXT NOT NULL DEFAULT '',   -- 'cli' | 'desktop' | ''
  started_at    INTEGER NOT NULL,           -- first event seen
  last_event_at INTEGER NOT NULL,
  ended_at      INTEGER,                    -- SessionEnd, else NULL
  event_count   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE events (
  id           INTEGER PRIMARY KEY,         -- increases with arrival order
  ts           INTEGER NOT NULL,            -- when Atoll received it
  session_id   TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
  kind         TEXT NOT NULL,               -- hook_event_name, e.g. 'PreToolUse'
  tool         TEXT,                        -- tool_name, if any
  tool_use_id  TEXT,                        -- pairs Pre/Post/PermissionRequest
  agent_id     TEXT,                        -- subagent that produced it, NULL = main agent [Unverified field]
  agent_type   TEXT,                        -- subagent type, if given [Unverified field]
  summary      TEXT NOT NULL,               -- one line, redacted, <= 200 chars
  detail       TEXT NOT NULL DEFAULT '',    -- searchable text, redacted (command, path, prompt, reply)
  payload      TEXT NOT NULL,               -- full event JSON, redacted
  bytes        INTEGER NOT NULL             -- length of payload
);
CREATE INDEX events_by_session ON events(session_id, ts);
CREATE INDEX events_by_time    ON events(ts);
CREATE INDEX events_by_tool_use ON events(tool_use_id) WHERE tool_use_id IS NOT NULL;

-- Full-text search over summary + detail (external content, kept in step by triggers).
CREATE VIRTUAL TABLE events_fts USING fts5(
  summary, detail, content='events', content_rowid='id', tokenize='unicode61'
);

CREATE TABLE usage_samples (
  ts        INTEGER NOT NULL,
  window    TEXT NOT NULL,                  -- 'five_hour' | 'seven_day'
  used      REAL NOT NULL,                  -- 0..100
  resets_at INTEGER NOT NULL,               -- Unix SECONDS, 0 = no window running
  source    TEXT NOT NULL,                  -- 'statusline' | 'signin' | 'log'
  PRIMARY KEY (window, ts)
) WITHOUT ROWID;
```

A usage sample is written only when `used` or `resets_at` changed, or ten
minutes passed since the last sample for that window, so a status line
refreshing every 60 s does not fill the table.

## 3. Redaction (applied before anything is written)

1. **By key.** Any JSON object key matching
   `(?i)(token|secret|password|passwd|api[_-]?key|authorization|cookie|credential|private[_-]?key|session[_-]?key)`
   has its value replaced by `"[redacted]"`. (`session_id` and
   `tool_use_id` are exempt: they are identifiers, not secrets.)
2. **By value pattern**, inside every string (including commands, file
   contents in `Edit`/`Write` inputs, prompts and replies):
   - Anthropic / OpenAI style keys: `sk-ant-…`, `sk-…` (20+ chars)
   - GitHub: `ghp_…`, `gho_…`, `ghu_…`, `ghs_…`, `github_pat_…`
   - AWS access key IDs: `AKIA…` / `ASIA…` (16 more chars)
   - Slack: `xox[abprs]-…`; Google API: `AIza…` (35 more chars)
   - `Bearer <token>` and `Basic <base64>` in headers
   - JWTs: three base64url parts starting `eyJ`
   - PEM blocks: `-----BEGIN … PRIVATE KEY-----` to the matching END
   - Passwords in URLs: `scheme://user:pass@host` → `scheme://user:[redacted]@host`
   - Assignments with a secret-like name: `API_KEY=…`, `"password": "…"`,
     `--token …`, `$env:SECRET = …`
   Each match becomes `[redacted]`.
3. Redaction is one pure function, `recorder::redact::redact_value(&mut Value)`,
   with its own unit tests. Other sessions may call it.
4. Known limit: a secret with no recognisable name or shape (for example a
   random password typed into a prompt with no label) cannot be detected.
   The UI says so next to the "delete all" button.

Note: `atoll-bridge` already drops `tool_response` and cuts every string
to 4,000 characters before Atoll sees the event, so the store never holds
tool output, and long `Edit` contents are stored cut short.

## 4. Rust interface (`src-tauri/src/recorder/mod.rs`)

```rust
pub type EventId = i64;

/// Cheap to clone; held in Tauri state as `recorder::Store`.
#[derive(Clone)]
pub struct Store { /* writer channel + db path */ }

impl Store {
    /// Opens (creating and migrating if needed) and starts the writer thread.
    pub fn open(path: &std::path::Path) -> Result<Store, String>;

    // ---- writes: queue only, never block, never error to the caller ----
    /// One Claude Code hook event, exactly as `/hook` received it.
    pub fn record_hook(&self, event: &serde_json::Value);
    /// One reading of a limit window.
    pub fn record_usage(&self, sample: UsageSample);
    /// Delete events and usage samples older than `days` (0 = keep forever).
    pub fn prune(&self, days: u32);
    /// Delete everything and VACUUM. Resolves once done.
    pub fn delete_all(&self) -> Result<(), String>;

    // ---- reads: synchronous, call from spawn_blocking ----
    pub fn sessions(&self, q: &SessionQuery) -> Result<Vec<SessionRow>, String>;
    pub fn events(&self, q: &EventQuery) -> Result<Vec<EventRow>, String>;
    pub fn event(&self, id: EventId) -> Result<Option<EventDetail>, String>;
    pub fn search(&self, text: &str, limit: u32) -> Result<Vec<EventRow>, String>;
    pub fn breakdown(&self, session_id: &str) -> Result<Breakdown, String>;
    pub fn agent_tree(&self, session_id: &str) -> Result<Vec<AgentNode>, String>;
    pub fn usage_samples(&self, window: Window, since_ms: i64) -> Result<Vec<UsageSample>, String>;
    pub fn stats(&self) -> Result<StoreStats, String>;
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Window { FiveHour, SevenDay }

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSample { pub ts: i64, pub window: Window, pub used: f64, pub resets_at: u64, pub source: String }

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQuery { pub since_ms: Option<i64>, pub limit: Option<u32> }   // newest first, default 50

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventQuery {
    pub session_id: Option<String>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub kinds: Option<Vec<String>>,   // hook_event_name filter
    pub limit: Option<u32>,           // default 500, max 5000; oldest first
}

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct SessionRow { pub session_id: String, pub project: String, pub entrypoint: String,
    pub started_at: i64, pub last_event_at: i64, pub ended_at: Option<i64>, pub event_count: i64 }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct EventRow { pub id: EventId, pub ts: i64, pub session_id: String, pub kind: String,
    pub tool: Option<String>, pub agent_id: Option<String>, pub summary: String }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct EventDetail { #[serde(flatten)] pub row: EventRow, pub tool_use_id: Option<String>,
    pub agent_type: Option<String>, pub detail: String, pub payload: serde_json::Value,
    /// For Edit / MultiEdit / Write: the change as old/new text pairs (redacted, may be cut short).
    pub diff: Option<Vec<DiffHunk>> }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct DiffHunk { pub path: String, pub old: String, pub new: String, pub truncated: bool }

/// Where a session's wall-clock time went, in ms. Buckets sum to `total`.
#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct Breakdown { pub total: i64, pub thinking: i64, pub tools: Vec<(String, i64)>,
    pub waiting_for_you: i64, pub idle: i64 }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct AgentNode { pub agent_id: String, pub agent_type: Option<String>,
    pub parent: Option<String>, pub started_at: i64, pub ended_at: Option<i64>, pub event_count: i64 }

#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct StoreStats { pub events: i64, pub sessions: i64, pub usage_samples: i64,
    pub file_bytes: i64, pub oldest_ts: Option<i64> }
```

Breakdown rules (so other features agree on the numbers):
- **thinking**: `UserPromptSubmit` → first tool or `Stop`, and `PostToolUse` → next `PreToolUse` / `Stop`.
- **tools**: `PreToolUse` → matching `PostToolUse`/`PostToolUseFailure` (by `tool_use_id`), per tool name, minus any `PermissionRequest` wait inside it.
- **waiting_for_you**: `PermissionRequest` → its tool's next event, and `Notification` → next `UserPromptSubmit`.
- **idle**: `Stop` → next `UserPromptSubmit`, capped at 30 min per gap (longer gaps count as the session being away, not part of `total`).

## 5. Tauri commands (invoke names, camelCase args)

| Command | Args | Returns |
|---|---|---|
| `recorder_sessions` | `query: SessionQuery` | `SessionRow[]` |
| `recorder_events` | `query: EventQuery` | `EventRow[]` |
| `recorder_event` | `id: number` | `EventDetail \| null` |
| `recorder_search` | `text: string, limit?: number` | `EventRow[]` |
| `recorder_breakdown` | `sessionId: string` | `Breakdown` |
| `recorder_agent_tree` | `sessionId: string` | `AgentNode[]` |
| `recorder_usage` | `window: "five_hour" \| "seven_day", sinceMs: number` | `UsageSample[]` |
| `recorder_stats` | – | `StoreStats` |
| `recorder_delete_all` | – | `null` |
| `forecast_current` | – | `Forecast` (see `docs/contracts/forecast.md`, published with Part 2) |

Frontend event: `recorder:changed` with `{ sessionId }`, at most once per
second per session, so open views can refresh without polling.

## 6. Settings (one block in `settings.rs`)

```rust
pub struct Recorder { pub enabled: bool /* true */, pub retention_days: u32 /* 30; 0 = forever */ }
```

## Changelog

- v1 (2026-11-10): first draft.
