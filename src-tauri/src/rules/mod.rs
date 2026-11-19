//! Permission rules (see `logic.rs` for what a rule is and the safety rules).
//!
//! Files, in `%LOCALAPPDATA%\Crest\permissions\`:
//! - `rules.json`: your rules. If it can't be read, it is set aside as
//!   `rules.json.broken-<time>` and no rule acts until you save again (fail safe).
//! - `answers.jsonl`: one line per answered request: when, project folder, tool,
//!   command kinds ("npm test", "edit src/lib"), risk, allow / deny, and whether you or
//!   a rule answered. Never the full command or file contents. Decisions made by a rule
//!   also keep the one-sentence summary, so the log can say what was allowed.
//!   Kept to the newest 10,000 lines.

pub mod commands;
pub mod logic;
pub mod recorder;

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

pub use logic::{Answer, Decision, DryRun, Facts, Hint, Rule};

use crate::permissions::Choice;
use crate::sessions::now_ms;

const MAX_ANSWERS: usize = 10_000;

struct Book {
    rules: Vec<Rule>,
    /// Set when rules.json could not be read; shown on the Settings page.
    error: Option<String>,
}

static BOOK: Mutex<Option<Book>> = Mutex::new(None);
/// Tests point the files at a temporary folder.
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn dir() -> PathBuf {
    DIR.lock().unwrap().clone().unwrap_or_else(|| crate::paths::local_dir().join("permissions"))
}

fn rules_file() -> PathBuf {
    dir().join("rules.json")
}

fn answers_file() -> PathBuf {
    dir().join("answers.jsonl")
}

fn load() -> Book {
    let path = rules_file();
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Book { rules: vec![], error: None },
        Err(e) => Book { rules: vec![], error: Some(format!("Atoll could not read your rules ({e}), so no rule is acting.")) },
        Ok(text) => match serde_json::from_str::<Vec<Rule>>(&text) {
            Ok(rules) => Book { rules, error: None },
            Err(e) => {
                // Keep the damaged file for the person, and act on no rule (never a guess).
                let aside = path.with_file_name(format!("rules.json.broken-{}", now_ms()));
                let _ = std::fs::rename(&path, &aside);
                Book { rules: vec![], error: Some(format!("Your rules file was damaged ({e}). It was set aside as {}, and no rule is acting.", aside.display())) }
            }
        },
    }
}

fn with_book<T>(f: impl FnOnce(&mut Book) -> T) -> T {
    let mut guard = BOOK.lock().unwrap();
    let book = guard.get_or_insert_with(load);
    f(book)
}

fn write_rules(rules: &[Rule]) -> Result<(), String> {
    let path = rules_file();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| format!("could not save your rules: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not save your rules: {e}"))
}

pub fn list() -> (Vec<Rule>, Option<String>) {
    with_book(|b| (b.rules.clone(), b.error.clone()))
}

/// Add or replace a rule after checking it. Returns the stored rule.
pub fn save(mut rule: Rule) -> Result<Rule, String> {
    let now = now_ms();
    logic::validate(&rule, now)?;
    rule.name = rule.name.trim().chars().take(60).collect();
    rule.pattern = rule.pattern.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    rule.project = rule.project.as_deref().map(logic::norm).filter(|p| !p.is_empty());
    if !rule.auto_allow {
        rule.expires_at = None;
    }
    rule.updated_at = now;
    with_book(|b| {
        if rule.id.is_empty() {
            rule.id = new_id();
            rule.created_at = now;
            b.rules.push(rule.clone());
        } else if let Some(old) = b.rules.iter_mut().find(|r| r.id == rule.id) {
            // Counters belong to the store, not the form.
            rule.hits = old.hits;
            rule.last_hit = old.last_hit;
            rule.created_at = old.created_at;
            *old = rule.clone();
        } else {
            b.rules.push(rule.clone());
        }
        write_rules(&b.rules)?;
        b.error = None;
        Ok(rule)
    })
}

pub fn delete(id: &str) -> Result<(), String> {
    with_book(|b| {
        b.rules.retain(|r| r.id != id);
        write_rules(&b.rules)
    })
}

/// The one-click switch: the rule stops acting at once (it stays in the list).
pub fn disable(id: &str) -> Result<(), String> {
    with_book(|b| {
        let r = b.rules.iter_mut().find(|r| r.id == id).ok_or("That rule no longer exists.")?;
        r.enabled = false;
        r.auto_allow = false;
        r.expires_at = None;
        r.updated_at = now_ms();
        write_rules(&b.rules)
    })
}

fn new_id() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("OS random number generator unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------- the request flow

/// What the rules do with a request. When a rule auto-allows it, the decision is
/// logged and the rule's hit count goes up before this returns.
pub fn check(event: &Value, analysis: &crest_intel::Analysis, project_dir: &str) -> (Facts, Decision) {
    let facts = logic::facts(event, analysis, project_dir);
    let now = now_ms();
    let decision = with_book(|b| {
        if b.error.is_some() {
            return Decision::default();
        }
        let d = logic::decide(&b.rules, &facts, &analysis.summary, now);
        if let Some(a) = &d.auto {
            if let Some(r) = b.rules.iter_mut().find(|r| r.id == a.rule_id) {
                r.hits += 1;
                r.last_hit = Some(now);
            }
            if let Err(e) = write_rules(&b.rules) {
                eprintln!("[crest] rules: {e}");
            }
        }
        d
    });
    if let Some(a) = &decision.auto {
        append(&Answer {
            at: now,
            project: facts.project.clone(),
            tool: facts.tool.clone(),
            keys: facts.keys.clone(),
            risk: facts.risk,
            choice: "allow".into(),
            by: "rule".into(),
            rule_id: Some(a.rule_id.clone()),
            rule_name: Some(a.rule_name.clone()),
            summary: Some(a.summary.clone()),
        });
    }
    (facts, decision)
}

/// Remember how you answered a request (kinds only), for later dry runs.
pub fn record_answer(facts: &Facts, choice: &Choice) {
    let choice = match choice {
        Choice::Allow => "allow",
        Choice::Deny(_) => "deny",
        // Handing back to Claude Code, or a multiple-choice answer, says nothing about the request.
        Choice::Pass | Choice::Answer(_) => return,
    };
    if facts.question || facts.keys.is_empty() {
        return;
    }
    append(&Answer {
        at: now_ms(),
        project: facts.project.clone(),
        tool: facts.tool.clone(),
        keys: facts.keys.clone(),
        risk: facts.risk,
        choice: choice.into(),
        by: "you".into(),
        rule_id: None,
        rule_name: None,
        summary: None,
    });
}

fn append(a: &Answer) {
    let path = answers_file();
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{}", serde_json::to_string(a).unwrap_or_default())
    })();
    if let Err(e) = result {
        eprintln!("[crest] rules: could not write the answers log: {e}");
    }
}

/// Every answer, oldest first. Trims the file to the newest `MAX_ANSWERS` lines.
pub fn answers() -> Vec<Answer> {
    let path = answers_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![] };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() > MAX_ANSWERS + MAX_ANSWERS / 5 {
        let keep = lines[lines.len() - MAX_ANSWERS..].join("\n") + "\n";
        let _ = std::fs::write(&path, keep);
    }
    let start = lines.len().saturating_sub(MAX_ANSWERS);
    lines[start..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Decisions made by rules, newest first.
pub fn decision_log(limit: usize) -> Vec<Answer> {
    let mut log: Vec<Answer> = answers().into_iter().filter(|a| a.by == "rule").collect();
    log.reverse();
    log.truncate(limit);
    log
}

/// Replay a rule (saved or not) over everything recorded.
pub fn dry_run(rule: &Rule) -> DryRun {
    logic::dry_run(rule, &answers(), &recorder::past_requests(), now_ms())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// One test owns the global files, so the steps run in order.
    #[test]
    fn store_flow_end_to_end() {
        let tmp = std::env::temp_dir().join(format!("atoll-rules-{}", now_ms()));
        *DIR.lock().unwrap() = Some(tmp.clone());
        *BOOK.lock().unwrap() = None;

        let ctx = crest_intel::Context { project: "C:/p".into(), cwd: "C:/p".into(), home: "C:/h".into(), ..Default::default() };
        let event = |cmd: &str| json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": "C:/p"});
        let analyze = |e: &Value| crest_intel::analyze_tool("Bash", &e["tool_input"], &ctx, None);

        // You allow `npm test` twice and deny `npm publish` once.
        for (cmd, choice) in [("npm test", Choice::Allow), ("npm test", Choice::Allow), ("npm publish", Choice::Deny(None))] {
            let e = event(cmd);
            let (f, d) = check(&e, &analyze(&e), "C:/p");
            assert_eq!(d, Decision::default());
            record_answer(&f, &choice);
        }
        assert_eq!(answers().len(), 3);

        // Make a rule from a request, dry-run it, turn on auto-allow, save.
        let e = event("npm test");
        let (f, _) = check(&e, &analyze(&e), "C:/p");
        let mut rule = logic::draft(&f, "p", now_ms());
        let d = dry_run(&rule);
        assert_eq!((d.matched, d.allowed, d.denied), (2, 2, 0));
        rule.dry_run = Some(d);
        rule.auto_allow = true;
        rule.expires_at = Some(now_ms() + 7 * logic::DAY_MS);
        let rule = save(rule).unwrap();

        // Now `npm test` is allowed by the rule and logged; `npm test --watch` still asks.
        let (_, d) = check(&e, &analyze(&e), "C:/p");
        assert_eq!(d.auto.as_ref().unwrap().rule_id, rule.id);
        let w = event("npm test --watch");
        let (_, d) = check(&w, &analyze(&w), "C:/p");
        assert!(d.auto.is_none() && d.hint.is_some());
        let log = decision_log(10);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].rule_name.as_deref(), Some(rule.name.as_str()));
        assert_eq!(list().0[0].hits, 1);

        // One click off: nothing more is allowed.
        disable(&rule.id).unwrap();
        let (_, d) = check(&e, &analyze(&e), "C:/p");
        assert_eq!(d, Decision::default());

        // Rules survive a restart; a damaged file means no rule acts and is set aside.
        *BOOK.lock().unwrap() = None;
        assert_eq!(list().0.len(), 1);
        std::fs::write(rules_file(), "{ not json").unwrap();
        *BOOK.lock().unwrap() = None;
        let (rules, error) = list();
        assert!(rules.is_empty() && error.unwrap().contains("damaged"));
        assert!(std::fs::read_dir(&tmp).unwrap().any(|f| f.unwrap().file_name().to_string_lossy().starts_with("rules.json.broken-")));

        // The answers log never holds the command text.
        let text = std::fs::read_to_string(answers_file()).unwrap();
        assert!(!text.contains("--watch\"") && text.contains("npm test"));

        *DIR.lock().unwrap() = None;
        *BOOK.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
