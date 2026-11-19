//! Requests that are not shell commands: file edits, reads, web fetches,
//! sub-agents and connector (MCP) tools.

use std::time::Instant;

use serde_json::Value;

use crate::bash::Word;
use crate::classify::{self, Cx, Write};
use crate::diff::Diff;
use crate::model::{Analysis, Builder, Context, Flag, Risk, Shell};
use crate::{paths, summary};

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn file_name(path: &str) -> String {
    path.replace('\\', "/").rsplit('/').next().unwrap_or(path).to_string()
}

fn lines(n: usize) -> String {
    if n == 1 { "1 line".into() } else { format!("{n} lines") }
}

/// Analyse any tool request. `existing` is the current text of the file a Write
/// would replace (None when it does not exist or could not be read).
/// Never panics: a failure comes back as `unknown`.
pub fn analyze_tool(tool: &str, input: &Value, ctx: &Context, existing: Option<&str>) -> Analysis {
    if matches!(tool, "Bash" | "PowerShell") {
        return crate::analyze(tool, s(input, "command"), ctx);
    }
    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(tool, input, ctx, existing)));
    let mut a = match result {
        Ok(a) => a,
        Err(_) => {
            let mut b = Builder::default();
            b.find(Flag::Unparsed, Risk::Unknown, "the analysis failed on this request".to_string());
            let mut a = crate::finish(b, Shell::Tool);
            a.summary = "Atoll could not check this request.".into();
            a
        }
    };
    a.elapsed_us = start.elapsed().as_micros() as u64;
    a
}

fn run(tool: &str, input: &Value, ctx: &Context, existing: Option<&str>) -> Analysis {
    let mut b = Builder { parsed: true, ..Default::default() };
    let cwd = if ctx.cwd.is_empty() { None } else { Some(vec![paths::norm(&ctx.cwd)]) };
    let mut diff: Option<Diff> = None;
    let sentence: String;
    {
        let mut cx = Cx { ctx, b: &mut b, depth: 0, functions: vec![] };
        match tool {
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
                let path = [s(input, "file_path"), s(input, "notebook_path"), s(input, "path")].into_iter().find(|p| !p.is_empty()).unwrap_or("");
                classify::shape(&mut cx, tool, "", &[]);
                if path.is_empty() {
                    cx.b.find(Flag::UnresolvedPath, Risk::Unknown, "Atoll could not tell which file this changes".to_string());
                    sentence = "Changes a file Atoll could not identify.".into();
                } else {
                    let how = if tool == "Write" { Write::Truncate } else { Write::Append };
                    classify::write(&mut cx, &cwd, &Word::lit(path), how);
                    let mut d = Diff::new(path);
                    match tool {
                        "Edit" => d.add_change(s(input, "old_string"), s(input, "new_string")),
                        "MultiEdit" => {
                            for e in input.get("edits").and_then(Value::as_array).into_iter().flatten() {
                                d.add_change(s(e, "old_string"), s(e, "new_string"));
                            }
                        }
                        "Write" => match existing {
                            Some(old) => d.add_change(old, s(input, "content")),
                            None => d.add_new_file(s(input, "content")),
                        },
                        _ => {
                            let new = s(input, "new_source");
                            if s(input, "edit_mode") == "delete" {
                                d.add_change(new, "");
                            } else {
                                d.add_new_file(new);
                            }
                        }
                    }
                    let name = file_name(path);
                    // Line counts are in the diff's header; the sentence names the action.
                    sentence = if d.new_file && tool == "Write" {
                        format!("Creates {name}, a new file of {}.", lines(d.added))
                    } else if tool == "Write" {
                        format!("Replaces the whole of {name}.")
                    } else if tool == "MultiEdit" {
                        let n = input.get("edits").and_then(Value::as_array).map_or(0, |e| e.len());
                        format!("Makes {n} edits to {name}.")
                    } else {
                        format!("Edits {name}.")
                    };
                    if d.cut {
                        cx.b.find(Flag::Truncated, Risk::Caution, "the change reached Atoll cut short, so the end is not shown".to_string());
                    }
                    diff = Some(d);
                }
            }
            "Read" => {
                let path = s(input, "file_path");
                classify::shape(&mut cx, "Read", "", &[]);
                classify::read(&mut cx, &cwd, &Word::lit(path));
                sentence = format!("Reads {}.", file_name(path));
            }
            "Glob" | "Grep" | "LS" => {
                classify::shape(&mut cx, tool, "", &[]);
                let what = [s(input, "pattern"), s(input, "path")].into_iter().find(|p| !p.is_empty()).unwrap_or("files");
                sentence = format!("Searches files for “{}”; changes nothing.", classify::short(what));
            }
            "WebFetch" => {
                let url = s(input, "url");
                classify::shape(&mut cx, "WebFetch", "", &[]);
                classify::network(&mut cx, Some(&Word::lit(url)), "fetches a page from");
                sentence = match classify::host_of(url) {
                    Some(h) => format!("Fetches a page from {h}."),
                    None => "Fetches a web page.".into(),
                };
            }
            "WebSearch" => {
                classify::shape(&mut cx, "WebSearch", "", &[]);
                cx.b.find(Flag::Network, Risk::Caution, "searches the web".to_string());
                sentence = format!("Searches the web for “{}”.", classify::short(s(input, "query")));
            }
            "Task" | "Agent" => {
                classify::shape(&mut cx, "Agent", "", &[]);
                cx.b.find(Flag::RunsCode, Risk::Caution, "starts a sub-agent that asks for its own permissions".to_string());
                let d = s(input, "description");
                sentence = if d.is_empty() { "Starts a sub-agent.".into() } else { format!("Starts a sub-agent to {}.", d.trim_end_matches('.').to_lowercase()) };
            }
            "ExitPlanMode" => {
                classify::shape(&mut cx, tool, "", &[]);
                cx.b.find(Flag::RunsCode, Risk::Caution, "leaves plan mode and starts making changes".to_string());
                sentence = "Asks to leave plan mode and start working.".into();
            }
            t if t.starts_with("mcp__") => {
                let mut parts = t.splitn(3, "__").skip(1);
                let server = parts.next().unwrap_or("");
                let name = parts.next().unwrap_or(t).replace('_', " ");
                // Connector ids are often random: say "a connector" rather than a hash.
                let readable = !server.is_empty() && server.len() <= 24 && !server.chars().filter(|c| c.is_ascii_hexdigit()).count().eq(&server.len());
                classify::shape(&mut cx, "mcp", &name, &[]);
                cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("uses “{name}” from {}", if readable { format!("the {server} connector") } else { "a connector".into() }));
                sentence = format!("Uses “{name}” from {}.", if readable { format!("the {server} connector") } else { "a connector".into() });
            }
            other => {
                classify::shape(&mut cx, other, "", &[]);
                cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("uses the {other} tool"));
                sentence = format!("Uses the {other} tool.");
            }
        }
    }
    let mut a = crate::finish(b, Shell::Tool);
    // A risky finding leads; otherwise the tool's own sentence says it plainest.
    a.summary = if a.risk >= Risk::Unknown {
        let notes: Vec<&str> = a.findings.iter().filter(|f| f.risk >= Risk::Unknown).map(|f| f.note.as_str()).take(2).collect();
        summary::join(&notes)
    } else {
        sentence
    };
    a.diff = diff;
    a.reasons = summary::reasons(&a);
    a
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ctx() -> Context {
        Context { project: "C:/p".into(), cwd: "C:/p".into(), home: "C:/h".into(), temp: vec!["C:/t".into()], ..Default::default() }
    }

    #[test]
    fn edits_in_and_out_of_the_project() {
        let a = analyze_tool("Edit", &json!({"file_path": "C:/p/src/a.rs", "old_string": "a\nb", "new_string": "a\nc\nd"}), &ctx(), None);
        assert_eq!(a.risk, Risk::Caution);
        assert_eq!(a.summary, "Edits a.rs.");
        assert!(a.auto_allow_eligible);
        assert_eq!(a.diff.as_ref().unwrap().added, 2);

        let a = analyze_tool("Write", &json!({"file_path": "C:/h/.bashrc", "content": "alias ls=rm"}), &ctx(), Some("# old"));
        assert_eq!(a.risk, Risk::Risky);
        assert_eq!(a.summary, "Changes your shell startup file (.bashrc), which runs every time a terminal opens.");
        assert!(!a.auto_allow_eligible);

        let a = analyze_tool("Write", &json!({"file_path": "C:/p/new.md", "content": "a\nb\n"}), &ctx(), None);
        assert_eq!(a.summary, "Creates new.md, a new file of 2 lines.");
    }

    #[test]
    fn other_tools() {
        assert_eq!(analyze_tool("WebFetch", &json!({"url": "https://docs.example.com/x"}), &ctx(), None).summary, "Fetches a page from docs.example.com.");
        let a = analyze_tool("Read", &json!({"file_path": "C:/p/.env"}), &ctx(), None);
        assert_eq!(a.risk, Risk::Caution);
        let a = analyze_tool("mcp__github__create_issue", &json!({}), &ctx(), None);
        assert_eq!(a.summary, "Uses “create issue” from the github connector.");
        let a = analyze_tool("Bash", &json!({"command": "rm -rf /"}), &ctx(), None);
        assert_eq!(a.risk, Risk::Risky);
    }
}
