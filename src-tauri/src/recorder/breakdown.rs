//! Where a session's time went. Pure function over the session's events, so
//! every feature computes the same numbers. Rules: event-store.md section 4.

use std::collections::BTreeMap;

use serde::Serialize;

/// Idle gaps longer than this count as "away", not as part of the session.
pub const IDLE_CAP_MS: i64 = 30 * 60 * 1000;

#[derive(Serialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub total: i64,
    pub thinking: i64,
    /// (tool name, ms), largest first.
    pub tools: Vec<(String, i64)>,
    pub waiting_for_you: i64,
    pub idle: i64,
}

/// The fields of one event the breakdown needs.
pub struct Ev<'a> {
    pub ts: i64,
    pub kind: &'a str,
    pub tool: Option<&'a str>,
    pub tool_use_id: Option<&'a str>,
    /// Set for events from inside a subagent: they don't change the main
    /// agent's phase (that time stays with the main agent's Agent/Task tool).
    pub agent_id: Option<&'a str>,
    /// Notification type, for telling "waiting for you" from "idle".
    pub notification_type: Option<&'a str>,
}

#[derive(Clone, PartialEq, Debug)]
enum Phase {
    Off,
    Thinking,
    Tool(String),
    Waiting,
    Idle,
}

pub fn compute(events: &[Ev]) -> Breakdown {
    let mut thinking = 0i64;
    let mut waiting = 0i64;
    let mut idle = 0i64;
    let mut tools: BTreeMap<String, i64> = BTreeMap::new();
    // Open tool calls of the main agent: tool_use_id -> tool name (parallel calls).
    let mut open: Vec<(String, String)> = Vec::new();
    let mut phase = Phase::Off;
    let mut last: Option<i64> = None;

    for e in events {
        if let Some(prev) = last {
            let gap = (e.ts - prev).max(0);
            match &phase {
                Phase::Off => {}
                Phase::Thinking => thinking += gap,
                Phase::Tool(name) => *tools.entry(name.clone()).or_default() += gap,
                Phase::Waiting => waiting += gap,
                Phase::Idle => idle += gap.min(IDLE_CAP_MS),
            }
        }
        last = Some(e.ts);
        if e.agent_id.is_some() {
            continue;
        }
        let tool = e.tool.unwrap_or("tool").to_string();
        let after_tool = |open: &Vec<(String, String)>| match open.last() {
            Some((_, name)) => Phase::Tool(name.clone()),
            None => Phase::Thinking,
        };
        phase = match e.kind {
            "SessionStart" => Phase::Idle,
            "SessionEnd" => Phase::Off,
            "UserPromptSubmit" => {
                open.clear();
                Phase::Thinking
            }
            "PreToolUse" => {
                open.push((e.tool_use_id.unwrap_or("").to_string(), tool.clone()));
                Phase::Tool(tool)
            }
            "PostToolUse" | "PostToolUseFailure" => {
                let id = e.tool_use_id.unwrap_or("");
                if let Some(i) = open.iter().rposition(|(t, n)| (!id.is_empty() && t == id) || (id.is_empty() && *n == tool)) {
                    open.remove(i);
                }
                after_tool(&open)
            }
            "PermissionRequest" => Phase::Waiting,
            "Notification" if e.notification_type == Some("idle_prompt") => Phase::Idle,
            "Notification" => Phase::Waiting,
            "Stop" | "StopFailure" => {
                open.clear();
                Phase::Idle
            }
            _ => phase,
        };
    }

    let mut tools: Vec<(String, i64)> = tools.into_iter().filter(|(_, ms)| *ms > 0).collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total = thinking + waiting + idle + tools.iter().map(|t| t.1).sum::<i64>();
    Breakdown { total, thinking, tools, waiting_for_you: waiting, idle }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev<'a>(ts: i64, kind: &'a str, tool: Option<&'a str>, id: Option<&'a str>) -> Ev<'a> {
        Ev { ts, kind, tool, tool_use_id: id, agent_id: None, notification_type: None }
    }

    #[test]
    fn splits_a_simple_turn() {
        let evs = vec![
            ev(0, "SessionStart", None, None),
            ev(1_000, "UserPromptSubmit", None, None),
            ev(3_000, "PreToolUse", Some("Bash"), Some("a")),
            ev(4_000, "PermissionRequest", Some("Bash"), Some("a")),
            ev(9_000, "PostToolUse", Some("Bash"), Some("a")),
            ev(12_000, "PreToolUse", Some("Read"), Some("b")),
            ev(12_500, "PostToolUse", Some("Read"), Some("b")),
            ev(14_000, "Stop", None, None),
            ev(20_000, "UserPromptSubmit", None, None),
        ];
        let b = compute(&evs);
        assert_eq!(b.idle, 1_000 + 6_000);
        assert_eq!(b.thinking, 2_000 + 3_000 + 1_500);
        assert_eq!(b.waiting_for_you, 5_000);
        assert_eq!(b.tools, vec![("Bash".into(), 1_000), ("Read".into(), 500)]);
        assert_eq!(b.total, 20_000);
    }

    #[test]
    fn caps_long_idle_and_ignores_subagent_phases() {
        let mut sub = ev(2_000, "PreToolUse", Some("Grep"), Some("s"));
        sub.agent_id = Some("agent-1");
        let evs = vec![
            ev(0, "UserPromptSubmit", None, None),
            ev(1_000, "PreToolUse", Some("Agent"), Some("x")),
            sub,
            ev(5_000, "PostToolUse", Some("Agent"), Some("x")),
            ev(6_000, "Stop", None, None),
            ev(6_000 + 3 * IDLE_CAP_MS, "UserPromptSubmit", None, None),
        ];
        let b = compute(&evs);
        assert_eq!(b.tools, vec![("Agent".into(), 4_000)]);
        assert_eq!(b.thinking, 2_000);
        assert_eq!(b.idle, IDLE_CAP_MS);
    }
}
