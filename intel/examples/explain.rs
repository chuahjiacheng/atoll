//! Print the analysis of one command as JSON.
//!
//! cargo run -p crest-intel --example explain -- [--ps] [--cwd <folder>] '<command>'
//! With no command argument, the command is read from stdin.

use std::io::Read;

use crest_intel::{analyze, Context};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut tool = "Bash";
    let mut cwd = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
    if let Some(i) = args.iter().position(|a| a == "--ps") {
        args.remove(i);
        tool = "PowerShell";
    }
    if let Some(i) = args.iter().position(|a| a == "--tool") {
        args.drain(i..(i + 2).min(args.len()));
    }
    if let Some(i) = args.iter().position(|a| a == "--cwd") {
        args.remove(i);
        if i < args.len() {
            cwd = args.remove(i);
        }
    }
    let command = match args.first() {
        Some(c) => c.clone(),
        None => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).expect("read stdin");
            s
        }
    };
    if std::env::var_os("EXPLAIN_TREE").is_some() {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_bash::LANGUAGE.into()).unwrap();
        let t = p.parse(&command, None).unwrap();
        fn dump(n: tree_sitter::Node, src: &str, d: usize) {
            let text: String = src[n.byte_range()].chars().take(40).collect::<String>().replace('\n', "⏎");
            println!("{}{}{} `{}`", "  ".repeat(d), if n.is_named() { "" } else { "~" }, n.kind(), text);
            let mut c = n.walk();
            for k in n.children(&mut c) {
                dump(k, src, d + 1);
            }
        }
        dump(t.root_node(), &command, 0);
    }
    let ctx = Context::from_system(&cwd, &cwd);
    // `--tool Edit '{"file_path": ..., "old_string": ..., "new_string": ...}'`: any tool, input as JSON.
    if let Some(i) = std::env::args().position(|a| a == "--tool") {
        let name = std::env::args().nth(i + 1).unwrap_or_default();
        let input: serde_json::Value = serde_json::from_str(&command).expect("tool input must be JSON");
        let existing = std::env::var("EXPLAIN_EXISTING").ok();
        let a = crest_intel::analyze_tool(&name, &input, &ctx, existing.as_deref());
        println!("{}", serde_json::to_string_pretty(&a).unwrap());
        return;
    }
    let a = analyze(tool, &command, &ctx);
    println!("{}", serde_json::to_string_pretty(&a).unwrap());
}
