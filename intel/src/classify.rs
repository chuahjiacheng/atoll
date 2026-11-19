//! What each program does: turns one invocation into findings.
//! Shared by the Bash front end and (for native programs) the PowerShell one.

use std::sync::LazyLock;

use regex::Regex;

use crate::bash::{Invocation, Word};
use crate::model::{Builder, CommandShape, Context, Flag, Risk};
use crate::paths::{self, Zone};
use crate::tools;

pub(crate) type Cwd = Option<Vec<String>>;

pub(crate) struct Cx<'a> {
    pub ctx: &'a Context,
    pub b: &'a mut Builder,
    /// How many shells deep we are (`bash -c "bash -c ..."`).
    pub depth: usize,
    /// Shell functions defined in the same command.
    pub functions: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Write {
    /// mkdir, touch: makes something new, replaces nothing.
    Create,
    Append,
    /// Replaces what was there.
    Truncate,
}

/// A path short enough to read in a sentence.
pub(crate) fn short(text: &str) -> String {
    let t = text.replace('\\', "/");
    let t = t.trim_end_matches('/');
    if t.chars().count() <= 44 {
        return if t.is_empty() { "/".into() } else { t.to_string() };
    }
    let parts: Vec<&str> = t.split('/').collect();
    let tail = parts[parts.len().saturating_sub(2)..].join("/");
    format!("…/{}", tail.chars().rev().take(40).collect::<String>().chars().rev().collect::<String>())
}

/// Folder part of a path that has wildcards: the part before the first wildcard.
fn glob_dir(text: &str) -> String {
    let cut = text.find(['*', '?', '[', '{']).unwrap_or(text.len());
    let head = &text[..cut];
    match head.rfind(['/', '\\']) {
        Some(i) => head[..=i].to_string(),
        None => ".".to_string(),
    }
}

/// The worse of the word's two readings: as written, and with maybe-unset variables empty.
fn zone_of(cx: &Cx, cwd: &Cwd, w: &Word) -> Option<Zone> {
    let main = zone_of_text(cx, cwd, w)?;
    match alt_word(w) {
        Some(alt) => zone_of_text(cx, cwd, &alt).and_then(|z| paths::worst(&[main, z])),
        None => Some(main),
    }
}

fn alt_word(w: &Word) -> Option<Word> {
    w.unset_alt.as_ref().filter(|a| !a.trim().is_empty()).map(|a| Word { text: a.clone(), unset_alt: None, ..w.clone() })
}

/// A path for a sentence, adding the empty-variable reading when that is the worse one.
fn named(cx: &Cx, cwd: &Cwd, w: &Word) -> String {
    if let Some(alt) = alt_word(w) {
        if zone_of_text(cx, cwd, &alt) != zone_of_text(cx, cwd, w) {
            return format!("{} (or {} if a variable it uses is empty)", short(&w.text), short(&alt.text));
        }
    }
    short(&w.text)
}

fn zone_of_text(cx: &Cx, cwd: &Cwd, w: &Word) -> Option<Zone> {
    if !w.exact {
        return None;
    }
    if paths::is_sink(&w.text) {
        return Some(Zone::Sink);
    }
    let text = if w.glob { glob_dir(&w.text) } else { w.text.clone() };
    let cands = paths::resolve(&text, cwd, cx.ctx);
    if cands.is_empty() {
        return None;
    }
    let zones: Vec<Zone> = cands.iter().map(|c| paths::zone(c, cx.ctx)).collect();
    paths::worst(&zones)
}

fn resolved(cx: &Cx, cwd: &Cwd, w: &Word) -> Vec<String> {
    if !w.exact || w.glob {
        return vec![];
    }
    paths::resolve(&w.text, cwd, cx.ctx)
}

/// Remember what a file written by this command contains (None = cannot see it).
pub(crate) fn created(cx: &mut Cx, cwd: &Cwd, w: &Word, text: Option<String>) {
    for p in resolved(cx, cwd, w) {
        match cx.b.created.iter_mut().find(|(q, _)| *q == p) {
            Some(entry) => entry.1 = text.clone(),
            None => cx.b.created.push((p, text.clone())),
        }
    }
}

/// `mv a b`, `cp a b`: b now holds whatever a held. When a was written or
/// downloaded by this same command, so is b; otherwise b is an existing file.
fn carry(cx: &mut Cx, cwd: &Cwd, sources: &[Word], dest: &Word) {
    let mut targets = resolved(cx, cwd, dest);
    if sources.len() == 1 {
        // `mv a dir/` lands at dir/a.
        let base = sources[0].text.replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase();
        let extra: Vec<String> = targets.iter().map(|t| format!("{t}/{base}")).collect();
        targets.extend(extra);
    }
    let src: Vec<String> = sources.iter().flat_map(|s| resolved(cx, cwd, s)).collect();
    let content = cx.b.created.iter().find(|(p, _)| src.contains(p)).map(|(_, c)| c.clone());
    let fetched = src.iter().any(|p| cx.b.downloaded.contains(p));
    cx.b.created.retain(|(p, _)| !targets.contains(p));
    if let Some(c) = content {
        for t in &targets {
            cx.b.created.push((t.clone(), c.clone()));
        }
    }
    if fetched {
        for t in targets {
            if !cx.b.downloaded.contains(&t) {
                cx.b.downloaded.push(t);
            }
        }
    }
}

pub(crate) fn downloaded(cx: &mut Cx, cwd: &Cwd, w: &Word) {
    for p in resolved(cx, cwd, w) {
        if !cx.b.downloaded.contains(&p) {
            cx.b.downloaded.push(p);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunKind {
    Shell,
    Python,
    Js,
    Other,
}

fn kind_of(name: &str, text: &str) -> RunKind {
    let t = name.to_lowercase();
    if t.ends_with(".py") {
        RunKind::Python
    } else if t.ends_with(".js") || t.ends_with(".mjs") || t.ends_with(".cjs") || t.ends_with(".ts") {
        RunKind::Js
    } else if t.ends_with(".sh") || t.ends_with(".bash") || text.starts_with("#!/bin/sh") || text.starts_with("#!/bin/bash") || text.starts_with("#!/usr/bin/env bash") {
        RunKind::Shell
    } else {
        RunKind::Other
    }
}

/// The command runs this file. Dangerous when the same command fetched or wrote it:
/// what runs is then whatever arrived, not something already on disk to look at.
pub(crate) fn runs_file(cx: &mut Cx, cwd: &Cwd, w: &Word, kind: RunKind) {
    let cands = resolved(cx, cwd, w);
    if cands.is_empty() {
        return;
    }
    if cands.iter().any(|c| cx.b.downloaded.contains(c)) {
        cx.b.find(Flag::RemoteCode, Risk::Risky, format!("downloads {} and then runs it", short(&w.text)));
        return;
    }
    let hit = cx.b.created.iter().find(|(p, _)| cands.contains(p)).cloned();
    match hit {
        Some((_, Some(text))) => {
            let kind = if kind == RunKind::Other { kind_of(&w.text, &text) } else { kind };
            match kind {
                RunKind::Shell => crate::nested_bash(cx, &text, cwd),
                RunKind::Python => tools::inline_code(cx, cwd, tools::Lang::Python, "Python", &text),
                RunKind::Js => tools::inline_code(cx, cwd, tools::Lang::Js, "JavaScript", &text),
                RunKind::Other => {
                    tools::inline_code(cx, cwd, tools::Lang::Other, "script", &text);
                    if RAW_DANGER.is_match(&text) {
                        cx.b.find(Flag::Delete, Risk::Risky, "the script it writes and runs can delete or overwrite work".to_string());
                    }
                }
            }
        }
        Some((_, None)) => cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("runs {}, a file this same command creates, so its contents cannot be checked", short(&w.text))),
        None => {}
    }
}

pub(crate) fn write(cx: &mut Cx, cwd: &Cwd, w: &Word, how: Write) {
    if w.exact && paths::is_sink(&w.text) {
        return;
    }
    if w.exact && (w.text.starts_with("/dev/tcp/") || w.text.starts_with("/dev/udp/")) {
        cx.b.find(Flag::Network, Risk::Risky, format!("opens a raw network connection ({})", short(&w.text)));
        cx.b.remote_network = true;
        return;
    }
    // Writes use the value as set: an empty-variable reading matters for deletes only.
    let name = short(&w.text);
    let Some(zone) = zone_of_text(cx, cwd, w) else {
        // `> /tmp/run$i.log`: the folder is plain even though the name is filled in later.
        if let Some(dir) = known_folder(w) {
            // Judge a file inside the folder, not the folder itself (`/tmp` is a temp root).
            let inside = Word::lit(&format!("{}/x", dir.text));
            if matches!(zone_of(cx, cwd, &inside), Some(Zone::Temp | Zone::Project)) {
                Builder::push_unique(&mut cx.b.writes, &w.text);
                cx.b.find(Flag::UnresolvedPath, Risk::Caution, format!("writes a file in {} whose name is filled in when it runs", short(&dir.text)));
                return;
            }
        }
        cx.b.find(Flag::UnresolvedPath, Risk::Risky, format!("writes to {name}, a location only known when it runs"));
        return;
    };
    Builder::push_unique(&mut cx.b.writes, &w.text);
    match zone {
        Zone::Sink => {}
        Zone::Temp | Zone::Project | Zone::ProjectRoot => {
            cx.b.find(Flag::WritesFile, Risk::Caution, format!("writes {name}"));
            if how == Write::Truncate {
                cx.b.find(Flag::Overwrite, Risk::Caution, format!("replaces {name} if it exists"));
            }
        }
        Zone::GitInternals => cx.b.find(Flag::Overwrite, Risk::Risky, format!("changes git's own files ({name})")),
        Zone::Secret => cx.b.find(Flag::SensitivePath, Risk::Risky, format!("writes to {name}, which holds secrets")),
        Zone::Persistence => cx.b.find(Flag::HiddenExec, Risk::Risky, format!("changes {name}, which runs or is trusted automatically later")),
        Zone::System => cx.b.find(Flag::SystemChange, Risk::Risky, format!("writes to a system location ({name})")),
        Zone::Elsewhere => {
            if how == Write::Create {
                cx.b.find(Flag::OutsideProject, Risk::Caution, format!("creates {name}, outside the project"));
            } else {
                cx.b.find(Flag::OutsideProject, Risk::Risky, format!("writes {name}, outside the project"));
            }
        }
    }
}

/// The plain folder in front of a partly unknown path: `/tmp/out/$name.log` -> `/tmp/out`.
/// None when the unknown part could itself hold a folder change (`/tmp/$x/..`) at the start.
fn known_folder(w: &Word) -> Option<Word> {
    let cut = w.text.find(['$', '`'])?;
    let head = &w.text[..cut];
    let tail = &w.text[cut..];
    let sep = head.rfind(['/', '\\'])?;
    // A later folder separator means the unknown part names a folder, not just a file.
    if tail.contains(['/', '\\']) || head[..sep].is_empty() {
        return None;
    }
    Some(Word::lit(&head[..sep]))
}

/// Folders that tools recreate: deleting them loses no one's work.
const REBUILDABLE: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", ".next", ".nuxt", ".svelte-kit", ".turbo", ".parcel-cache", ".cache",
    "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "coverage", ".nyc_output", ".gradle", ".dart_tool", ".expo",
];

/// True when every place `w` can point to is inside a rebuildable folder of the project.
fn rebuildable(cx: &Cx, cwd: &Cwd, w: &Word) -> bool {
    let text = if w.glob { glob_dir(&w.text) } else { w.text.clone() };
    let project = paths::norm(&cx.ctx.project);
    let cands = paths::resolve(&text, cwd, cx.ctx);
    !cands.is_empty()
        && cands.iter().all(|c| {
            paths::within(c, &project)
                && c[project.len()..].split('/').any(|part| REBUILDABLE.contains(&part))
        })
}

/// `strong` = recursive or wildcard: can take a lot with it.
pub(crate) fn delete(cx: &mut Cx, cwd: &Cwd, w: &Word, strong: bool) {
    let name = named(cx, cwd, w);
    let strong = strong || w.glob;
    let Some(zone) = zone_of(cx, cwd, w) else {
        cx.b.find(Flag::Delete, Risk::Risky, format!("deletes {name}, a location only known when it runs"));
        cx.b.find(Flag::UnresolvedPath, Risk::Risky, format!("cannot tell what {name} points to"));
        return;
    };
    Builder::push_unique(&mut cx.b.deletes, &w.text);
    // A name with an extension is most likely a file: "and everything in it" would mislead.
    let base = w.text.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    let looks_like_file = !w.glob && base.rfind('.').is_some_and(|i| i > 0 && i + 1 < base.len());
    let everything = if strong && !looks_like_file { " and everything in it" } else { "" };
    match zone {
        Zone::Sink => {}
        Zone::Temp => cx.b.find(Flag::Delete, Risk::Caution, format!("deletes {name} (a temp folder)")),
        Zone::Project if !strong => cx.b.find(Flag::Delete, Risk::Caution, format!("deletes {name}")),
        Zone::Project if rebuildable(cx, cwd, w) => cx.b.find(Flag::Delete, Risk::Caution, format!("deletes {name}{everything} (build output or downloaded packages, which can be rebuilt)")),
        Zone::Project => cx.b.find(Flag::Delete, Risk::Risky, format!("deletes {name}{everything}")),
        Zone::ProjectRoot if w.glob => cx.b.find(Flag::Delete, Risk::Risky, format!("deletes everything matching {name} in the project folder")),
        Zone::ProjectRoot => cx.b.find(Flag::Delete, Risk::Risky, "deletes the project folder or everything in it".to_string()),
        Zone::GitInternals => cx.b.find(Flag::Delete, Risk::Risky, format!("deletes git's own files ({name})")),
        Zone::Secret | Zone::Persistence => cx.b.find(Flag::Delete, Risk::Risky, format!("deletes {name}")),
        Zone::System => cx.b.find(Flag::Delete, Risk::Risky, format!("deletes a system or parent folder ({name})")),
        Zone::Elsewhere => {
            cx.b.find(Flag::Delete, Risk::Risky, format!("deletes {name}{everything}, outside the project"));
            cx.b.find(Flag::OutsideProject, Risk::Risky, format!("{name} is outside the project"));
        }
    }
}

pub(crate) fn read(cx: &mut Cx, cwd: &Cwd, w: &Word) {
    if !w.exact || w.text.starts_with('-') {
        return;
    }
    if zone_of(cx, cwd, w) == Some(Zone::Secret) {
        cx.b.find(Flag::SensitivePath, Risk::Caution, format!("reads {}, which holds secrets", short(&w.text)));
        Builder::push_unique(&mut cx.b.secret_reads, &w.text);
    }
    Builder::push_unique(&mut cx.b.reads, &w.text);
}

/// The source of a move: it is gone from where it was.
fn move_from(cx: &mut Cx, cwd: &Cwd, w: &Word) {
    let name = named(cx, cwd, w);
    match zone_of(cx, cwd, w) {
        None => cx.b.find(Flag::UnresolvedPath, Risk::Risky, format!("moves {name}, a location only known when it runs")),
        Some(Zone::Temp | Zone::Project | Zone::Sink) => {}
        Some(Zone::Secret) => {
            cx.b.find(Flag::SensitivePath, Risk::Risky, format!("moves {name}, which holds secrets"));
        }
        Some(_) => cx.b.find(Flag::OutsideProject, Risk::Risky, format!("moves {name} away from where it is")),
    }
}

// ---------------------------------------------------------------- arguments

pub(crate) fn is_opt(w: &Word) -> bool {
    let t = w.text.as_str();
    w.exact && t.len() > 1 && t.starts_with('-') && t != "--" && !t[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Options and plain arguments. `valued` lists options whose value is the next word.
pub(crate) struct Args {
    pub opts: Vec<String>,
    pub pos: Vec<Word>,
}

pub(crate) fn split(args: &[Word], valued: &[&str]) -> Args {
    let mut out = Args { opts: vec![], pos: vec![] };
    let mut i = 0;
    let mut only_pos = false;
    while i < args.len() {
        let w = &args[i];
        if !only_pos && w.exact && w.text == "--" {
            only_pos = true;
        } else if !only_pos && is_opt(w) {
            out.opts.push(w.text.clone());
            let name = w.text.split('=').next().unwrap_or("");
            if valued.contains(&name) && !w.text.contains('=') {
                i += 1;
            }
        } else {
            out.pos.push(w.clone());
        }
        i += 1;
    }
    out
}

impl Args {
    /// `-rf` has short option `r`; `--recursive` is a long option.
    pub fn short(&self, c: char) -> bool {
        self.opts.iter().any(|o| !o.starts_with("--") && o[1..].contains(c))
    }
    pub fn long(&self, name: &str) -> bool {
        self.opts.iter().any(|o| o == name || o.starts_with(&format!("{name}=")))
    }
    pub fn any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| if n.starts_with("--") { self.long(n) } else { self.opts.iter().any(|o| o == n) })
    }
}

/// Value of `-o file`, `--output file` or `--output=file`.
pub(crate) fn value_of(args: &[Word], names: &[&str]) -> Option<Word> {
    for (i, w) in args.iter().enumerate() {
        if !w.exact {
            continue;
        }
        for n in names {
            if w.text == *n {
                return args.get(i + 1).cloned();
            }
            if let Some(rest) = w.text.strip_prefix(&format!("{n}=")) {
                return Some(Word { text: rest.to_string(), ..w.clone() });
            }
            // -ofile
            if n.len() == 2 && !n.starts_with("--") && w.text.len() > 2 && w.text.starts_with(n) && !w.text.starts_with("--") {
                return Some(Word { text: w.text[2..].to_string(), ..w.clone() });
            }
        }
    }
    None
}

pub(crate) fn prog_name(text: &str) -> (String, bool) {
    let t = text.replace('\\', "/");
    let has_path = t.contains('/');
    let mut base = t.rsplit('/').next().unwrap_or("").to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(b) = base.strip_suffix(ext) {
            base = b.to_string();
            break;
        }
    }
    (base, has_path)
}

pub(crate) fn shape(cx: &mut Cx, program: &str, sub: &str, args: &[Word]) {
    if cx.b.commands.len() >= 32 {
        return;
    }
    let mut options: Vec<String> = args.iter().filter(|w| is_opt(w)).map(|w| w.text.split('=').next().unwrap_or("").to_string()).collect();
    options.sort();
    options.dedup();
    let s = CommandShape { program: program.to_string(), sub: sub.to_string(), options };
    if !cx.b.commands.contains(&s) {
        cx.b.commands.push(s);
    }
}

// ---------------------------------------------------------------- hosts

pub(crate) fn host_of(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = match u.find("://") {
        Some(i) if u[..i].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.') => &u[i + 3..],
        _ => u,
    };
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    let rest = rest.rsplit('@').next().unwrap_or("");
    let host = if rest.starts_with('[') { rest.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { rest.split(':').next().unwrap_or("").to_string() };
    let host = host.to_lowercase();
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']' | ':')) {
        return None;
    }
    Some(host)
}

pub(crate) fn is_local_host(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || host == "[::1]" || host == "0.0.0.0" || host == "::1"
}

pub(crate) fn looks_like_url(w: &Word) -> bool {
    let t = w.text.to_lowercase();
    t.contains("://") || t.starts_with("www.")
}

/// Record a network destination. Unknown or non-local hosts can carry data out.
pub(crate) fn network(cx: &mut Cx, target: Option<&Word>, what: &str) {
    match target {
        Some(w) if w.exact => match host_of(&w.text) {
            Some(h) if is_local_host(&h) => {
                Builder::push_unique(&mut cx.b.hosts, &h);
                cx.b.find(Flag::Network, Risk::Safe, format!("{what} this computer ({h})"));
            }
            Some(h) => {
                Builder::push_unique(&mut cx.b.hosts, &h);
                cx.b.find(Flag::Network, Risk::Caution, format!("{what} {h}"));
                cx.b.remote_network = true;
            }
            None => {
                cx.b.find(Flag::Network, Risk::Caution, format!("{what} the network"));
                cx.b.remote_network = true;
            }
        },
        _ => {
            cx.b.find(Flag::Network, Risk::Caution, format!("{what} an address only known when it runs"));
            cx.b.remote_network = true;
        }
    }
}

// ---------------------------------------------------------------- environment variables

/// Variables that make later programs run something else.
const EXEC_VARS: &[&str] = &[
    "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "BASH_ENV", "ENV", "PROMPT_COMMAND",
    "GIT_SSH", "GIT_SSH_COMMAND", "GIT_EXTERNAL_DIFF", "GIT_PAGER", "PAGER", "GIT_EDITOR", "EDITOR", "VISUAL", "GIT_ASKPASS", "SSH_ASKPASS",
    "GIT_EXEC_PATH", "GIT_TEMPLATE_DIR", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM", "GIT_PROXY_COMMAND", "NODE_OPTIONS", "PYTHONSTARTUP", "PERL5OPT", "PERL5DB", "RUBYOPT",
    "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "BROWSER", "SHELL", "HOME", "USERPROFILE", "XDG_CONFIG_HOME", "ZDOTDIR", "COMSPEC", "PATHEXT",
    "RUSTC_WRAPPER", "RUSTC", "CARGO_BUILD_RUSTC_WRAPPER", "LESSOPEN", "LESSCLOSE", "MANPAGER", "TAR_OPTIONS", "HISTFILE", "PS4", "BASH_FUNC", "GCONV_PATH", "PYTHONINSPECT",
    "NPM_CONFIG_SCRIPT_SHELL", "NPM_CONFIG_REGISTRY", "PIP_INDEX_URL", "PIP_EXTRA_INDEX_URL", "DOCKER_HOST", "KUBECONFIG",
];

pub(crate) fn env_var(cx: &mut Cx, cwd: &Cwd, name: &str, value: &Word) {
    let upper = name.to_uppercase();
    if upper == "IFS" {
        // `IFS= read -r line` and `IFS=$'\n'` are everyday shell; anything else changes how words split.
        if !(value.exact && value.text.chars().all(char::is_whitespace)) {
            cx.b.find(Flag::HiddenExec, Risk::Caution, format!("changes how the shell splits words (IFS={})", value.text.escape_debug()));
        }
        return;
    }
    if EXEC_VARS.contains(&upper.as_str()) || upper.starts_with("BASH_FUNC_") || upper.starts_with("GIT_CONFIG_") {
        cx.b.find(Flag::HiddenExec, Risk::Risky, format!("sets {name}, which changes what later programs run"));
        return;
    }
    if upper == "PATH" || upper == "PYTHONPATH" || upper == "NODE_PATH" || upper == "PERL5LIB" || upper == "RUBYLIB" || upper == "CLASSPATH" {
        // Prepending a folder is normal; a relative or temp folder is how a fake `git` gets in.
        // `C:\x;C:\y` (Windows) or `/x:/y`. An empty entry means the current folder only with `:`.
        let windows_style = value.text.contains(';');
        let text = value.text.replace(';', ":");
        let mut drive_pending: Option<String> = None;
        for part in text.split(':') {
            // Re-join `C:` + `/path` split by the colon.
            let part = match drive_pending.take() {
                Some(d) => format!("{d}:{part}"),
                None => {
                    if part.len() == 1 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                        drive_pending = Some(part.to_string());
                        continue;
                    }
                    part.to_string()
                }
            };
            if part.is_empty() || part.contains('$') || part.contains('%') {
                if part.is_empty() && upper == "PATH" && !windows_style {
                    cx.b.find(Flag::HiddenExec, Risk::Risky, "adds the current folder to PATH".to_string());
                }
                continue;
            }
            let n = paths::norm(&part);
            let bad = !paths::is_abs(&n) || matches!(paths::zone(&n, cx.ctx), Zone::Temp);
            if bad {
                cx.b.find(Flag::HiddenExec, Risk::Risky, format!("puts {} on {name}, so programs there run instead of the real ones", short(&part)));
            }
        }
        let _ = cwd;
        return;
    }
    if matches!(upper.as_str(), "NODE_TLS_REJECT_UNAUTHORIZED" | "GIT_SSL_NO_VERIFY" | "PYTHONHTTPSVERIFY" | "CURL_CA_BUNDLE" | "SSL_CERT_FILE" | "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY") {
        cx.b.find(Flag::Network, Risk::Risky, format!("sets {name}, which weakens or reroutes network security"));
    }
}

// ---------------------------------------------------------------- one invocation

const WRAPPERS: &[&str] = &["command", "builtin", "exec", "nohup", "time", "nice", "ionice", "stdbuf", "setsid", "chronic", "unbuffer", "winpty", "caffeinate", "timeout", "gtimeout", "watch", "xargs", "busybox", "chrt", "taskset", "doas", "sudo", "gsudo", "pkexec", "runas", "env", "wsl", "flock", "strace", "ltrace", "script", "proxychains", "torsocks", "arch", "rosetta"];

pub(crate) fn invocation(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    // `cat > f <<EOF`, `echo text > f`, `printf text > f`: the file's contents are in plain sight.
    let head = inv.argv.first().filter(|w| w.exact).map(|w| prog_name(&w.text).0).unwrap_or_default();
    let shown: Option<String> = match (&inv.stdin_text, head.as_str()) {
        (Some(t), "cat" | "tee" | "") => Some(t.clone()),
        (None, "echo" | "printf") if inv.argv[1..].iter().all(|w| w.exact) => Some(
            inv.argv[1..].iter().filter(|w| !(w.text.starts_with('-') && w.text.len() <= 3)).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
        ),
        _ => None,
    };
    for r in &inv.redirects {
        if r.op.starts_with('<') {
            read(cx, &cwd, &r.target);
            continue;
        }
        let append = r.op.contains(">>");
        write(cx, &cwd, &r.target, if append { Write::Append } else { Write::Truncate });
        // Remember what went into the file, in case the same command runs it later.
        match (&shown, append) {
            (Some(t), false) => created(cx, &cwd, &r.target, Some(t.clone())),
            (Some(t), true) => {
                let before = resolved(cx, &cwd, &r.target).iter().find_map(|p| cx.b.created.iter().find(|(q, _)| q == p).map(|(_, c)| c.clone()));
                // Appending to a file this command did not write: the old part is an existing file, like any other.
                let text = match before {
                    Some(Some(old)) => Some(format!("{old}{t}")),
                    Some(None) => None,
                    None => Some(t.clone()),
                };
                created(cx, &cwd, &r.target, text);
            }
            (None, _) => created(cx, &cwd, &r.target, None),
        }
    }
    if let Some(text) = shown {
        if head == "tee" {
            for w in inv.argv[1..].iter().filter(|w| !is_opt(w)) {
                created(cx, &cwd, w, Some(text.clone()));
            }
        }
    }
    for (name, value) in &inv.env {
        env_var(cx, &cwd, name, value);
    }
    if inv.background {
        cx.b.find(Flag::Background, Risk::Caution, "keeps running in the background".to_string());
    }
    if inv.argv.is_empty() {
        return;
    }
    let mut argv: Vec<Word> = inv.argv.clone();
    let mut inv = inv.clone();

    // Strip wrappers until the real program is in front.
    for _ in 0..8 {
        let head = &argv[0];
        if !head.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the program to run is only decided when it runs".to_string());
            return;
        }
        let (name, _) = prog_name(&head.text);
        // A line break inside a program name means the text was misread (often a heredoc):
        // say so instead of guessing.
        if head.text.contains(['\n', '\r']) {
            cx.b.parsed = false;
            cx.b.find(Flag::Unparsed, Risk::Unknown, "part of the command could not be read, so this may be incomplete".to_string());
            return;
        }
        if !name.is_ascii() || head.text.chars().any(|c| c.is_control()) {
            cx.b.find(Flag::Obfuscated, Risk::Risky, format!("the program name “{}” uses look-alike or hidden characters", head.text.escape_debug()));
            return;
        }
        if head.obf {
            cx.b.find(Flag::Obfuscated, Risk::Risky, format!("the program name is written in a disguised way (it is “{name}”)"));
        }
        if !WRAPPERS.contains(&name.as_str()) {
            break;
        }
        match unwrap(cx, &name, &argv, &mut inv) {
            Some(rest) if !rest.is_empty() => argv = rest,
            _ => return,
        }
    }
    inv.argv = argv;
    program(cx, &inv);
}

/// Remove one wrapper. None = nothing more to classify.
fn unwrap(cx: &mut Cx, name: &str, argv: &[Word], inv: &mut Invocation) -> Option<Vec<Word>> {
    let args = &argv[1..];
    let skip = |valued: &[&str]| -> Vec<Word> {
        let mut i = 0;
        while i < args.len() && is_opt(&args[i]) {
            let n = args[i].text.as_str();
            i += if valued.contains(&n) { 2 } else { 1 };
        }
        args.get(i.min(args.len())..).unwrap_or(&[]).to_vec()
    };
    match name {
        "sudo" | "doas" | "gsudo" | "pkexec" | "runas" => {
            cx.b.find(Flag::Privilege, Risk::Risky, format!("runs with administrator rights ({name})"));
            shape(cx, name, "", args);
            Some(skip(&["-u", "-g", "-U", "-p", "-C", "-h", "-r", "-t", "-D", "--user", "--group", "/user"]))
        }
        "env" => {
            let mut rest = skip(&["-u", "--unset", "-C", "--chdir", "-S", "--split-string"]);
            while let Some(first) = rest.first() {
                match first.text.split_once('=') {
                    Some((k, v)) if first.exact && !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                        let value = Word { text: v.to_string(), ..first.clone() };
                        env_var(cx, &inv.cwd, k, &value);
                        rest.remove(0);
                    }
                    _ => break,
                }
            }
            if rest.is_empty() {
                shape(cx, "env", "", args);
                Builder::push_unique(&mut cx.b.secret_reads, "environment variables");
            }
            Some(rest)
        }
        "command" | "builtin" => {
            if args.first().is_some_and(|a| a.exact && (a.text == "-v" || a.text == "-V")) {
                shape(cx, name, "", args);
                return None;
            }
            Some(skip(&[]))
        }
        "timeout" | "gtimeout" => {
            let rest = skip(&["-s", "--signal", "-k", "--kill-after"]);
            Some(rest.get(1..).unwrap_or(&[]).to_vec())
        }
        "nice" | "ionice" | "chrt" | "taskset" | "stdbuf" | "flock" | "strace" | "ltrace" => {
            let rest = skip(&["-n", "-c", "-p", "-o", "-e", "-i", "--adjustment", "-w", "--timeout"]);
            // flock takes the lock file first; chrt/taskset take a number.
            let drop = match name {
                "flock" | "taskset" | "chrt" => 1,
                _ => 0,
            };
            Some(rest.get(drop.min(rest.len())..).unwrap_or(&[]).to_vec())
        }
        "watch" => Some(skip(&["-n", "--interval", "-d"])),
        "xargs" => {
            let mut rest = skip(&["-n", "-I", "-P", "-L", "-d", "-a", "-s", "-E", "--max-args", "--max-procs", "--delimiter", "--arg-file", "--replace"]);
            if rest.is_empty() {
                shape(cx, "xargs", "", args);
                return None;
            }
            // What xargs adds comes from another program's output: unknown.
            rest.push(Word { text: "(input)".into(), exact: false, ..Default::default() });
            inv.stdin_piped = false;
            Some(rest)
        }
        "wsl" => {
            if args.iter().any(|a| a.exact && matches!(a.text.as_str(), "--shutdown" | "--unregister" | "--terminate" | "-t")) {
                cx.b.find(Flag::SystemChange, Risk::Risky, "stops or removes a Linux system on this PC".to_string());
                shape(cx, "wsl", "", args);
                return None;
            }
            Some(skip(&["-d", "--distribution", "-u", "--user", "--cd", "-e", "--exec"]))
        }
        "script" => {
            cx.b.find(Flag::UnknownProgram, Risk::Caution, "records a terminal session".to_string());
            None
        }
        _ => Some(skip(&[])),
    }
}

static HELP_ONLY: &[&str] = &["--version", "-v", "-V", "--help", "-h", "version", "help", "-version", "/?"];

/// Read-only programs: nothing to find beyond which files they read.
const SAFE: &[&str] = &[
    "ls", "dir", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "ag", "ack", "file", "stat", "du", "df", "pwd", "echo", "printf",
    "date", "which", "where", "whereis", "type", "whoami", "id", "hostname", "uname", "ps", "tasklist", "netstat", "test", "[", "[[", "true",
    "false", "sleep", "basename", "dirname", "realpath", "readlink", "tree", "less", "more", "bat", "jq", "yq", "cut", "tr", "uniq", "diff",
    "cmp", "comm", "join", "paste", "seq", "nl", "tac", "rev", "fold", "expand", "column", "sha256sum", "sha1sum", "sha512sum", "md5sum",
    "cksum", "cygpath", "nproc", "free", "uptime", "lsof", "ipconfig", "ifconfig", "tty", "ver", "systeminfo", "locale", "groups", "cal",
    "hexdump", "od", "strings", "zcat", "zgrep", "lsblk", "getconf", "tput", "clear", "cls", "wait", "jobs", "read", "shift", "return",
    "exit", "break", "continue", ":", "set", "shopt", "local", "declare", "typeset", "export", "readonly", "umask", "getopts", "hash",
    "history", "fg", "bg", "dirs", "help", "man", "info", "pgrep", "findstr", "clip", "md5", "shasum", "numfmt", "fmt", "pr", "cd", "pushd",
    "popd", "let", "unset", "printenv", "base64", "xxd", "sort", "pidof", "top", "htop", "vmstat", "iostat", "lscpu", "sw_vers", "nvidia-smi",
    "mktemp", "tsort", "look", "wslpath", "chcp", "title", "disown", "complete", "compgen", "bind", "caller", "mapfile", "readarray", "times",
    "logname", "users", "who", "w", "last", "env", "sum", "b2sum", "dircolors", "pathchk", "factor", "expr", "bc", "dc", "yes", "tasklist",
    "get-process", "ss", "ip", "route", "arp", "tracert", "traceroute", "getent", "fc-list", "xdpyinfo", "lsusb", "lspci", "dmesg", "journalctl",
];

/// Programs that mostly run the project's own code or tooling.
const DEV_TOOLS: &[&str] = &[
    "tsc", "eslint", "prettier", "vitest", "jest", "pytest", "mocha", "svelte-check", "vite", "webpack", "rollup", "esbuild", "tauri",
    "graphify", "ruff", "mypy", "black", "isort", "flake8", "pylint", "rustc", "rustfmt", "gcc", "g++", "cc", "clang", "clang++", "cmake",
    "ninja", "gradle", "gradlew", "mvn", "mvnw", "ant", "bazel", "dotnet", "msbuild", "java", "javac", "kotlinc", "ruby", "bundle", "rake",
    "php", "composer", "lua", "swift", "swiftc", "xcodebuild", "flutter", "dart", "adb", "expo", "playwright", "cypress", "storybook",
    "turbo", "nx", "lerna", "tsx", "ts-node", "nodemon", "pm2", "serve", "http-server", "ffmpeg", "ffprobe", "magick", "convert", "pandoc",
    "make", "gmake", "nmake", "just", "task", "tox", "nox", "invoke", "meson", "scons", "zig", "ghc", "stack", "cabal", "mix", "elixir",
    "sbt", "lein", "clj", "rspec", "phpunit", "ava", "tap", "nyc", "c8", "stylelint", "biome", "oxlint", "tailwindcss", "sass", "lessc",
    "astro", "next", "nuxt", "remix", "svelte-kit", "ng", "vue-cli-service", "react-scripts", "parcel", "tsup", "unbuild", "wasm-pack",
    "cross", "trunk", "maturin", "hatch", "flit", "pre-commit", "husky", "lint-staged", "commitlint", "semantic-release", "changeset",
    "typedoc", "jsdoc", "sphinx-build", "mkdocs", "hugo", "jekyll", "gatsby", "eleventy", "claude", "ollama", "code", "cursor", "gemini",
    "qmake", "ctest", "cpack", "clippy-driver", "wasm-bindgen", "protoc", "buf", "sqlx", "diesel", "prisma", "drizzle-kit", "knex", "alembic",
    "watchexec", "entr", "concurrently", "wait-on", "cross-env", "dotenv", "tsc-watch", "electron", "electron-builder", "nw", "godot", "unity",
];

fn program(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let argv = &inv.argv;
    let (name, has_path) = prog_name(&argv[0].text);
    let args = &argv[1..];

    if cx.functions.contains(&argv[0].text) {
        return;
    }
    let only_help = args.len() == 1 && args[0].exact && HELP_ONLY.contains(&args[0].text.as_str());

    match name.as_str() {
        // ---- shells and things that run text as code
        "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish" | "ash" | "csh" | "tcsh" => shell(cx, inv, &name),
        "eval" => {
            shape(cx, "eval", "", args);
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "builds a command from text and runs it (eval)".to_string());
            if args.iter().all(|a| a.exact) {
                let text = args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(" ");
                crate::nested_bash(cx, &text, &cwd);
            }
        }
        "source" | "." => {
            shape(cx, "source", "", args);
            match args.first() {
                Some(a) if a.subst || !a.exact => cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs a script produced by another command".to_string()),
                Some(a) => {
                    read(cx, &cwd, a);
                    cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(&a.text)));
                    runs_file(cx, &cwd, a, RunKind::Shell);
                }
                None => {}
            }
        }
        "trap" => {
            shape(cx, "trap", "", args);
            if let Some(a) = args.first() {
                if a.exact && !a.text.is_empty() && a.text != "-" && !is_opt(a) {
                    crate::nested_bash(cx, &a.text.clone(), &cwd);
                } else if !a.exact {
                    cx.b.find(Flag::DynamicCommand, Risk::Risky, "sets up a command to run later, built at run time".to_string());
                }
            }
        }
        "alias" => {
            shape(cx, "alias", "", args);
            if !args.is_empty() {
                cx.b.find(Flag::HiddenExec, Risk::Caution, "renames a command for the rest of the session".to_string());
            }
        }
        "cmd" => cmd_exe(cx, inv),
        "powershell" | "pwsh" => powershell_exe(cx, inv, &name),

        // ---- files
        "rm" | "del" | "erase" => {
            shape(cx, "rm", "", args);
            let a = split(args, &[]);
            let slash = |s: &str| name != "rm" && args.iter().any(|w| w.exact && w.text.eq_ignore_ascii_case(s));
            let recursive = a.short('r') || a.short('R') || a.long("--recursive") || slash("/s");
            let force = a.short('f') || a.long("--force") || slash("/f") || slash("/q");
            if a.long("--no-preserve-root") {
                cx.b.find(Flag::Delete, Risk::Risky, "turns off the safety that protects the root folder".to_string());
            }
            // -f alone only skips questions; on one named file it takes nothing extra with it.
            let _ = force;
            for p in a.pos.iter().filter(|p| !(p.exact && p.text.starts_with("/") && p.text.len() == 2 && name != "rm")) {
                delete(cx, &cwd, p, recursive);
            }
        }
        "rmdir" | "rd" | "unlink" => {
            shape(cx, &name, "", args);
            let a = split(args, &[]);
            let recursive = a.opts.iter().any(|o| o.eq_ignore_ascii_case("/s")) || a.pos.iter().any(|p| p.exact && p.text.eq_ignore_ascii_case("/s"));
            for p in a.pos.iter().filter(|p| !(p.exact && p.text.len() == 2 && p.text.starts_with('/'))) {
                delete(cx, &cwd, p, recursive);
            }
        }
        "shred" | "truncate" | "wipe" | "srm" => {
            shape(cx, &name, "", args);
            let a = split(args, &["-s", "--size", "-n", "--iterations", "-r", "--reference"]);
            for p in &a.pos {
                delete(cx, &cwd, p, true);
            }
        }
        "mv" | "move" | "ren" | "rename" => {
            shape(cx, "mv", "", args);
            let target = value_of(args, &["-t", "--target-directory"]);
            let a = split(args, &["-t", "--target-directory", "-S", "--suffix"]);
            let (sources, dest) = match target {
                Some(t) => (a.pos.clone(), Some(t)),
                None => match a.pos.split_last() {
                    Some((d, s)) if !s.is_empty() => (s.to_vec(), Some(d.clone())),
                    _ => (a.pos.clone(), None),
                },
            };
            for s in &sources {
                move_from(cx, &cwd, s);
            }
            if let Some(d) = dest {
                // Moving project files somewhere else takes them out of the project.
                let to = zone_of(cx, &cwd, &d);
                let project = paths::norm(&cx.ctx.project);
                let dest_paths = if d.exact { paths::resolve(&d.text, &cwd, cx.ctx) } else { vec![] };
                let inside = !dest_paths.is_empty() && dest_paths.iter().all(|p| paths::within(p, &project));
                let stays = inside || matches!(to, Some(Zone::Project | Zone::ProjectRoot | Zone::GitInternals));
                for s in &sources {
                    if matches!(zone_of(cx, &cwd, s), Some(Zone::Project | Zone::ProjectRoot)) && !stays {
                        cx.b.find(Flag::Delete, Risk::Risky, format!("moves {} out of the project", short(&s.text)));
                    }
                }
                let how = if a.short('n') || a.long("--no-clobber") { Write::Create } else { Write::Truncate };
                write(cx, &cwd, &d, how);
                carry(cx, &cwd, &sources, &d);
            }
        }
        "cp" | "copy" | "xcopy" | "robocopy" | "install" | "ditto" => {
            shape(cx, "cp", "", args);
            let target = value_of(args, &["-t", "--target-directory"]);
            let a = split(args, &["-t", "--target-directory", "-S", "--suffix", "-m", "--mode", "-o", "--owner", "-g", "--group"]);
            let (sources, dest) = match target {
                Some(t) => (a.pos.clone(), Some(t)),
                None => match a.pos.split_last() {
                    Some((d, s)) if !s.is_empty() => (s.to_vec(), Some(d.clone())),
                    _ => (a.pos.clone(), None),
                },
            };
            for s in &sources {
                read(cx, &cwd, s);
            }
            if let Some(d) = dest {
                let how = if a.short('n') || a.long("--no-clobber") { Write::Create } else { Write::Truncate };
                write(cx, &cwd, &d, how);
                carry(cx, &cwd, &sources, &d);
            }
            if name == "robocopy" && args.iter().any(|w| w.exact && matches!(w.text.to_lowercase().as_str(), "/mir" | "/purge" | "/move" | "/mov")) {
                cx.b.find(Flag::Delete, Risk::Risky, "deletes files at the destination or source to make them match".to_string());
            }
        }
        "ln" | "mklink" => {
            shape(cx, "ln", "", args);
            let a = split(args, &["-t", "--target-directory"]);
            let link = if name == "mklink" { a.pos.first() } else { a.pos.last() };
            if let Some(d) = link {
                write(cx, &cwd, d, if a.short('f') { Write::Truncate } else { Write::Create });
            }
        }
        "touch" | "mkdir" | "md" => {
            shape(cx, &name, "", args);
            let a = split(args, &["-m", "--mode", "-d", "--date", "-r", "--reference", "-t"]);
            for p in &a.pos {
                write(cx, &cwd, p, Write::Create);
            }
        }
        "tee" | "sponge" => {
            shape(cx, &name, "", args);
            let a = split(args, &[]);
            let how = if a.short('a') || a.long("--append") { Write::Append } else { Write::Truncate };
            for p in &a.pos {
                write(cx, &cwd, p, how);
                if inv.stdin_text.is_none() {
                    created(cx, &cwd, p, None);
                }
            }
        }
        "dd" => {
            shape(cx, "dd", "", args);
            // `dd if=/dev/zero of=/tmp/test.bin` makes a test file; writing to a device or
            // anywhere else can wipe a disk or a file.
            let target = args.iter().find_map(|w| w.text.strip_prefix("of=").map(|p| Word { text: p.to_string(), ..w.clone() }));
            let routine = target.as_ref().is_some_and(|t| !t.text.starts_with("/dev/") && matches!(zone_of_text(cx, &cwd, t), Some(Zone::Temp | Zone::Project)));
            if !routine {
                cx.b.find(Flag::Overwrite, Risk::Risky, "copies raw data and can overwrite a file or a whole disk (dd)".to_string());
            }
            for w in args {
                if let Some(p) = w.text.strip_prefix("of=") {
                    write(cx, &cwd, &Word { text: p.to_string(), ..w.clone() }, Write::Truncate);
                }
                if let Some(p) = w.text.strip_prefix("if=") {
                    read(cx, &cwd, &Word { text: p.to_string(), ..w.clone() });
                }
            }
        }
        "chmod" | "chown" | "chgrp" | "icacls" | "takeown" | "cacls" | "attrib" | "setfacl" | "chflags" | "xattr" => {
            shape(cx, &name, "", args);
            let a = split(args, &["--reference"]);
            let recursive = a.short('R') || a.long("--recursive") || args.iter().any(|w| w.exact && matches!(w.text.to_lowercase().as_str(), "/t" | "/r" | "/s"));
            let mode = a.pos.first().map(|m| m.text.clone()).unwrap_or_default();
            let changes = match name.as_str() {
                "icacls" | "cacls" => args.iter().any(|w| w.exact && w.text.starts_with('/') && !matches!(w.text.to_lowercase().as_str(), "/t" | "/c" | "/q" | "/l")),
                "attrib" => args.iter().any(|w| w.exact && (w.text.starts_with('+') || (w.text.starts_with('-') && w.text.len() == 2))),
                _ => true,
            };
            if !changes {
                return;
            }
            if name == "chmod" && (mode.contains("+s") || mode.contains("u+s") || mode.contains("g+s") || (mode.len() == 4 && mode.starts_with(['2', '4', '6', '7']) && mode.chars().all(|c| c.is_ascii_digit()))) {
                cx.b.find(Flag::Privilege, Risk::Risky, "makes a file run with its owner's rights (setuid)".to_string());
            }
            if matches!(name.as_str(), "takeown" | "icacls" | "cacls") {
                cx.b.find(Flag::Privilege, Risk::Risky, "changes who may access files".to_string());
            }
            let targets: Vec<Word> = if matches!(name.as_str(), "chmod" | "chown" | "chgrp") { a.pos.iter().skip(1).cloned().collect() } else { a.pos.iter().filter(|p| !p.text.starts_with('/') || p.text.len() > 3).filter(|p| !p.text.starts_with('+')).cloned().collect() };
            for t in &targets {
                match zone_of(cx, &cwd, t) {
                    Some(Zone::Temp | Zone::Project) => cx.b.find(Flag::WritesFile, Risk::Caution, format!("changes permissions on {}", short(&t.text))),
                    Some(Zone::ProjectRoot) if !recursive => cx.b.find(Flag::WritesFile, Risk::Caution, "changes permissions on the project folder".to_string()),
                    Some(Zone::Sink) => {}
                    _ => cx.b.find(Flag::SystemChange, Risk::Risky, format!("changes permissions on {}{}", short(&t.text), if recursive { " and everything in it" } else { "" })),
                }
            }
        }
        "sed" | "gsed" => tools::sed(cx, inv),
        "awk" | "gawk" | "mawk" | "nawk" => tools::awk(cx, inv),
        "find" | "gfind" | "fd" | "fdfind" => tools::find(cx, inv),
        "tar" | "bsdtar" | "gtar" => tools::tar(cx, inv),
        "unzip" | "zip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz" | "zstd" | "unzstd" | "7z" | "7za" | "7zr" | "unrar" | "rar" | "expand-archive" => tools::archive(cx, inv, &name),
        "patch" => {
            shape(cx, "patch", "", args);
            cx.b.find(Flag::WritesFile, Risk::Caution, "applies a patch to files in the current folder".to_string());
            if !split(args, &[]).short('R') && args.iter().any(|w| w.exact && w.text == "--dry-run") {
                // dry run: nothing written
            }
        }
        "sort" => {
            shape(cx, "sort", "", args);
            if let Some(o) = value_of(args, &["-o", "--output"]) {
                write(cx, &cwd, &o, Write::Truncate);
            }
            for p in &split(args, &["-o", "--output", "-k", "-t", "-T", "-S"]).pos {
                read(cx, &cwd, p);
            }
        }

        // ---- network
        "curl" => tools::curl(cx, inv),
        "wget" | "wget2" | "aria2c" | "http" | "https" | "xh" | "httpie" => tools::wget(cx, inv, &name),
        "ssh" | "scp" | "sftp" | "rsync" | "rclone" | "mosh" | "ssh-copy-id" => tools::ssh_family(cx, inv, &name),
        "nc" | "ncat" | "netcat" | "socat" | "telnet" | "ftp" | "tftp" | "lftp" | "openssl" => tools::raw_net(cx, inv, &name),
        "ping" | "nslookup" | "dig" | "host" | "whois" | "nmap" | "masscan" | "test-netconnection" => {
            shape(cx, &name, "", args);
            let a = split(args, &["-c", "-n", "-w", "-W", "-i", "-t", "-p", "-s"]);
            network(cx, a.pos.first(), "contacts");
            if matches!(name.as_str(), "nmap" | "masscan") {
                cx.b.find(Flag::Network, Risk::Risky, "scans other machines on the network".to_string());
            }
        }

        // ---- processes
        "kill" | "pkill" | "killall" | "taskkill" | "stop-process" | "skill" | "xkill" => {
            shape(cx, &name, "", args);
            // `kill -9 -1` (every process) and `kill 1` (the first process).
            let everything = name == "kill" && args.last().is_some_and(|l| l.exact && (l.text == "1" || (l.text == "-1" && args.len() >= 2)));
            if everything {
                cx.b.find(Flag::KillsProcess, Risk::Risky, "stops every process it is allowed to stop".to_string());
            } else {
                cx.b.find(Flag::KillsProcess, Risk::Caution, "stops running programs".to_string());
            }
        }

        // ---- developer tools with their own rules
        "git" => tools::git(cx, inv),
        "npm" | "pnpm" | "yarn" | "bun" | "npx" | "pnpx" | "bunx" | "corepack" => tools::node_pm(cx, inv, &name),
        "node" | "nodejs" | "deno" => tools::node(cx, inv, &name),
        "python" | "python3" | "python2" | "py" | "pypy" | "pypy3" | "ipython" => tools::python(cx, inv, &name),
        "perl" | "ruby" | "php" | "lua" | "tclsh" | "osascript" | "rscript" | "julia" | "groovy" => {
            tools::script_runner(cx, inv, &name);
        }
        "pip" | "pip3" | "pipx" | "uv" | "poetry" | "conda" | "mamba" | "pdm" | "easy_install" => tools::python_pm(cx, inv, &name),
        "cargo" | "rustup" | "go" => tools::rust_go(cx, inv, &name),
        "docker" | "podman" | "nerdctl" | "docker-compose" => tools::docker(cx, inv, &name),
        "psql" | "mysql" | "mariadb" | "sqlite3" | "sqlcmd" | "mongosh" | "mongo" | "redis-cli" | "duckdb" | "clickhouse-client" | "pg_dump" | "pg_restore" | "mysqldump" | "dropdb" | "createdb" | "pg_ctl" | "cqlsh" | "bq" => tools::sql_client(cx, inv, &name),
        "gh" | "glab" | "hub" | "kubectl" | "oc" | "helm" | "terraform" | "tofu" | "pulumi" | "aws" | "gcloud" | "gsutil" | "az" | "doctl" | "flyctl" | "fly" | "heroku" | "vercel" | "netlify" | "firebase" | "wrangler" | "supabase" | "railway" | "render" | "eas" | "eas-cli" | "stripe" | "twilio" | "sentry-cli" | "ngrok" | "cloudflared" | "ansible" | "ansible-playbook" | "vagrant" | "packer" | "linode-cli" | "scw" | "ibmcloud" | "oci" | "cdk" | "sam" | "serverless" | "sls" | "amplify" | "neonctl" | "turso" | "planetscale" | "pscale" | "surge" | "gh-pages" => tools::cloud(cx, inv, &name),
        "apt" | "apt-get" | "aptitude" | "dnf" | "yum" | "pacman" | "zypper" | "apk" | "brew" | "port" | "snap" | "flatpak" | "winget" | "choco" | "scoop" | "nix-env" | "nix" | "dpkg" | "rpm" | "gem" | "install-module" | "msiexec" => tools::system_pm(cx, inv, &name),

        // ---- changes to the machine
        "shutdown" | "reboot" | "halt" | "poweroff" | "init" | "logoff" | "stop-computer" | "restart-computer" | "mkfs" | "fdisk" | "parted" | "wipefs" | "mkswap" | "sfdisk" | "gdisk" | "diskpart" | "format" | "bcdedit" | "fsutil" | "netsh" | "mount" | "umount" | "losetup" | "swapoff" | "swapon" | "iptables" | "ip6tables" | "nft" | "ufw" | "firewall-cmd" | "setenforce" | "chattr" | "useradd" | "userdel" | "usermod" | "passwd" | "groupadd" | "groupdel" | "visudo" | "chsh" | "modprobe" | "insmod" | "rmmod" | "dmsetup" | "cryptsetup" | "setcap" | "at" | "batch" | "launchctl" | "systemctl" | "service" | "sc" | "reg" | "regedit" | "net" | "net1" | "schtasks" | "crontab" | "wmic" | "setx" | "cipher" | "vssadmin" | "wevtutil" | "rundll32" | "mshta" | "regsvr32" | "wscript" | "cscript" | "bitsadmin" | "certutil" | "installutil" | "regasm" | "odbcconf" | "forfiles" | "pcalua" | "scriptrunner" | "sysctl" | "defaults" | "csrutil" | "spctl" | "dscl" | "nvram" | "diskutil" | "hdiutil" | "tccutil" | "security" | "cmdkey" | "vaultcmd" | "su" | "newgrp" | "sudoedit" | "update-alternatives" | "ldconfig" | "gpg" | "ssh-keygen" | "ssh-add" | "keytool" | "secedit" | "gpupdate" | "dism" | "sfc" | "chkdsk" | "defrag" | "powercfg" | "tzutil" | "w32tm" | "pnputil" | "devcon" | "bootrec" | "mountvol" | "subst" | "compact" | "manage-bde" => tools::system(cx, inv, &name),

        "start" | "open" | "xdg-open" | "explorer" | "cygstart" | "invoke-item" | "gio" => {
            shape(cx, &name, "", args);
            let a = split(args, &[]);
            match a.pos.iter().find(|p| !p.text.is_empty()) {
                Some(p) if looks_like_url(p) => {
                    network(cx, Some(p), "opens a page on");
                }
                Some(p) => cx.b.find(Flag::RunsCode, Risk::Caution, format!("opens {} with its default program", short(&p.text))),
                None => {}
            }
        }

        _ if name.starts_with("mkfs.") || name.starts_with("fsck") => tools::system(cx, inv, "mkfs"),
        _ if SAFE.contains(&name.as_str()) && !has_path => {
            shape(cx, &name, "", args);
            if matches!(name.as_str(), "printenv" | "env") || (matches!(name.as_str(), "set" | "export" | "declare") && args.iter().all(|a| a.exact && a.text.starts_with('-'))) {
                Builder::push_unique(&mut cx.b.secret_reads, "environment variables");
            }
            if !matches!(name.as_str(), "echo" | "printf" | "test" | "[" | "[[" | "export" | "declare" | "local" | "typeset" | "readonly" | "set" | "cd" | "pushd" | "which" | "type" | "sleep" | "seq" | "expr" | "read" | "let" | "unset"
                // These look at names, sizes and dates, not at what is inside a file.
                | "ls" | "dir" | "stat" | "du" | "df" | "file" | "tree" | "realpath" | "readlink" | "basename" | "dirname" | "cygpath" | "wslpath") {
                for p in args.iter().filter(|w| !is_opt(w)) {
                    read(cx, &cwd, p);
                }
            }
            if matches!(name.as_str(), "base64" | "xxd") && inv.stdin_text.is_none() {
                // Decoding is only a finding when the result is run: see `shell`.
            }
        }
        _ if only_help => shape(cx, &name, "", args),
        _ if DEV_TOOLS.contains(&name.as_str()) => {
            let a = split(args, &[]);
            let sub = a.pos.first().filter(|p| p.exact && p.text.len() < 24 && !p.text.contains(['/', '\\', '.'])).map(|p| p.text.clone()).unwrap_or_default();
            shape(cx, &name, &sub, args);
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs {name}, a developer tool, on this project"));
            if matches!(name.as_str(), "prettier" | "eslint" | "black" | "ruff" | "rustfmt" | "isort" | "biome" | "stylelint") && a.any(&["--write", "--fix", "-w", "--in-place", "-i"]) {
                cx.b.find(Flag::WritesFile, Risk::Caution, "rewrites source files in place".to_string());
            }
            for o in ["-o", "--output", "--out", "--outfile", "--outDir", "--out-dir", "-d"] {
                if let Some(v) = value_of(args, &[o]) {
                    if matches!(name.as_str(), "ffmpeg" | "magick" | "convert" | "pandoc" | "gcc" | "g++" | "cc" | "clang" | "clang++" | "rustc" | "tsc" | "esbuild" | "sass") {
                        write(cx, &cwd, &v, Write::Truncate);
                    }
                }
            }
        }
        _ => {
            shape(cx, &name, "", args);
            if has_path || name.ends_with(".sh") || name.ends_with(".py") || name.ends_with(".js") || name.ends_with(".ps1") {
                read(cx, &cwd, &argv[0]);
                cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the program or script {}", short(&argv[0].text)));
                runs_file(cx, &cwd, &argv[0], RunKind::Other);
            } else {
                cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “{name}”, a program Atoll does not know"));
            }
        }
    }
}

// ---------------------------------------------------------------- shells

const FETCHERS: &[&str] = &["curl", "wget", "nc", "ncat", "netcat", "iwr", "invoke-webrequest", "irm", "invoke-restmethod", "fetch", "aria2c", "http", "ftp", "scp", "ssh", "socat", "openssl", "telnet", "dig", "nslookup"];
const DECODERS: &[&str] = &["base64", "xxd", "openssl", "gunzip", "gzip", "zcat", "bzcat", "xz", "rev", "tr", "uudecode", "basenc", "base32", "od", "printf", "certutil", "zstd", "unzip", "funzip"];

fn upstream_kinds(inv: &Invocation) -> (bool, bool) {
    let names: Vec<String> = inv.upstream.iter().map(|u| prog_name(u).0).collect();
    (names.iter().any(|n| FETCHERS.contains(&n.as_str())), names.iter().any(|n| DECODERS.contains(&n.as_str())))
}

/// A program that runs whatever arrives on its input.
pub(crate) fn reads_code_from_stdin(cx: &mut Cx, inv: &Invocation, what: &str) {
    let (fetch, decode) = upstream_kinds(inv);
    cx.b.find(Flag::PipeToShell, Risk::Risky, format!("feeds another command's output straight into {what}, which runs it"));
    if fetch {
        cx.b.find(Flag::RemoteCode, Risk::Risky, "downloads something and runs it without showing it".to_string());
    }
    if decode {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "decodes hidden text and runs it".to_string());
    }
}

fn shell(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    // Find `-c` (also bundled: -lc, -ec).
    let mut i = 0;
    let mut script: Option<&Word> = None;
    let mut command: Option<&Word> = None;
    let mut reads_stdin = false;
    while i < args.len() {
        let a = &args[i];
        if a.exact && a.text.starts_with('-') && !a.text.starts_with("--") && a.text.len() > 1 {
            if a.text[1..].contains('c') {
                command = args.get(i + 1);
                break;
            }
            if a.text[1..].contains('s') {
                reads_stdin = true;
            }
            if matches!(a.text.as_str(), "-o" | "-O") {
                i += 1;
            }
        } else if a.exact && a.text.starts_with("--") {
            if matches!(a.text.as_str(), "--rcfile" | "--init-file") {
                cx.b.find(Flag::HiddenExec, Risk::Risky, "starts a shell with a custom startup file".to_string());
                i += 1;
            }
        } else if a.exact && a.text == "-" {
            reads_stdin = true;
        } else {
            script = Some(a);
            break;
        }
        i += 1;
    }
    if let Some(c) = command {
        if c.exact {
            crate::nested_bash(cx, &c.text.clone(), &cwd);
        } else if c.subst {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("runs text produced by another command as a {name} script"));
        } else {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("runs a {name} command that is only known when it runs"));
        }
        return;
    }
    if let Some(s) = script {
        if s.subst || !s.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs a script produced by another command".to_string());
        } else {
            read(cx, &cwd, s);
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(&s.text)));
            runs_file(cx, &cwd, s, RunKind::Shell);
        }
        return;
    }
    if let Some(text) = &inv.stdin_text {
        crate::nested_bash(cx, &text.clone(), &cwd);
        return;
    }
    if inv.stdin_piped {
        reads_code_from_stdin(cx, inv, name);
        return;
    }
    if reads_stdin || inv.redirects.iter().any(|r| r.op.starts_with('<')) {
        cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs a script read from a file ({name})"));
        return;
    }
    if inv.redirects.iter().any(|r| r.target.text.starts_with("/dev/tcp/") || r.target.text.starts_with("/dev/udp/")) || args.iter().any(|a| a.exact && a.text == "-i") {
        cx.b.find(Flag::HiddenExec, Risk::Risky, format!("starts an interactive {name} that something else can control"));
        return;
    }
    cx.b.find(Flag::RunsCode, Risk::Caution, format!("starts a {name} shell"));
}

/// PowerShell text handed over by bash (`powershell -Command "..."`). Where bash fills in a
/// variable first, the text is still checked as written, with a note. Text produced by
/// another command cannot be checked at all.
fn ps_text(cx: &mut Cx, inv: &Invocation, words: &[Word], cwd: &Cwd) {
    if words.is_empty() {
        cx.b.find(Flag::RunsCode, Risk::Caution, "starts PowerShell".to_string());
        return;
    }
    if words.iter().any(|w| w.subst) {
        cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs PowerShell text built by another command".to_string());
        return;
    }
    let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    if text.trim() == "-" {
        reads_code_from_stdin(cx, inv, "PowerShell");
        return;
    }
    if words.iter().any(|w| !w.exact) {
        cx.b.find(Flag::DynamicCommand, Risk::Caution, "bash fills in part of the PowerShell text before it runs".to_string());
    }
    crate::powershell::analyze_into(cx, &text, cwd);
}

fn powershell_exe(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "powershell", "", &[]);
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let lower = a.text.to_lowercase();
        if !a.exact || !lower.starts_with('-') {
            // First plain word starts the command text.
            ps_text(cx, inv, &args[i..], &cwd);
            return;
        }
        let key = lower.trim_start_matches('-');
        if !key.is_empty() && ("encodedcommand".starts_with(key) && key.len() <= 14 && (key.starts_with('e') && !key.starts_with("ex"))) && key != "executionpolicy" {
            cx.b.find(Flag::Obfuscated, Risk::Risky, "runs a PowerShell command hidden in encoded text".to_string());
            return;
        }
        if "command".starts_with(key) && !key.is_empty() {
            ps_text(cx, inv, &args[i + 1..], &cwd);
            return;
        }
        if "file".starts_with(key) && !key.is_empty() {
            match args.get(i + 1) {
                Some(f) if f.exact => {
                    read(cx, &cwd, f);
                    cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the PowerShell script {}", short(&f.text)));
                    runs_file(cx, &cwd, f, RunKind::Other);
                }
                _ => cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs a PowerShell script that is only known when it runs".to_string()),
            }
            return;
        }
        if "windowstyle".starts_with(key) && key.len() >= 1 && args.get(i + 1).is_some_and(|v| v.text.eq_ignore_ascii_case("hidden")) {
            cx.b.find(Flag::Background, Risk::Caution, "runs in a hidden window".to_string());
        }
        if matches!(key, "executionpolicy" | "ep" | "ex" | "windowstyle" | "w" | "wi" | "win" | "inputformat" | "outputformat" | "configurationname" | "workingdirectory" | "wd" | "version" | "psconsolefile" | "settingsfile" | "customppipename") {
            i += 1;
        }
        i += 1;
    }
    if inv.stdin_piped {
        reads_code_from_stdin(cx, inv, "PowerShell");
    } else if let Some(text) = &inv.stdin_text {
        crate::powershell::analyze_into(cx, &text.clone(), &cwd);
    } else {
        cx.b.find(Flag::RunsCode, Risk::Caution, format!("starts {name}"));
    }
}

static CMD_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%[A-Za-z_~][^%\s]*%|![A-Za-z_][A-Za-z0-9_]*!|%%?[A-Za-z]\b").unwrap());

/// `cmd /c "..."`: split the line the way cmd does (roughly) and classify each part.
fn cmd_exe(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "cmd", "", &[]);
    let at = args.iter().position(|a| a.exact && matches!(a.text.to_lowercase().as_str(), "/c" | "//c" | "/k" | "//k" | "-c"));
    let Some(at) = at else {
        if inv.stdin_piped {
            reads_code_from_stdin(cx, inv, "cmd");
        } else {
            cx.b.find(Flag::RunsCode, Risk::Caution, "starts a Windows command prompt".to_string());
        }
        return;
    };
    let rest = &args[at + 1..];
    if rest.iter().any(|w| !w.exact) {
        cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs a cmd command that is only known when it runs".to_string());
        return;
    }
    let line = rest.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    cmd_line(cx, &line, &cwd);
}

pub(crate) fn cmd_line(cx: &mut Cx, line: &str, cwd: &Cwd) {
    if cx.depth > 4 {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "commands are nested unusually deep".to_string());
        return;
    }
    if line.contains('^') && line.chars().filter(|c| *c == '^').count() >= 2 {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "uses cmd escape characters to disguise a command".to_string());
    }
    // Split into simple commands on & && | || outside quotes.
    let mut parts: Vec<(String, bool)> = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut piped = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '^' if !quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '&' | '|' if !quoted => {
                let was_pipe = c == '|' && chars.peek() != Some(&'|');
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                parts.push((std::mem::take(&mut cur), piped));
                piped = was_pipe;
            }
            '(' | ')' if !quoted => {
                parts.push((std::mem::take(&mut cur), piped));
                piped = false;
            }
            _ => cur.push(c),
        }
    }
    parts.push((cur, piped));
    let mut upstream: Vec<String> = vec![];
    for (part, piped) in parts {
        let mut words: Vec<Word> = vec![];
        let mut redirects = vec![];
        let mut tok = String::new();
        let mut q = false;
        let mut pending_redirect: Option<String> = None;
        let flush = |tok: &mut String, words: &mut Vec<Word>, redirects: &mut Vec<crate::bash::Redirect>, pending: &mut Option<String>| {
            if tok.is_empty() {
                return;
            }
            let exact = !CMD_VAR.is_match(tok);
            let w = Word { text: std::mem::take(tok), exact, ..Default::default() };
            match pending.take() {
                Some(op) => redirects.push(crate::bash::Redirect { op, target: w }),
                None => words.push(w),
            }
        };
        let mut it = part.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '"' => q = !q,
                ' ' | '\t' if !q => flush(&mut tok, &mut words, &mut redirects, &mut pending_redirect),
                '>' | '<' if !q => {
                    // `2>&1` and `2>nul`
                    if tok.chars().all(|d| d.is_ascii_digit()) {
                        tok.clear();
                    }
                    flush(&mut tok, &mut words, &mut redirects, &mut pending_redirect);
                    let mut op = c.to_string();
                    if it.peek() == Some(&'>') {
                        it.next();
                        op.push('>');
                    }
                    if it.peek() == Some(&'&') {
                        it.next();
                        it.next();
                        continue;
                    }
                    pending_redirect = Some(op);
                }
                _ => tok.push(c),
            }
        }
        flush(&mut tok, &mut words, &mut redirects, &mut pending_redirect);
        if words.is_empty() && redirects.is_empty() {
            continue;
        }
        // cmd built-ins that are not programs.
        let head = words.first().map(|w| w.text.to_lowercase()).unwrap_or_default();
        let head = head.trim_start_matches('@').to_string();
        let name = prog_name(&head).0;
        let inv = Invocation { argv: words.clone(), redirects, cwd: cwd.clone(), stdin_piped: piped, upstream: upstream.clone(), ..Default::default() };
        upstream.push(name.clone());
        match name.as_str() {
            "echo" | "echo." | "rem" | "set" | "setlocal" | "endlocal" | "cd" | "chdir" | "pushd" | "popd" | "type" | "dir" | "cls" | "ver" | "vol" | "title" | "pause" | "exit" | "if" | "for" | "goto" | "call" | "do" | "else" | "path" | "prompt" | "color" | "assoc" | "ftype" | "break" | "verify" | "date" | "time" => {
                for r in &inv.redirects {
                    write(cx, cwd, &r.target, if r.op == ">>" { Write::Append } else { Write::Truncate });
                }
                if matches!(name.as_str(), "call" | "if" | "for" | "do" | "else") && words.len() > 1 {
                    // The part after the keyword is a command too, but its shape varies: treat as unknown.
                    cx.b.find(Flag::RunsCode, Risk::Caution, "runs a cmd batch command".to_string());
                    let tail = words[1..].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ").to_lowercase();
                    if RAW_DANGER.is_match(&tail) {
                        cx.b.find(Flag::Delete, Risk::Risky, "contains a command that can delete or overwrite work".to_string());
                    }
                }
                // `assoc .ics` only shows the association; `assoc .ics=x` changes it.
                if matches!(name.as_str(), "assoc" | "ftype" | "path") && words.len() > 1 && (name == "path" || words[1..].iter().any(|w| w.text.contains('='))) {
                    cx.b.find(Flag::SystemChange, Risk::Risky, "changes how Windows finds or opens programs".to_string());
                }
            }
            _ => {
                // cmd uses /x switches: make them look like options so paths are not mistaken.
                let mut inv = inv;
                inv.argv[0].text = head.clone();
                invocation(cx, &inv);
            }
        }
    }
}

/// Text patterns that mean "can delete or overwrite work", used when the
/// command cannot be parsed and as a floor for opaque text (batch, heredocs).
pub(crate) static RAW_DANGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\brm\s+-\w*[rf]|\bdel\s+/[sqf]|\b(rd|rmdir)\s+/s|remove-item\b[^|;]*-(recurse|force)|git\s+reset\s+--hard|git\s+push\b[^|;]*(-f\b|--force)",
        r"|git\s+clean\s+-\w*f|git\s+checkout\s+--\s|git\s+branch\s+-D|drop\s+(table|database)|truncate\s+table|\bformat\s+[a-z]:|\bmkfs|\bdd\s+if=",
        r"|stop-computer|\bshutdown\b|\|\s*(sudo\s+)?(ba|z|da)?sh\b|\biex\b|invoke-expression|\bsudo\b|\beval\b|base64\s+(-d|--decode)|-encodedcommand|\bchmod\s+-R|>\s*/dev/sd|:\(\)\s*\{",
    ))
    .unwrap()
});
