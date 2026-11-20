//! Permission rules: the pure part (no files, no app). What a rule is, what a
//! request looks like to a rule, matching, the dry run, and the auto-allow decision.
//!
//! Safety rules, all enforced here and tested below:
//! - A rule only ever *allows*; it never denies and never touches Claude Code's settings.
//! - Auto-allow needs the rule's own opt-in, an expiry no more than 30 days ahead,
//!   and a dry run of the rule exactly as it is now.
//! - A request flagged risky or "couldn't check", or not eligible for auto-allow
//!   (PowerShell, code written inside the command, ...), is never auto-allowed.
//! - Every command kind in the request must be one the dry run saw you allow, so a
//!   rule never widens beyond what its dry run showed.
//! - Claude's multiple-choice questions are never handled by rules.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crest_intel::{Analysis, Risk};

pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// Longest an auto-allow may run before it must be renewed.
pub const MAX_AUTO_DAYS: u64 = 30;
/// Examples kept in a dry run, newest first.
const EXAMPLES: usize = 6;

/// Programs that only set the stage. They never need to be in a rule's pattern.
const NEUTRAL: &[&str] = &["cd", "pushd", "popd", "echo", "printf", "true", ":", "sleep", "test", "[", "[["];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    /// Off = the rule does nothing at all.
    pub enabled: bool,
    /// The session's starting folder (normalised), or None for every project.
    pub project: Option<String>,
    /// "Bash", "files" (Edit, MultiEdit, Write, NotebookEdit), "WebFetch", or an exact tool name.
    pub tool: String,
    /// Bash: "program" or "program sub" ("npm test", "tail").
    /// files: folder globs relative to the project ("src/**", "docs").
    /// WebFetch: hosts ("docs.rs", "*.github.com"). Other tools: empty.
    pub pattern: Vec<String>,
    /// Allow matching requests without asking. Needs `expires_at` and a current dry run.
    pub auto_allow: bool,
    pub expires_at: Option<u64>,
    pub dry_run: Option<DryRun>,
    pub hits: u64,
    pub last_hit: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DryRun {
    pub at: u64,
    /// The rule content (tool, project, pattern) the dry run was made for.
    pub fingerprint: String,
    /// Past requests looked at.
    pub looked_at: usize,
    pub matched: usize,
    pub allowed: usize,
    pub denied: usize,
    /// Matched, but how they were answered is not known (from the recorder).
    pub unanswered: usize,
    /// Matched but flagged risky or "couldn't check": these always ask.
    pub risky: usize,
    /// Command kinds you allowed: the only kinds the rule may auto-allow.
    pub shapes: Vec<String>,
    pub examples: Vec<Example>,
    /// Where the history came from, in plain words.
    pub sources: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub at: u64,
    pub key: String,
    /// "allow", "deny" or "" (unknown).
    pub choice: String,
    pub risk: Risk,
}

/// One answered request, as kept in the answers log: kinds only, never the full command.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub at: u64,
    pub project: String,
    pub tool: String,
    pub keys: Vec<String>,
    pub risk: Risk,
    /// "allow" or "deny".
    pub choice: String,
    /// "you" or "rule".
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    /// Kept for decisions made by a rule only, so the log can say what was allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A past request from the recorder (no answer known).
#[derive(Clone, Debug, PartialEq)]
pub struct Past {
    pub at: u64,
    pub project: String,
    pub tool: String,
    pub keys: Vec<String>,
    pub risk: Risk,
}

/// A request as rules see it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub project: String,
    pub tool: String,
    pub keys: Vec<String>,
    pub risk: Risk,
    pub eligible: bool,
    pub ineligible_reason: String,
    pub question: bool,
}

/// Shown in the peek when a rule matches but did not decide.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    pub rule_id: String,
    pub rule_name: String,
    /// Why it still asks you, in plain words.
    pub why_not: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoAllow {
    pub rule_id: String,
    pub rule_name: String,
    pub summary: String,
    pub at: u64,
}

#[derive(Debug, Default, PartialEq)]
pub struct Decision {
    pub auto: Option<AutoAllow>,
    pub hint: Option<Hint>,
}

// ---------------------------------------------------------------- facts

/// Lower-case, forward slashes, "/c/x" -> "c:/x", no trailing slash.
pub fn norm(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/").to_lowercase();
    let b = p.as_bytes();
    if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
        p = format!("{}:{}", b[1] as char, if b.len() == 2 { "/" } else { &p[2..] });
    }
    while p.len() > 3 && p.ends_with('/') {
        p.pop();
    }
    p
}

pub fn tool_family(tool: &str) -> String {
    match tool {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "files".into(),
        t => t.to_string(),
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("").to_lowercase()
}

/// The folder of a file relative to the project ("src/lib", "." for the project root),
/// or None when the file is outside the project.
fn project_folder(file: &str, project: &str) -> Option<String> {
    let f = norm(file);
    let dir = f.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    if project.is_empty() {
        return None;
    }
    if dir == project {
        return Some(".".into());
    }
    dir.strip_prefix(&format!("{project}/")).map(String::from)
}

/// The command kinds of a request: "npm test", "git commit -m", "edit src/lib", "fetch docs.rs".
pub fn keys(tool: &str, input: &Value, analysis: &Analysis, project: &str) -> Vec<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let mut out: Vec<String> = Vec::new();
    let mut push = |k: String| {
        if !out.contains(&k) {
            out.push(k);
        }
    };
    match tool_family(tool).as_str() {
        "Bash" | "PowerShell" => {
            let all: Vec<_> = analysis.commands.iter().collect();
            let kept: Vec<_> = all.iter().filter(|c| !NEUTRAL.contains(&c.program.as_str())).collect();
            for c in if kept.is_empty() { all.iter().collect::<Vec<_>>() } else { kept } {
                let mut k = c.program.clone();
                if !c.sub.is_empty() {
                    k.push(' ');
                    k.push_str(&c.sub);
                }
                for o in &c.options {
                    k.push(' ');
                    k.push_str(o);
                }
                push(k);
            }
        }
        "files" => {
            let file = [s("file_path"), s("notebook_path"), s("path")].into_iter().find(|p| !p.is_empty()).unwrap_or("");
            match project_folder(file, project) {
                Some(folder) => push(format!("edit {folder}")),
                // Outside the project: a key no folder pattern can match.
                None => push(format!("edit outside {}", norm(file))),
            }
        }
        "WebFetch" => push(format!("fetch {}", host_of(s("url")))),
        t => push(t.to_string()),
    }
    out
}

pub fn facts(event: &Value, analysis: &Analysis, project_dir: &str) -> Facts {
    let tool = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
    let project = norm(project_dir);
    Facts {
        keys: keys(tool, &input, analysis, &project),
        project,
        tool: tool_family(tool),
        risk: analysis.risk,
        eligible: analysis.auto_allow_eligible,
        ineligible_reason: analysis.ineligible_reason.clone(),
        question: tool == "AskUserQuestion",
    }
}

// ---------------------------------------------------------------- matching

/// Folder glob: "**" any depth, "*" one folder, otherwise literal. "src/**" also matches "src".
fn glob(pattern: &str, folder: &str) -> bool {
    fn rec(p: &[&str], f: &[&str]) -> bool {
        match (p.first(), f.first()) {
            (None, None) => true,
            (Some(&"**"), _) => rec(&p[1..], f) || (!f.is_empty() && rec(p, &f[1..])),
            (Some(a), Some(b)) => (*a == "*" || a == b) && rec(&p[1..], &f[1..]),
            _ => false,
        }
    }
    let p = pattern.trim().trim_matches('/').to_lowercase();
    let p: Vec<&str> = if p.is_empty() || p == "." { vec![] } else { p.split('/').collect() };
    let f: Vec<&str> = if folder == "." { vec![] } else { folder.split('/').collect() };
    rec(&p, &f)
}

/// Does one key fit one pattern entry?
fn key_fits(tool: &str, entry: &str, key: &str) -> bool {
    let entry = entry.trim().to_lowercase();
    if entry.is_empty() {
        return false;
    }
    match tool {
        "Bash" | "PowerShell" => {
            let mut k = key.split(' ');
            let (prog, sub) = (k.next().unwrap_or(""), k.next().filter(|w| !w.starts_with('-')).unwrap_or(""));
            let mut e = entry.split_whitespace();
            let (eprog, esub) = (e.next().unwrap_or(""), e.next().unwrap_or(""));
            prog == eprog && (esub.is_empty() || sub == esub)
        }
        "files" => key.strip_prefix("edit ").is_some_and(|folder| !folder.starts_with("outside ") && glob(&entry, folder)),
        "WebFetch" => key.strip_prefix("fetch ").is_some_and(|host| host == entry || entry.strip_prefix("*.").is_some_and(|d| host == d || host.ends_with(&format!(".{d}")))),
        _ => true,
    }
}

/// The rule's pattern and scope cover the request (on or off, auto or not).
pub fn covers(rule: &Rule, tool: &str, project: &str, keys: &[String]) -> bool {
    if rule.tool != tool || keys.is_empty() {
        return false;
    }
    if rule.project.as_deref().is_some_and(|p| norm(p) != project) {
        return false;
    }
    let pattern_needed = matches!(tool, "Bash" | "PowerShell" | "files" | "WebFetch");
    if pattern_needed && rule.pattern.is_empty() {
        return false;
    }
    keys.iter().all(|k| !pattern_needed || rule.pattern.iter().any(|e| key_fits(tool, e, k)))
}

/// Same tool, project and pattern = same fingerprint. Editing any of them voids the dry run.
pub fn fingerprint(rule: &Rule) -> String {
    let mut pattern: Vec<String> = rule.pattern.iter().map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
    pattern.sort();
    pattern.dedup();
    let text = format!("{}|{}|{}", rule.tool, rule.project.as_deref().map(norm).unwrap_or_default(), pattern.join("\n"));
    // FNV-1a, 64 bit: stable across runs and versions.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn date(ms: u64) -> String {
    // Days since 1970 -> y-m-d (UTC), enough for a plain sentence.
    let days = (ms / DAY_MS) as i64;
    let (mut y, mut d) = (1970i64, days);
    loop {
        let len = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!("{y}-{:02}-{:02}", m + 1, d + 1)
}

/// Why this rule may not auto-allow this request (None = it may).
pub fn why_not_auto(rule: &Rule, f: &Facts, now: u64) -> Option<String> {
    if !rule.auto_allow {
        return Some("auto-allow is off for this rule".into());
    }
    match rule.expires_at {
        None => return Some("the rule has no expiry date, so it cannot auto-allow".into()),
        Some(t) if t <= now => return Some(format!("its auto-allow ended on {}", date(t))),
        Some(t) if t > now + MAX_AUTO_DAYS * DAY_MS + DAY_MS => return Some("its expiry is more than 30 days away".into()),
        _ => {}
    }
    let Some(dry) = rule.dry_run.as_ref().filter(|d| d.fingerprint == fingerprint(rule)) else {
        return Some("the rule changed since its dry run".into());
    };
    if f.question {
        return Some("Claude's questions always ask you".into());
    }
    if f.risk >= Risk::Unknown {
        return Some(if f.risk == Risk::Risky { "this request is risky".into() } else { "Atoll couldn't check this request".into() });
    }
    if !f.eligible {
        let r = if f.ineligible_reason.is_empty() { "Atoll can't fully check this kind of request".to_string() } else { f.ineligible_reason.clone() };
        return Some(r);
    }
    if let Some(new) = f.keys.iter().find(|k| !dry.shapes.contains(k)) {
        return Some(format!("its dry run never saw “{new}” allowed"));
    }
    None
}

/// What the rules do with a request: allow it, hint at a rule, or nothing.
pub fn decide(rules: &[Rule], f: &Facts, summary: &str, now: u64) -> Decision {
    if f.question {
        return Decision::default();
    }
    let mut hint = None;
    for r in rules.iter().filter(|r| r.enabled && covers(r, &f.tool, &f.project, &f.keys)) {
        match why_not_auto(r, f, now) {
            None => {
                return Decision {
                    auto: Some(AutoAllow { rule_id: r.id.clone(), rule_name: r.name.clone(), summary: summary.to_string(), at: now }),
                    hint: None,
                }
            }
            Some(why) if hint.is_none() => hint = Some(Hint { rule_id: r.id.clone(), rule_name: r.name.clone(), why_not: why }),
            _ => {}
        }
    }
    Decision { auto: None, hint }
}

// ---------------------------------------------------------------- dry run

/// Replay a rule over history. Answers come from Atoll's own log; past requests
/// from the recorder only fill the time before that log starts (no double counting).
pub fn dry_run(rule: &Rule, answers: &[Answer], past: &[Past], now: u64) -> DryRun {
    let mut d = DryRun { at: now, fingerprint: fingerprint(rule), ..Default::default() };
    let log_start = answers.iter().map(|a| a.at).min().unwrap_or(u64::MAX);
    let mut seen: Vec<(u64, String, String, Risk)> = Vec::new();
    for a in answers {
        d.looked_at += 1;
        if !covers(rule, &a.tool, &a.project, &a.keys) {
            continue;
        }
        d.matched += 1;
        let risky = a.risk >= Risk::Unknown;
        if risky {
            d.risky += 1;
        }
        match a.choice.as_str() {
            "allow" => {
                d.allowed += 1;
                // Only kinds *you* allowed, on requests that were not risky, can be auto-allowed.
                if a.by == "you" && !risky {
                    for k in &a.keys {
                        if !d.shapes.contains(k) {
                            d.shapes.push(k.clone());
                        }
                    }
                }
            }
            "deny" => d.denied += 1,
            _ => d.unanswered += 1,
        }
        seen.push((a.at, a.keys.join(" · "), a.choice.clone(), a.risk));
    }
    let mut from_recorder = 0;
    for p in past.iter().filter(|p| p.at < log_start) {
        d.looked_at += 1;
        from_recorder += 1;
        if !covers(rule, &p.tool, &p.project, &p.keys) {
            continue;
        }
        d.matched += 1;
        d.unanswered += 1;
        if p.risk >= Risk::Unknown {
            d.risky += 1;
        }
        seen.push((p.at, p.keys.join(" · "), String::new(), p.risk));
    }
    seen.sort_by(|a, b| b.0.cmp(&a.0));
    d.examples = seen.into_iter().take(EXAMPLES).map(|(at, key, choice, risk)| Example { at, key, choice, risk }).collect();
    d.shapes.sort();
    d.sources.push(format!("{} answered requests in Atoll", answers.len()));
    d.sources.push(if past.is_empty() { "the recorder is not connected yet".into() } else { format!("{from_recorder} older requests from the recorder") });
    d
}

/// A starting rule built from one request: its tool, its project, and a pattern
/// that covers exactly what it does.
pub fn draft(f: &Facts, project_name: &str, now: u64) -> Rule {
    let pattern: Vec<String> = match f.tool.as_str() {
        "Bash" | "PowerShell" => {
            let mut p: Vec<String> = Vec::new();
            for k in &f.keys {
                let mut w = k.split(' ');
                let prog = w.next().unwrap_or("").to_string();
                let sub = w.next().filter(|s| !s.starts_with('-'));
                let e = match sub {
                    Some(s) => format!("{prog} {s}"),
                    None => prog,
                };
                if !p.contains(&e) {
                    p.push(e);
                }
            }
            p
        }
        "files" => f
            .keys
            .iter()
            .filter_map(|k| k.strip_prefix("edit "))
            .filter(|folder| !folder.starts_with("outside "))
            .map(|folder| if folder == "." { "**".to_string() } else { format!("{folder}/**") })
            .collect(),
        "WebFetch" => f.keys.iter().filter_map(|k| k.strip_prefix("fetch ")).map(String::from).collect(),
        _ => vec![],
    };
    let what = match f.tool.as_str() {
        "Bash" | "PowerShell" => pattern.join(", "),
        "files" => format!("Edits in {}", pattern.join(", ")),
        "WebFetch" => format!("Pages from {}", pattern.join(", ")),
        t => t.trim_start_matches("mcp__").replace("__", " · ").replace('_', " "),
    };
    // Named by what it covers, not "Allow …": most rules only hint, and the status line says which.
    let mut name = what;
    if !project_name.is_empty() {
        name.push_str(&format!(" in {project_name}"));
    }
    if name.chars().count() > 60 {
        name = name.chars().take(59).collect::<String>() + "…";
    }
    Rule {
        name,
        created_at: now,
        updated_at: now,
        enabled: true,
        project: if f.project.is_empty() { None } else { Some(f.project.clone()) },
        tool: f.tool.clone(),
        pattern,
        ..Default::default()
    }
}

/// Check a rule before it is saved. Returns a message for the person when it is not acceptable.
pub fn validate(rule: &Rule, now: u64) -> Result<(), String> {
    if rule.name.trim().is_empty() {
        return Err("Give the rule a name.".into());
    }
    if rule.tool.trim().is_empty() {
        return Err("Choose which tool the rule is for.".into());
    }
    if matches!(rule.tool.as_str(), "Bash" | "PowerShell" | "files" | "WebFetch") && rule.pattern.iter().all(|p| p.trim().is_empty()) {
        return Err("Add at least one command, folder or address the rule covers.".into());
    }
    if rule.tool == "Bash" && rule.pattern.iter().any(|p| matches!(p.trim(), "bash" | "sh" | "sudo" | "env" | "eval" | "xargs" | "powershell" | "cmd")) {
        return Err("A rule can't cover shells or wrappers (bash, sh, sudo, env, eval, xargs…): they can run anything.".into());
    }
    if rule.auto_allow {
        let Some(exp) = rule.expires_at else { return Err("Auto-allow needs an end date.".into()) };
        if exp <= now {
            return Err("The end date for auto-allow has already passed.".into());
        }
        if exp > now + MAX_AUTO_DAYS * DAY_MS + DAY_MS {
            return Err("Auto-allow can run for 30 days at most.".into());
        }
        let Some(dry) = rule.dry_run.as_ref() else { return Err("Run the dry run before turning on auto-allow.".into()) };
        if dry.fingerprint != fingerprint(rule) {
            return Err("The rule changed since its dry run. Run it again before turning on auto-allow.".into());
        }
        if dry.shapes.is_empty() {
            return Err("The dry run found nothing you allowed, so there is nothing this rule could safely auto-allow yet.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn ctx() -> crest_intel::Context {
        crest_intel::Context { project: "C:/p".into(), cwd: "C:/p".into(), home: "C:/h".into(), temp: vec!["C:/t".into()], ..Default::default() }
    }

    fn bash(cmd: &str) -> Facts {
        let ev = json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": "C:/p"});
        let a = crest_intel::analyze_tool("Bash", &ev["tool_input"], &ctx(), None);
        facts(&ev, &a, "C:/p")
    }

    fn edit(path: &str) -> Facts {
        let ev = json!({"tool_name": "Edit", "tool_input": {"file_path": path, "old_string": "a", "new_string": "b"}, "cwd": "C:/p"});
        let a = crest_intel::analyze_tool("Edit", &ev["tool_input"], &ctx(), None);
        facts(&ev, &a, "C:/p")
    }

    fn answer(f: &Facts, choice: &str) -> Answer {
        Answer { at: NOW - 1000, project: f.project.clone(), tool: f.tool.clone(), keys: f.keys.clone(), risk: f.risk, choice: choice.into(), by: "you".into(), rule_id: None, rule_name: None, summary: None }
    }

    /// A rule with auto-allow on and a current dry run over `history`.
    fn armed(pattern: &[&str], history: &[Answer]) -> Rule {
        let mut r = Rule { id: "r1".into(), name: "Tests".into(), enabled: true, tool: "Bash".into(), project: Some("c:/p".into()), pattern: pattern.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        r.dry_run = Some(dry_run(&r, history, &[], NOW));
        r.auto_allow = true;
        r.expires_at = Some(NOW + 7 * DAY_MS);
        r
    }

    #[test]
    fn keys_describe_kinds_not_full_commands() {
        assert_eq!(bash("cd src && npm test 2>&1 | tail -15").keys, ["npm test", "tail"], "line counts are numbers, not options");
        assert_eq!(bash("git commit -m \"secret text\"").keys, ["git commit -m"]);
        assert_eq!(edit("C:/p/src/lib/a.ts").keys, ["edit src/lib"]);
        assert_eq!(edit("C:/p/a.ts").keys, ["edit ."]);
        assert_eq!(edit("C:/elsewhere/a.ts").keys, ["edit outside c:/elsewhere/a.ts"]);
    }

    #[test]
    fn auto_allows_only_what_the_dry_run_saw() {
        let seen = bash("npm test");
        let rule = armed(&["npm test"], &[answer(&seen, "allow")]);
        // Exactly what was allowed before: allowed.
        let d = decide(&[rule.clone()], &bash("npm test"), "Runs tests.", NOW);
        assert_eq!(d.auto.unwrap().rule_id, "r1");
        // Same program, new option: the dry run never saw it, so it asks (never widens).
        let d = decide(&[rule.clone()], &bash("npm test --watch"), "", NOW);
        assert!(d.auto.is_none());
        assert!(d.hint.unwrap().why_not.contains("never saw"));
        // Outside the pattern: no hint, no allow.
        assert_eq!(decide(&[rule], &bash("npm publish"), "", NOW), Decision::default());
    }

    #[test]
    fn risky_and_unchecked_are_never_auto_allowed() {
        // Even a rule whose pattern and dry run cover the programs involved.
        let history = [answer(&bash("rm a.txt"), "allow"), answer(&bash("git push"), "allow")];
        let rule = armed(&["rm", "git push"], &history);
        for cmd in ["rm -rf ~", "rm -rf /", "git push --force origin main", "echo 'oops && rm x"] {
            let f = bash(cmd);
            assert!(f.risk >= Risk::Unknown, "{cmd} should be risky or unknown");
            let d = decide(&[rule.clone()], &f, "", NOW);
            assert!(d.auto.is_none(), "{cmd} was auto-allowed");
        }
        // Not eligible (code written inside the command) also asks.
        let f = bash("python -c \"print(1)\"");
        let rule = armed(&["python"], &[answer(&f, "allow")]);
        assert!(decide(&[rule], &f, "", NOW).auto.is_none());
    }

    #[test]
    fn auto_allow_needs_opt_in_expiry_and_a_current_dry_run() {
        let seen = bash("npm test");
        let base = armed(&["npm test"], &[answer(&seen, "allow")]);
        let f = bash("npm test");

        let mut r = base.clone();
        r.auto_allow = false;
        assert!(decide(&[r], &f, "", NOW).hint.unwrap().why_not.contains("off"));

        let mut r = base.clone();
        r.expires_at = Some(NOW - 1);
        assert!(decide(&[r], &f, "", NOW).auto.is_none());

        let mut r = base.clone();
        r.expires_at = None;
        assert!(decide(&[r], &f, "", NOW).auto.is_none());

        // Editing the pattern voids the dry run.
        let mut r = base.clone();
        r.pattern.push("npm run".into());
        assert!(decide(&[r], &f, "", NOW).hint.unwrap().why_not.contains("changed since its dry run"));

        let mut r = base.clone();
        r.enabled = false;
        assert_eq!(decide(&[r], &f, "", NOW), Decision::default());

        // Another project.
        let mut r = base;
        r.project = Some("c:/other".into());
        assert_eq!(decide(&[r], &f, "", NOW), Decision::default());
    }

    #[test]
    fn dry_run_counts_answers_and_only_learns_from_your_allows() {
        let t = bash("npm test");
        let mut denied = answer(&bash("npm test --watch"), "deny");
        denied.at = NOW - 500;
        let mut by_rule = answer(&bash("npm test --ci"), "allow");
        by_rule.by = "rule".into();
        let unrelated = answer(&bash("cargo build"), "allow");
        let rule = Rule { tool: "Bash".into(), project: Some("c:/p".into()), pattern: vec!["npm test".into()], ..Default::default() };
        let d = dry_run(&rule, &[answer(&t, "allow"), denied, by_rule, unrelated], &[], NOW);
        assert_eq!((d.looked_at, d.matched, d.allowed, d.denied), (4, 3, 2, 1));
        assert_eq!(d.shapes, ["npm test"]);
        assert_eq!(d.examples[0].choice, "deny");

        // The recorder only fills the time before Atoll's own log: no double counting.
        let old = Past { at: NOW - 10_000, project: "c:/p".into(), tool: "Bash".into(), keys: vec!["npm test".into()], risk: Risk::Caution };
        let overlap = Past { at: NOW, ..old.clone() };
        let d = dry_run(&rule, &[answer(&t, "allow")], &[old, overlap], NOW);
        assert_eq!((d.matched, d.unanswered), (2, 1));
        assert_eq!(d.shapes, ["npm test"], "recorder requests have no known answer, so they teach nothing");
    }

    #[test]
    fn files_and_web_patterns() {
        let mut r = Rule { tool: "files".into(), project: Some("c:/p".into()), pattern: vec!["src/**".into()], ..Default::default() };
        assert!(covers(&r, "files", "c:/p", &edit("C:/p/src/a.ts").keys));
        assert!(covers(&r, "files", "c:/p", &edit("C:/p/src/lib/x/a.ts").keys));
        assert!(!covers(&r, "files", "c:/p", &edit("C:/p/docs/a.md").keys));
        assert!(!covers(&r, "files", "c:/p", &edit("C:/elsewhere/a.ts").keys));
        r.pattern = vec!["**".into()];
        assert!(!covers(&r, "files", "c:/p", &edit("C:/elsewhere/a.ts").keys), "outside the project never matches");
        let w = Rule { tool: "WebFetch".into(), pattern: vec!["*.rust-lang.org".into()], ..Default::default() };
        assert!(covers(&w, "WebFetch", "c:/p", &["fetch doc.rust-lang.org".into()]));
        assert!(!covers(&w, "WebFetch", "c:/p", &["fetch evil.com".into()]));
    }

    #[test]
    fn drafts_and_validation() {
        let f = bash("cd src && npm test 2>&1 | tail -15");
        let d = draft(&f, "proj", NOW);
        assert_eq!(d.pattern, ["npm test", "tail"]);
        assert_eq!(d.name, "npm test, tail in proj");
        assert_eq!(d.project.as_deref(), Some("c:/p"));
        assert!(validate(&d, NOW).is_ok());
        assert_eq!(draft(&edit("C:/p/src/lib/a.ts"), "", NOW).pattern, ["src/lib/**"]);

        let mut a = d.clone();
        a.auto_allow = true;
        a.expires_at = Some(NOW + 7 * DAY_MS);
        assert!(validate(&a, NOW).unwrap_err().contains("dry run"));
        a.dry_run = Some(dry_run(&a, &[answer(&f, "allow")], &[], NOW));
        assert!(validate(&a, NOW).is_ok());
        a.expires_at = Some(NOW + 31 * DAY_MS + 1);
        assert!(validate(&a, NOW).unwrap_err().contains("30 days"));
        a.expires_at = Some(NOW + 7 * DAY_MS);
        a.dry_run = Some(dry_run(&a, &[], &[], NOW));
        assert!(validate(&a, NOW).unwrap_err().contains("nothing you allowed"));

        let mut s = d;
        s.pattern = vec!["bash".into()];
        assert!(validate(&s, NOW).is_err());
    }

    #[test]
    fn questions_are_never_touched() {
        let ev = json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": []}});
        let a = crest_intel::analyze_tool("AskUserQuestion", &ev["tool_input"], &ctx(), None);
        let f = facts(&ev, &a, "C:/p");
        let r = Rule { id: "q".into(), name: "q".into(), enabled: true, tool: "AskUserQuestion".into(), auto_allow: true, expires_at: Some(NOW + DAY_MS), ..Default::default() };
        assert_eq!(decide(&[r], &f, "", NOW), Decision::default());
    }

    /// Timing for the Stage C report: `cargo test --release -- --ignored --nocapture rules_timing`.
    /// 50 armed rules against one request (the hook's path), and a dry run over the full 10,000-answer log.
    #[test]
    #[ignore]
    fn rules_timing() {
        let seen = bash("npm test");
        let mut history: Vec<Answer> = (0..10_000).map(|i| answer(&seen, if i % 7 == 0 { "deny" } else { "allow" })).collect();
        for (i, a) in history.iter_mut().enumerate() {
            if i % 3 == 0 {
                a.keys = vec![format!("cargo build{}", i % 40)];
            }
        }
        let rules: Vec<Rule> = (0..50)
            .map(|i| {
                let mut r = armed(&[if i == 49 { "npm test" } else { "cargo build" }], &history[..200]);
                r.id = format!("r{i}");
                r
            })
            .collect();
        let req = bash("npm test");
        let t = std::time::Instant::now();
        let n = 1000;
        for _ in 0..n {
            std::hint::black_box(decide(&rules, &req, "Runs tests.", NOW));
        }
        let per = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = std::time::Instant::now();
        let d = dry_run(&rules[49], &history, &[], NOW);
        let dry = t.elapsed().as_secs_f64() * 1e3;
        println!("decide over 50 rules: {per:.1} us per request; dry run over {} answers: {dry:.2} ms (matched {})", history.len(), d.matched);
    }
}

#[cfg(test)]
mod parity {
    use super::*;

    /// The Settings page computes the same fingerprint (src/lib/rules.test.ts checks this value).
    #[test]
    fn fingerprint_matches_the_page() {
        let r = Rule { tool: "Bash".into(), project: Some("c:/users/me/proj".into()), pattern: vec!["npm test".into(), " Tail ".into(), "npm test".into()], ..Default::default() };
        assert_eq!(fingerprint(&r), "c264f5babbc7a4a1".to_string(), "update rules.test.ts if this changes: {}", fingerprint(&r));
    }
}
