//! Pending permission requests waiting for an Allow / Deny click in the island.
//!
//! A `PermissionRequest` hook call (see https://code.claude.com/docs/en/hooks,
//! "PermissionRequest decision control") is held open by the listener until:
//! - the user clicks Allow / Deny / Terminal in the island, or
//! - `WAIT` elapses (then we pass, and Claude Code continues its normal flow), or
//! - the bridge disconnects (Claude Code cancelled the hook), or
//! - the session moves on (Stop / new prompt / SessionEnd), meaning it was
//!   answered somewhere else.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::sessions::{describe_tool, now_ms};

/// How long the island holds a request before handing it back to Claude Code.
pub const WAIT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Allow,
    /// Deny, optionally with a typed reply that Claude reads as the reason.
    Deny(Option<String>),
    /// No decision: Claude Code continues with its own prompt.
    Pass,
    /// AskUserQuestion answered in the island: { "<question text>": "<chosen label(s)>" }.
    Answer(Value),
}

impl Choice {
    pub fn parse(s: &str) -> Option<Choice> {
        match s {
            "allow" => Some(Choice::Allow),
            "deny" => Some(Choice::Deny(None)),
            "pass" => Some(Choice::Pass),
            _ => None,
        }
    }

    /// The JSON the bridge prints to stdout for Claude Code. `input` is the
    /// request's tool input (an answer goes back as that input plus `answers`).
    pub fn to_hook_output(&self, input: &Value) -> Value {
        match self {
            Choice::Answer(answers) => {
                let mut updated = if input.is_object() { input.clone() } else { json!({}) };
                updated["answers"] = answers.clone();
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PermissionRequest",
                        "decision": { "behavior": "allow", "updatedInput": updated }
                    }
                })
            }
            Choice::Allow => json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": "allow" }
                }
            }),
            Choice::Deny(reply) => {
                let message = match reply.as_deref().map(str::trim) {
                    Some(r) if !r.is_empty() => format!("The user said no from the Atoll island and replied: {r}"),
                    _ => "The user denied this action from the Atoll island.".to_string(),
                };
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PermissionRequest",
                        "decision": { "behavior": "deny", "message": message }
                    }
                })
            }
            Choice::Pass => json!({}),
        }
    }
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionView {
    pub id: u64,
    pub session_id: String,
    pub project: String,
    pub tool: String,
    /// "Running", "Editing", ...
    pub label: String,
    /// Short target: file name or first line of the command.
    pub detail: String,
    /// Longer multi-line preview (full command, edit snippet, ...).
    pub preview: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// AskUserQuestion only: Claude's questions with their options, as sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questions: Option<Value>,
}

struct Pending {
    view: PermissionView,
    tx: oneshot::Sender<Choice>,
}

#[derive(Default)]
pub struct PermissionQueue {
    items: Vec<Pending>,
    next_id: u64,
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn clip_lines(text: &str, max_lines: usize, max_chars: usize) -> String {
    let mut out: String = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>() + "…";
    } else if text.lines().count() > max_lines {
        out.push_str("\n…");
    }
    out
}

/// Multi-line preview of what the tool is about to do.
fn preview(tool: &str, input: &Value) -> String {
    match tool {
        "Bash" | "PowerShell" => clip_lines(s(input, "command"), 8, 600),
        "Edit" => {
            let old = clip_lines(s(input, "old_string"), 4, 240);
            let new = clip_lines(s(input, "new_string"), 4, 240);
            let minus: Vec<String> = old.lines().map(|l| format!("- {l}")).collect();
            let plus: Vec<String> = new.lines().map(|l| format!("+ {l}")).collect();
            format!("{}\n{}\n{}", s(input, "file_path"), minus.join("\n"), plus.join("\n"))
        }
        "Write" => format!("{}\n{}", s(input, "file_path"), clip_lines(s(input, "content"), 6, 400)),
        "WebFetch" => s(input, "url").to_string(),
        _ => clip_lines(&serde_json::to_string_pretty(input).unwrap_or_default(), 10, 500),
    }
}

/// Claude Code's multiple-choice question tool.
pub fn tool_is_question(tool: &str) -> bool {
    tool == "AskUserQuestion"
}

impl PermissionQueue {
    /// Queue a request; returns its id and the receiver the listener awaits.
    pub fn push(&mut self, event: &Value, project: String) -> (u64, oneshot::Receiver<Choice>) {
        self.next_id += 1;
        let id = self.next_id;
        let tool = s(event, "tool_name").to_string();
        let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
        let activity = describe_tool(&tool, &input);
        let now = now_ms();
        let view = PermissionView {
            id,
            session_id: s(event, "session_id").to_string(),
            project,
            label: activity.label,
            detail: activity.detail,
            preview: preview(&tool, &input),
            tool,
            created_at: now,
            expires_at: now + WAIT.as_millis() as u64,
            questions: if tool_is_question(s(event, "tool_name")) { input.get("questions").cloned() } else { None },
        };
        let (tx, rx) = oneshot::channel();
        self.items.push(Pending { view, tx });
        (id, rx)
    }

    /// Answer one request. Returns false if it was already gone.
    pub fn resolve(&mut self, id: u64, choice: Choice) -> bool {
        match self.items.iter().position(|p| p.view.id == id) {
            Some(i) => {
                let pending = self.items.remove(i);
                let _ = pending.tx.send(choice);
                true
            }
            None => false,
        }
    }

    /// Drop a request without answering (its hook call ended). Returns true if removed.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|p| p.view.id != id);
        self.items.len() != before
    }

    /// Pass every request of a session (it was answered elsewhere). Returns true if any.
    pub fn pass_session(&mut self, session_id: &str) -> bool {
        let (gone, keep): (Vec<_>, Vec<_>) = self.items.drain(..).partition(|p| p.view.session_id == session_id);
        self.items = keep;
        let any = !gone.is_empty();
        for p in gone {
            let _ = p.tx.send(Choice::Pass);
        }
        any
    }

    pub fn views(&self) -> Vec<PermissionView> {
        self.items.iter().map(|p| p.view.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(session: &str) -> Value {
        json!({"session_id": session, "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": {"command": "npm test"}})
    }

    #[test]
    fn resolve_sends_choice() {
        let mut q = PermissionQueue::default();
        let (id, mut rx) = q.push(&req("a"), "demo".into());
        assert_eq!(q.views()[0].detail, "npm test");
        assert!(q.resolve(id, Choice::Deny(None)));
        assert_eq!(rx.try_recv().unwrap(), Choice::Deny(None));
        assert!(!q.resolve(id, Choice::Allow));
    }

    #[test]
    fn pass_session_only_touches_that_session() {
        let mut q = PermissionQueue::default();
        let (_, mut a) = q.push(&req("a"), "a".into());
        let (_, _b) = q.push(&req("b"), "b".into());
        assert!(q.pass_session("a"));
        assert_eq!(a.try_recv().unwrap(), Choice::Pass);
        assert_eq!(q.views().len(), 1);
    }

    #[test]
    fn hook_output_shapes() {
        let none = json!({});
        let allow = Choice::Allow.to_hook_output(&none);
        assert_eq!(allow["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(Choice::Deny(None).to_hook_output(&none)["hookSpecificOutput"]["decision"]["behavior"], "deny");
        let reply = Choice::Deny(Some("use pnpm instead".into())).to_hook_output(&none);
        assert!(reply["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().ends_with("use pnpm instead"));
        assert_eq!(Choice::Pass.to_hook_output(&none), json!({}));
        // An answered question goes back as the original input plus the answers.
        let input = json!({"questions": [{"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}]});
        let out = Choice::Answer(json!({"Which?": "B"})).to_hook_output(&input);
        let d = &out["hookSpecificOutput"]["decision"];
        assert_eq!(d["behavior"], "allow");
        assert_eq!(d["updatedInput"]["answers"]["Which?"], "B");
        assert_eq!(d["updatedInput"]["questions"][0]["question"], "Which?");
    }
}
