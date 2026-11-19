//! Sweep: run the analyser over a large file of real commands (unlabelled) to
//! measure parse failures, the spread of risk levels and timing, and to pull
//! samples for a manual spot check. Ignored by default because the input is
//! the owner's private transcript data, which never goes in the repository.
//!
//! SWEEP_IN=<commands.jsonl> SWEEP_OUT=<report.txt> \
//!   cargo test -p crest-intel --release --test sweep -- --ignored --nocapture
//!
//! Each input line: {"tool": "Bash" | "PowerShell", "command": "...", "cwd": "..."}

use std::collections::HashMap;
use std::time::Instant;

use crest_intel::{analyze, Context, Risk};

#[test]
#[ignore]
fn sweep() {
    let input = std::env::var("SWEEP_IN").expect("set SWEEP_IN");
    let out = std::env::var("SWEEP_OUT").expect("set SWEEP_OUT");
    let text = std::fs::read_to_string(input).expect("read SWEEP_IN");
    let mut counts: HashMap<(String, &str), usize> = HashMap::new();
    let mut times = Vec::new();
    let mut samples: HashMap<&str, Vec<String>> = HashMap::new();
    let mut eligible = 0usize;
    let mut total = 0usize;
    for (i, line) in text.lines().enumerate() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let tool = v["tool"].as_str().unwrap_or("Bash").to_string();
        let cmd = v["command"].as_str().unwrap_or("");
        let cwd = v["cwd"].as_str().unwrap_or("");
        // The session's starting folder is the project; `cwd` is where the shell is now.
        let project = v["project"].as_str().filter(|p| !p.is_empty()).unwrap_or(cwd);
        let ctx = Context::from_system(project, cwd);
        let t = Instant::now();
        let a = analyze(&tool, cmd, &ctx);
        times.push(t.elapsed().as_secs_f64() * 1e6);
        total += 1;
        if a.auto_allow_eligible {
            eligible += 1;
        }
        let level = match a.risk {
            Risk::Safe => "safe",
            Risk::Caution => "caution",
            Risk::Unknown => "unknown",
            Risk::Risky => "risky",
        };
        *counts.entry((tool.clone(), level)).or_default() += 1;
        // Every 37th of each level (and every unknown) for a manual look.
        let list = samples.entry(level).or_default();
        if level == "unknown" || level == "risky" || i % 37 == 0 {
            // Every kind of line break folded, so each sample stays on one line next to its reason.
            let one: String = cmd.replace("\r\n", " ⏎ ").replace(['\n', '\r', '\u{2028}', '\u{2029}', '\u{0085}', '\u{000B}', '\u{000C}'], " ⏎ ").replace('\0', "\\0").chars().take(220).collect();
            let why = a.findings.first().map(|f| f.note.clone()).unwrap_or_default();
            list.push(format!("[{tool}] {one}\n      -> {why}"));
        }
    }
    times.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pct = |p: f64| times[((times.len() as f64 - 1.0) * p) as usize];
    let mut report = format!(
        "Commands: {total}\nEligible for a rule's auto-allow: {eligible}\nTime: median {:.0} µs, p95 {:.0} µs, p99 {:.0} µs, max {:.0} µs\n\n",
        pct(0.5),
        pct(0.95),
        pct(0.99),
        times.last().copied().unwrap_or(0.0)
    );
    let mut keys: Vec<_> = counts.iter().collect();
    keys.sort();
    for ((tool, level), n) in keys {
        report.push_str(&format!("{tool:<10} {level:<8} {n}\n"));
    }
    for level in ["unknown", "risky", "caution", "safe"] {
        report.push_str(&format!("\n===== {level} samples =====\n"));
        for s in samples.get(level).into_iter().flatten() {
            report.push_str(s);
            report.push('\n');
        }
    }
    std::fs::write(out, report).expect("write SWEEP_OUT");
}
