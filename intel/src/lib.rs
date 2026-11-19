//! Permission intelligence for Atoll.
//!
//! `analyze` turns one Bash or PowerShell command into a structured result:
//! what it reads, writes and deletes, whether it uses the network, pipes into
//! a shell, asks for privileges or hides what it does, and a risk level.
//!
//! Fail-safe rules, in one place:
//! - Anything that cannot be parsed, is cut off, is too long, or makes the
//!   analysis crash comes back as `Risk::Unknown` (or `Risky` when the text
//!   plainly contains something destructive). It is never eligible for auto-allow.
//! - A finding can only raise the risk, never lower it.
//! - Where the analysis has to guess (unknown folder, unknown variable), it
//!   takes the worse reading.

mod bash;
pub mod diff;
mod classify;
mod model;
mod paths;
mod powershell;
mod summary;
mod other;
mod tools;

use std::time::Instant;

pub use diff::{Diff, DiffLine, LineKind};
pub use other::analyze_tool;
pub use model::{Analysis, CommandShape, Context, Finding, Flag, Risk, Shell};

use classify::{Cwd, Cx};
use model::Builder;

/// Longest command analysed. Longer ones are not worth trusting a verdict on.
pub const MAX_COMMAND_BYTES: usize = 20_000;
/// The bridge cuts every string at this many bytes and appends "…".
pub const BRIDGE_CUT: usize = 4_000;

/// Analyse one command. `tool` is the Claude Code tool name ("Bash" or "PowerShell").
/// Never panics and never blocks on I/O: it only looks at the text.
pub fn analyze(tool: &str, command: &str, ctx: &Context) -> Analysis {
    let start = Instant::now();
    let shell = if tool.eq_ignore_ascii_case("powershell") { Shell::PowerShell } else { Shell::Bash };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(shell, command, ctx)));
    let mut analysis = match result {
        Ok(a) => a,
        Err(_) => {
            let mut b = Builder::default();
            b.find(Flag::Unparsed, Risk::Unknown, "the analysis failed on this command".to_string());
            finish(b, shell)
        }
    };
    analysis.summary = summary::shell(&analysis);
    analysis.reasons = summary::reasons(&analysis);
    analysis.elapsed_us = start.elapsed().as_micros() as u64;
    analysis
}

fn run(shell: Shell, command: &str, ctx: &Context) -> Analysis {
    let mut b = Builder { parsed: true, ..Default::default() };
    if command.len() > MAX_COMMAND_BYTES {
        b.parsed = false;
        b.find(Flag::TooLong, Risk::Unknown, "the command is too long to analyse reliably".to_string());
        raw_floor(&mut b, &command[..floor_char(command, MAX_COMMAND_BYTES)]);
        return finish(b, shell);
    }
    let cut_off = command.len() >= BRIDGE_CUT && command.trim_end().ends_with('…');
    if cut_off {
        b.parsed = false;
        b.find(Flag::Truncated, Risk::Unknown, "the command was cut short before Atoll saw it, so the end is missing".to_string());
    }
    hidden_characters(&mut b, command);

    let cwd: Cwd = if ctx.cwd.is_empty() { None } else { Some(vec![paths::norm(&ctx.cwd)]) };
    {
        let mut cx = Cx { ctx, b: &mut b, depth: 0, functions: vec![] };
        match shell {
            Shell::Bash | Shell::Tool => nested_bash(&mut cx, command, &cwd),
            Shell::PowerShell => powershell::analyze_into(&mut cx, command, &cwd),
        }
    }
    if !b.parsed {
        b.find(Flag::Unparsed, Risk::Unknown, "part of the command could not be read, so this may be incomplete".to_string());
        raw_floor(&mut b, command);
    }
    finish(b, shell)
}

fn floor_char(s: &str, mut at: usize) -> usize {
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// When the structure is unknown, plain text patterns still catch the obvious.
fn raw_floor(b: &mut Builder, text: &str) {
    if classify::RAW_DANGER.is_match(text) {
        b.find(Flag::Delete, Risk::Risky, "the text contains something that can delete or overwrite work, or run hidden code".to_string());
    }
}

/// Characters that make text read differently from what runs.
fn hidden_characters(b: &mut Builder, command: &str) {
    let chars: Vec<char> = command.chars().collect();
    let bad = chars.iter().enumerate().any(|(i, &c)| {
        // Direction overrides reorder what is shown: always suspicious.
        let bidi = matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}');
        // Zero-width characters join emoji (👨‍👩‍👧) harmlessly; next to a letter or digit they can hide a word.
        let zero_width = matches!(c, '\u{200B}'..='\u{200D}' | '\u{00AD}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{180E}');
        let near_ascii = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(|n| n.is_ascii_alphanumeric());
        bidi || (zero_width && (near_ascii(i.checked_sub(1)) || near_ascii(Some(i + 1)))) || (c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
    });
    if bad {
        b.find(Flag::Obfuscated, Risk::Risky, "contains invisible or text-reversing characters, so it may not do what it appears to say".to_string());
    }
}

/// Parse and classify Bash text (the whole command, or text handed to `bash -c`, `eval`, a heredoc...).
pub(crate) fn nested_bash(cx: &mut Cx, text: &str, cwd: &Cwd) {
    if cx.depth > 4 {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "commands are nested unusually deep".to_string());
        return;
    }
    let Some(structure) = bash::parse(text, cx.ctx, cwd) else {
        cx.b.parsed = false;
        return;
    };
    if structure.has_error {
        cx.b.parsed = false;
    }
    // `:(){ :|:& };:` — a function that pipes into itself multiplies until the PC freezes.
    for f in &structure.functions {
        if text.contains(&format!("{f}|{f}")) || text.contains(&format!("{f} | {f}")) {
            cx.b.find(Flag::SystemChange, Risk::Risky, "starts copies of itself without end until the PC freezes (fork bomb)".to_string());
        }
    }
    cx.depth += 1;
    let outer_functions = cx.functions.clone();
    cx.functions.extend(structure.functions.iter().cloned());
    for (name, value) in &structure.assigned {
        classify::env_var(cx, cwd, name, value);
    }
    for inv in &structure.invocations {
        classify::invocation(cx, inv);
    }
    if structure.background && !cx.b.findings.iter().any(|f| f.flag == Flag::Background) {
        cx.b.find(Flag::Background, Risk::Caution, "keeps running in the background".to_string());
    }
    cx.functions = outer_functions;
    cx.depth -= 1;
}

pub(crate) fn finish(mut b: Builder, shell: Shell) -> Analysis {
    // A secret read plus a way out, in one command, is how data leaves.
    if b.remote_network && b.secret_reads.iter().any(|s| !s.is_empty()) {
        let what = b.secret_reads.iter().find(|s| !s.is_empty()).cloned().unwrap_or_default();
        let dest = b.hosts.iter().find(|h| !classify::is_local_host(h)).cloned().unwrap_or_else(|| "the internet".into());
        b.find(Flag::Exfiltration, Risk::Risky, format!("can send {what} to {dest}"));
    }
    let mut risk = b.findings.iter().map(|f| f.risk).max().unwrap_or(Risk::Safe);
    if !b.parsed && risk < Risk::Unknown {
        risk = Risk::Unknown;
    }
    let mut flags: Vec<Flag> = b.findings.iter().map(|f| f.flag).collect();
    flags.sort();
    flags.dedup();
    // Most worrying first: that is the order the peek shows them in.
    b.findings.sort_by(|x, y| y.risk.cmp(&x.risk));
    let ineligible_reason = eligibility(risk, b.parsed, shell, &flags);
    Analysis {
        risk,
        shell,
        parsed: b.parsed,
        flags,
        findings: b.findings,
        commands: b.commands,
        reads: b.reads,
        writes: b.writes,
        deletes: b.deletes,
        hosts: b.hosts,
        auto_allow_eligible: ineligible_reason.is_empty(),
        ineligible_reason,
        summary: String::new(),
        reasons: vec![],
        outside: std::mem::take(&mut b.outside),
        diff: None,
        elapsed_us: 0,
    }
}

/// Why a rule may not auto-allow this request. Empty = it may.
fn eligibility(risk: Risk, parsed: bool, shell: Shell, flags: &[Flag]) -> String {
    if !parsed || risk == Risk::Unknown {
        return "Atoll could not fully read this command".into();
    }
    if risk == Risk::Risky {
        return "it is flagged risky".into();
    }
    if shell == Shell::PowerShell {
        return "PowerShell commands are explained but not auto-allowed in this version".into();
    }
    if flags.contains(&Flag::InlineCode) {
        return "it runs code written inside the command, which Atoll cannot fully check".into();
    }
    String::new()
}

/// The result to show when the analysis could not run in time or at all:
/// `unknown`, never eligible for auto-allow.
pub fn unchecked(reason: &str) -> Analysis {
    let mut b = Builder::default();
    b.find(Flag::Unparsed, Risk::Unknown, reason.to_string());
    let mut a = finish(b, Shell::Tool);
    a.summary = format!("{}.", summary::capitalise(reason.trim_end_matches('.')));
    a
}
