//! Turns raw Claude Code hook events into per-session state for the island.
//!
//! Hook event reference: https://code.claude.com/docs/en/hooks
//! Every event carries `session_id`, `cwd` and `hook_event_name`.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

/// Sessions with no events for this long are dropped (terminal closed without SessionEnd).
const STALE_AFTER_MS: u64 = 3 * 60 * 60 * 1000;

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Session open, waiting for a prompt.
    Idle,
    /// Claude is working on a prompt.
    Working,
    /// Claude needs the user (permission prompt, question, idle reminder).
    Attention,
    /// Claude finished its turn.
    Done,
    /// The turn ended on an error (API error, rate limit...).
    Error,
}

/// One line of the Agent tab's "Live activity" list.
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub session_id: String,
    pub project: String,
    /// "read" | "edit" | "run" | "web" | "agent" | "tool"
    pub kind: String,
    /// Past tense: "Read", "Edited", "Ran"...
    pub verb: String,
    pub target: String,
    pub at: u64,
}

/// One finished turn for the Agent tab's "History" list.
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub session_id: String,
    pub project: String,
    /// The prompt that started the turn (shortened), or the project name.
    pub title: String,
    /// "done" | "error"
    pub outcome: String,
    pub at: u64,
}

#[derive(Clone, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub log: Vec<LogEntry>,
    pub history: Vec<HistoryEntry>,
}

const LOG_LEN: usize = 40;
const HISTORY_LEN: usize = 12;

fn past_tense(label: &str) -> &'static str {
    match label {
        "Reading" => "Read",
        "Searching" | "Searching web" => "Searched",
        "Finding" => "Found",
        "Editing" => "Edited",
        "Writing" => "Wrote",
        "Running" => "Ran",
        "Fetching" => "Fetched",
        "Delegating" => "Delegated",
        "Planning" => "Planned",
        _ => "Used",
    }
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// "read" | "edit" | "run" | "web" | "agent" | "tool"
    pub kind: String,
    /// Human label, e.g. "Reading", "Editing", "Running".
    pub label: String,
    /// File path, command, URL... (already shortened).
    pub detail: String,
    /// True while the tool call is in progress (PreToolUse seen, PostToolUse not yet).
    pub active: bool,
}

/// A question Claude ended its turn with, with reply choices. Comes from the
/// Claude desktop app's "next step" box (its `wrap_up` tool): picking a choice
/// sends that text to Claude as your reply.
#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub question: String,
    /// Reply buttons, recommended first. May be empty (then: answer in Claude).
    pub choices: Vec<String>,
    pub at: u64,
}

/// The desktop app's end-of-turn box (`mcp__ccd_turn__wrap_up`).
pub fn is_next_step_tool(tool: &str) -> bool {
    tool.ends_with("ccd_turn__wrap_up")
}

/// An app action button (commit, create PR...) as a reply Claude can act on, or
/// None for ones only the app itself can do (archive, new session).
fn app_action_reply(action: &str) -> Option<&'static str> {
    Some(match action {
        "commit" => "Commit the changes",
        "create_pr" => "Create a pull request",
        "merge" => "Merge the pull request",
        "fix_ci" => "Fix the failing CI",
        "address_comments" => "Address the review comments",
        "resolve_conflicts" => "Resolve the merge conflicts",
        "verify" => "Verify it works",
        _ => return None,
    })
}

/// The question and reply choices in a `wrap_up` call, if it asks one. App
/// actions become a plain reply asking Claude to do the same thing; ones only
/// the app can do are left out.
pub fn ask_from_next_step(input: &Value, now: u64) -> Option<Ask> {
    let question = input.get("ask").and_then(Value::as_str).unwrap_or("").trim();
    if question.is_empty() {
        return None;
    }
    let choices = input
        .get("actions")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| match a.get("action").and_then(Value::as_str).unwrap_or("") {
                    "" => a.get("label").and_then(Value::as_str),
                    action => app_action_reply(action),
                })
                .map(|l| shorten(l.trim(), 60))
                .filter(|l| !l.is_empty())
                .take(4)
                .collect()
        })
        .unwrap_or_default();
    Some(Ask { question: shorten(question, 200), choices, at: now })
}

/// One step of Claude's own task list (TodoWrite, TaskCreate/TaskUpdate, or the
/// desktop app's update_plan).
#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub content: String,
    /// "pending" | "in_progress" | "completed"
    pub status: String,
    /// Task id for TaskCreate/TaskUpdate lists.
    #[serde(skip)]
    pub id: String,
}

fn plan_status(v: &Value) -> String {
    match v.as_str().unwrap_or("pending") {
        "in_progress" => "in_progress",
        "completed" | "deleted" => "completed",
        _ => "pending",
    }
    .to_string()
}

/// Update `plan` from a task-list tool call. Returns true if the call was one.
pub fn apply_plan(plan: &mut Vec<PlanStep>, tool: &str, input: &Value) -> bool {
    let step = |item: &Value, text_keys: &[&str]| {
        let status = plan_status(item.get("status").unwrap_or(&Value::Null));
        // TodoWrite's activeForm reads better while the step runs ("Running tests").
        let mut keys: Vec<&str> = Vec::new();
        if status == "in_progress" {
            keys.push("activeForm");
        }
        keys.extend_from_slice(text_keys);
        let content = keys.iter().map(|k| str_field(item, k)).find(|t| !t.trim().is_empty()).unwrap_or("");
        PlanStep { content: shorten(content, 100), status, id: String::new() }
    };
    let finish_running = |plan: &mut Vec<PlanStep>| {
        for s in plan.iter_mut().filter(|s| s.status == "in_progress") {
            s.status = "completed".into();
        }
    };
    match tool {
        "TodoWrite" => {
            if let Some(items) = input.get("todos").and_then(Value::as_array) {
                *plan = items.iter().map(|i| step(i, &["content"])).collect();
            }
        }
        "mcp__ccd_turn__update_plan" => {
            if let Some(items) = input.get("items").and_then(Value::as_array) {
                *plan = items.iter().map(|i| step(i, &["content"])).collect();
            }
            for n in input.get("done").and_then(Value::as_array).into_iter().flatten() {
                if let Some(s) = n.as_u64().and_then(|n| plan.get_mut((n as usize).wrapping_sub(1))) {
                    s.status = "completed".into();
                }
            }
            if input.get("advance").and_then(Value::as_bool) == Some(true) {
                finish_running(plan);
                if let Some(s) = plan.iter_mut().find(|s| s.status == "pending") {
                    s.status = "in_progress".into();
                }
            }
            if let Some(n) = input.get("start").and_then(Value::as_u64).map(|n| n as usize) {
                if n >= 1 && n <= plan.len() {
                    finish_running(plan);
                    plan[n - 1].status = "in_progress".into();
                }
            }
        }
        "mcp__ccd_turn__wrap_up" => {
            for s in plan.iter_mut() {
                s.status = "completed".into();
            }
        }
        "TaskCreate" => {
            let mut s = step(input, &["subject", "description"]);
            s.id = (plan.len() + 1).to_string();
            plan.push(s);
        }
        "TaskUpdate" => {
            let id = match input.get("taskId") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => return true,
            };
            if let Some(s) = plan.iter_mut().find(|s| s.id == id) {
                if let Some(st) = input.get("status") {
                    s.status = plan_status(st);
                }
                let subject = str_field(input, "subject");
                if !subject.is_empty() {
                    s.content = shorten(subject, 100);
                }
            }
        }
        _ => return false,
    }
    true
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    /// Folder name of `cwd`, used as the display name.
    pub project: String,
    pub cwd: String,
    /// "terminal" or "desktop" (Claude desktop app Code tab).
    pub source: String,
    pub status: Status,
    pub activity: Option<Activity>,
    /// Claude's own task list for this turn (empty when it made none).
    pub plan: Vec<PlanStep>,
    /// Latest notification or final assistant message (shortened).
    pub message: Option<String>,
    /// The prompt of the current or last turn (shortened).
    pub prompt: Option<String>,
    /// Claude's full final message of the last turn ("View answer"), capped.
    pub answer: Option<String>,
    pub tool_count: u32,
    /// Claude's end-of-turn question with reply choices, until you answer or clear it.
    pub ask: Option<Ask>,
    pub started_at: u64,
    pub updated_at: u64,
}

#[derive(Default)]
pub struct SessionStore {
    sessions: HashMap<String, Session>,
    log: VecDeque<LogEntry>,
    history: VecDeque<HistoryEntry>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Keep the first line, at most `max` characters.
pub fn shorten(s: &str, max: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Last path component, accepting both `\` and `/` separators.
fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).find(|p| !p.is_empty()).unwrap_or(path).to_string()
}

/// Map a tool call to what the island shows: "Reading foo.rs", "Running npm test", ...
pub fn describe_tool(tool: &str, input: &Value) -> Activity {
    let path = || {
        let p = str_field(input, "file_path");
        let p = if p.is_empty() { str_field(input, "notebook_path") } else { p };
        let p = if p.is_empty() { str_field(input, "path") } else { p };
        file_name(p)
    };
    let (kind, label, detail) = match tool {
        "Read" => ("read", "Reading", path()),
        "Grep" => ("read", "Searching", shorten(str_field(input, "pattern"), 60)),
        "Glob" => ("read", "Finding", shorten(str_field(input, "pattern"), 60)),
        "Edit" | "MultiEdit" | "NotebookEdit" => ("edit", "Editing", path()),
        "Write" => ("edit", "Writing", path()),
        "Bash" | "PowerShell" => ("run", "Running", shorten(str_field(input, "command"), 80)),
        "WebFetch" => ("web", "Fetching", shorten(str_field(input, "url"), 80)),
        "WebSearch" => ("web", "Searching web", shorten(str_field(input, "query"), 80)),
        "Task" | "Agent" => ("agent", "Delegating", shorten(str_field(input, "description"), 60)),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" | "mcp__ccd_turn__update_plan" => {
            ("tool", "Planning", String::new())
        }
        "mcp__ccd_turn__wrap_up" => ("tool", "Wrapping up", String::new()),
        t if t.starts_with("mcp__") => {
            let name = t.rsplit("__").next().unwrap_or(t);
            ("tool", "Using", name.replace('_', " "))
        }
        t => ("tool", "Using", t.to_string()),
    };
    Activity { kind: kind.into(), label: label.into(), detail, active: true }
}

impl SessionStore {
    /// Apply one hook event. Returns true if anything visible changed.
    pub fn apply(&mut self, event: &Value) -> bool {
        let id = str_field(event, "session_id");
        if id.is_empty() {
            return false;
        }
        let name = str_field(event, "hook_event_name");
        let now = now_ms();

        if name == "SessionEnd" {
            return self.sessions.remove(id).is_some();
        }

        let session = self.sessions.entry(id.to_string()).or_insert_with(|| {
            let cwd = str_field(event, "cwd").to_string();
            let source = if str_field(event, "crest_entrypoint").contains("desktop") { "desktop" } else { "terminal" };
            Session {
                id: id.to_string(),
                project: file_name(&cwd),
                cwd,
                source: source.into(),
                status: Status::Idle,
                activity: None,
                plan: Vec::new(),
                message: None,
                prompt: None,
                answer: None,
                tool_count: 0,
                ask: None,
                started_at: now,
                updated_at: now,
            }
        });
        session.updated_at = now;

        match name {
            "SessionStart" => {
                session.status = Status::Idle;
            }
            "UserPromptSubmit" => {
                session.status = Status::Working;
                session.activity = None;
                session.message = None;
                session.tool_count = 0;
                // A finished list belongs to the last turn; an unfinished one carries on.
                if session.plan.iter().all(|s| s.status == "completed") {
                    session.plan.clear();
                }
                session.started_at = now;
                session.answer = None;
                session.ask = None;
                let prompt = shorten(str_field(event, "prompt"), 120);
                session.prompt = if prompt.is_empty() { None } else { Some(prompt) };
            }
            "PreToolUse" if is_next_step_tool(str_field(event, "tool_name")) => {
                // Bookkeeping, not work: keep it out of the activity log.
                let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
                session.ask = ask_from_next_step(&input, now);
            }
            "PreToolUse" => {
                let tool = str_field(event, "tool_name");
                let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
                session.status = Status::Working;
                apply_plan(&mut session.plan, tool, &input);
                let a = describe_tool(tool, &input);
                let entry = LogEntry {
                    session_id: session.id.clone(),
                    project: session.project.clone(),
                    kind: a.kind.clone(),
                    verb: past_tense(&a.label).to_string(),
                    target: a.detail.clone(),
                    at: now,
                };
                session.activity = Some(a);
                session.tool_count += 1;
                self.log.push_front(entry);
                self.log.truncate(LOG_LEN);
            }
            "PostToolUse" | "PostToolUseFailure" => {
                if let Some(a) = session.activity.as_mut() {
                    a.active = false;
                }
                // A permission prompt that got answered in the terminal lands here too.
                if session.status == Status::Attention {
                    session.status = Status::Working;
                }
            }
            "PermissionRequest" => {
                let tool = str_field(event, "tool_name");
                let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
                let a = describe_tool(tool, &input);
                session.status = Status::Attention;
                session.message = Some(shorten(&format!("Permission: {} {}", a.label, a.detail), 160));
            }
            "Notification" => {
                // permission_prompt, idle_prompt, elicitation_dialog, ...
                session.status = Status::Attention;
                session.message = Some(shorten(str_field(event, "message"), 160));
            }
            "Stop" => {
                // Ended on a question: it waits for you ("Needs you"), not just "Done".
                session.status = if session.ask.is_some() { Status::Attention } else { Status::Done };
                if let Some(a) = session.activity.as_mut() {
                    a.active = false;
                }
                let last = str_field(event, "last_assistant_message");
                if !last.is_empty() {
                    session.message = Some(shorten(last, 200));
                    session.answer = Some(last.trim().chars().take(4000).collect());
                }
                if let Some(ask) = &session.ask {
                    session.message = Some(ask.question.clone());
                }
                let entry = HistoryEntry {
                    session_id: session.id.clone(),
                    project: session.project.clone(),
                    title: session.prompt.clone().unwrap_or_else(|| session.project.clone()),
                    outcome: "done".into(),
                    at: now,
                };
                self.history.push_front(entry);
                self.history.truncate(HISTORY_LEN);
            }
            "StopFailure" => {
                // Turn ended on an API error (rate limit, auth, overloaded...).
                session.status = Status::Error;
                let kind = str_field(event, "error");
                let detail = str_field(event, "error_details");
                let detail = if detail.is_empty() { kind } else { detail };
                session.message = Some(if kind == "rate_limit" {
                    "Usage limit reached".to_string()
                } else {
                    shorten(&format!("Stopped: {detail}"), 160)
                });
                let entry = HistoryEntry {
                    session_id: session.id.clone(),
                    project: session.project.clone(),
                    title: session.prompt.clone().unwrap_or_else(|| session.project.clone()),
                    outcome: "error".into(),
                    at: now,
                };
                self.history.push_front(entry);
                self.history.truncate(HISTORY_LEN);
            }
            "SubagentStop" => {}
            _ => {}
        }
        true
    }

    /// Sessions newest-first, dropping stale ones.
    pub fn snapshot(&mut self) -> Vec<Session> {
        let now = now_ms();
        self.sessions.retain(|_, s| now.saturating_sub(s.updated_at) < STALE_AFTER_MS);
        let mut list: Vec<Session> = self.sessions.values().cloned().collect();
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        list
    }

    /// Recent tool activity and finished turns, newest first.
    pub fn feed(&self) -> Feed {
        Feed { log: self.log.iter().cloned().collect(), history: self.history.iter().cloned().collect() }
    }

    pub fn project_of(&self, id: &str) -> String {
        self.sessions.get(id).map(|s| s.project.clone()).unwrap_or_default()
    }

    /// After a permission answer the session is working again (allow) or
    /// Claude is told no and carries on (deny) - both mean "working".
    pub fn resume(&mut self, id: &str) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) if s.status == Status::Attention => {
                s.status = Status::Working;
                s.message = None;
                true
            }
            _ => false,
        }
    }

    /// Clear a session's message (an error, a "needs you" or a "done"): it goes
    /// back to idle and the island stops showing it. The session, its last
    /// answer and its history stay; the next hook event sets the status again.
    pub fn clear(&mut self, id: &str) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) if matches!(s.status, Status::Error | Status::Attention | Status::Done) => {
                s.status = Status::Idle;
                s.message = None;
                s.ask = None;
                if let Some(a) = s.activity.as_mut() {
                    a.active = false;
                }
                true
            }
            _ => false,
        }
    }

    /// Is `text` one of the reply choices this session's question offers?
    pub fn is_choice(&self, id: &str, text: &str) -> bool {
        self.sessions.get(id).and_then(|s| s.ask.as_ref()).is_some_and(|a| a.choices.iter().any(|c| c == text))
    }

    /// You answered the end-of-turn question (from the island): Claude is
    /// about to get your reply, so the question goes.
    pub fn answered(&mut self, id: &str) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) if s.ask.is_some() => {
                s.ask = None;
                s.status = Status::Working;
                s.message = None;
                true
            }
            _ => false,
        }
    }

    pub fn dismiss(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tracks_a_turn() {
        let mut store = SessionStore::default();
        let base = |name: &str| json!({"session_id": "s1", "cwd": "C:\\work\\demo", "hook_event_name": name});
        store.apply(&base("SessionStart"));
        store.apply(&base("UserPromptSubmit"));
        let mut pre = base("PreToolUse");
        pre["tool_name"] = json!("Read");
        pre["tool_input"] = json!({"file_path": "C:\\work\\demo\\src\\main.rs"});
        store.apply(&pre);
        let s = &store.snapshot()[0];
        assert_eq!(s.project, "demo");
        assert_eq!(s.status, Status::Working);
        let a = s.activity.as_ref().unwrap();
        assert_eq!((a.label.as_str(), a.detail.as_str()), ("Reading", "main.rs"));

        let mut stop = base("Stop");
        stop["last_assistant_message"] = json!("All done.\nDetails...");
        store.apply(&stop);
        let s = &store.snapshot()[0];
        assert_eq!(s.status, Status::Done);
        assert_eq!(s.message.as_deref(), Some("All done."));

        let mut fail = base("StopFailure");
        fail["error"] = json!("rate_limit");
        store.apply(&fail);
        let s = &store.snapshot()[0];
        assert_eq!((s.status, s.message.as_deref()), (Status::Error, Some("Usage limit reached")));
        assert!(store.clear("s1"));
        let s = &store.snapshot()[0];
        assert_eq!((s.status, s.message.is_none()), (Status::Idle, true));
        assert!(s.answer.is_some(), "clearing keeps the last answer");
        assert!(!store.clear("s1"), "nothing left to clear");

        // The desktop app's next-step box: a question with two reply buttons and an app action.
        store.apply(&base("UserPromptSubmit"));
        let mut wrap = base("PreToolUse");
        wrap["tool_name"] = json!("mcp__ccd_turn__wrap_up");
        wrap["tool_input"] = json!({"recap": "r", "ask": "What next?", "actions": [
            {"label": "Run the polish pass"}, {"label": "Start Milestone 4"}, {"action": "archive"}]});
        store.apply(&wrap);
        store.apply(&base("Stop"));
        let s = &store.snapshot()[0];
        assert_eq!(s.status, Status::Attention);
        assert_eq!(s.message.as_deref(), Some("What next?"));
        assert_eq!(s.ask.as_ref().unwrap().choices, vec!["Run the polish pass", "Start Milestone 4"]);
        // An app action Claude can do from a reply becomes a choice, even when it's the only one.
        let only = ask_from_next_step(&json!({"ask": "Commit these changes?", "actions": [{"action": "commit"}]}), 0);
        assert_eq!(only.unwrap().choices, vec!["Commit the changes"]);
        assert!(store.feed().log.iter().all(|e| !e.target.contains("wrap")), "kept out of the log");
        assert!(store.answered("s1"));
        assert!(store.snapshot()[0].ask.is_none());
        // A box with no question asks nothing.
        assert!(ask_from_next_step(&json!({"recap": "r"}), 0).is_none());

        store.apply(&base("SessionEnd"));
        assert!(store.snapshot().is_empty());
    }

    #[test]
    fn follows_task_lists() {
        let mut plan = Vec::new();
        let items = json!({"items": [
            {"content": "Find the code", "status": "in_progress"},
            {"content": "Fix it", "status": "pending"},
            {"content": "Verify", "status": "pending"}]});
        assert!(apply_plan(&mut plan, "mcp__ccd_turn__update_plan", &items));
        apply_plan(&mut plan, "mcp__ccd_turn__update_plan", &json!({"advance": true}));
        let st: Vec<&str> = plan.iter().map(|s| s.status.as_str()).collect();
        assert_eq!(st, ["completed", "in_progress", "pending"]);
        apply_plan(&mut plan, "mcp__ccd_turn__update_plan", &json!({"done": [2], "start": 3}));
        assert_eq!(plan[2].status, "in_progress");
        apply_plan(&mut plan, "mcp__ccd_turn__wrap_up", &json!({"recap": "x"}));
        assert!(plan.iter().all(|s| s.status == "completed"));

        let todos = json!({"todos": [
            {"content": "Run tests", "activeForm": "Running tests", "status": "in_progress"},
            {"content": "Ship", "activeForm": "Shipping", "status": "pending"}]});
        apply_plan(&mut plan, "TodoWrite", &todos);
        assert_eq!((plan.len(), plan[0].content.as_str(), plan[1].content.as_str()), (2, "Running tests", "Ship"));

        let mut plan = Vec::new();
        apply_plan(&mut plan, "TaskCreate", &json!({"subject": "A", "description": "a"}));
        apply_plan(&mut plan, "TaskCreate", &json!({"subject": "B", "description": "b"}));
        apply_plan(&mut plan, "TaskUpdate", &json!({"taskId": "2", "status": "in_progress"}));
        assert_eq!(plan[1].status, "in_progress");
        assert!(!apply_plan(&mut plan, "Bash", &json!({})));
    }

    #[test]
    fn describes_commands_and_mcp() {
        let a = describe_tool("Bash", &json!({"command": "npm test\nmore"}));
        assert_eq!((a.kind.as_str(), a.detail.as_str()), ("run", "npm test"));
        let a = describe_tool("mcp__github__create_issue", &json!({}));
        assert_eq!(a.detail, "create issue");
    }
}
