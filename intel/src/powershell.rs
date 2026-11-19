//! PowerShell front end. A tokenizer and a table of cmdlets, not a full
//! parser: good enough to explain a command and flag danger, deliberately not
//! trusted for auto-allow (see `eligibility` in lib.rs).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::bash::{Invocation, Redirect, Word};
use crate::classify::{self, delete, network, read, reads_code_from_stdin, shape, short, write, Cwd, Cx, Write};
use crate::model::{Builder, Flag, Risk};
use crate::paths;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// Bare word, string or variable, already resolved into a word.
    Word(Word, Kind),
    /// `;`, newline, `&&`, `||`
    Sep,
    Pipe,
    /// `(`, `{`, `$(`, `@(`, `@{`
    Open,
    Close,
    /// `=`, `+=`, ...
    Assign,
    /// `>`, `>>`, `2>` ...
    Redirect(String),
    /// `&` or `.` used to call something.
    Call,
    Comma,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Bare,
    Quoted,
    Var,
    Type,
}

struct Lexer<'a> {
    c: Vec<char>,
    i: usize,
    vars: &'a HashMap<String, String>,
    out: Vec<Tok>,
}

fn ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl<'a> Lexer<'a> {
    fn peek(&self, n: usize) -> char {
        self.c.get(self.i + n).copied().unwrap_or('\0')
    }

    fn at_command_start(&self) -> bool {
        matches!(self.out.last(), None | Some(Tok::Sep | Tok::Pipe | Tok::Open | Tok::Assign | Tok::Close))
    }

    /// `$name`, `${name}`, `$env:NAME`; returns the lower-cased name.
    fn var_name(&mut self) -> String {
        let mut name = String::new();
        if self.peek(0) == '{' {
            self.i += 1;
            while self.i < self.c.len() && self.peek(0) != '}' {
                name.push(self.peek(0));
                self.i += 1;
            }
            self.i += 1;
        } else {
            while ident(self.peek(0)) || (self.peek(0) == ':' && ident(self.peek(1)) && !name.contains(':')) || (name.is_empty() && matches!(self.peek(0), '?' | '$' | '^')) {
                name.push(self.peek(0));
                self.i += 1;
            }
        }
        name.to_lowercase()
    }

    fn lookup(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn double_quoted(&mut self, end_here: bool) -> Word {
        let mut w = Word { exact: true, ..Default::default() };
        while self.i < self.c.len() {
            let c = self.peek(0);
            if end_here {
                if c == '\n' && self.peek(1) == '"' && self.peek(2) == '@' {
                    self.i += 3;
                    return w;
                }
            } else if c == '"' {
                if self.peek(1) == '"' {
                    w.text.push('"');
                    self.i += 2;
                    continue;
                }
                self.i += 1;
                return w;
            }
            match c {
                '`' if !end_here => {
                    self.i += 1;
                    let n = self.peek(0);
                    w.text.push(match n {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => ' ',
                        other => other,
                    });
                    self.i += 1;
                }
                '$' if self.peek(1) == '(' => {
                    // Sub-expression: its commands are found by the outer scan of the raw text.
                    let mut depth = 0;
                    let start = self.i;
                    while self.i < self.c.len() {
                        match self.peek(0) {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    self.i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.i += 1;
                    }
                    w.text.extend(&self.c[start..self.i.min(self.c.len())]);
                    w.exact = false;
                    w.subst = true;
                }
                '$' if ident(self.peek(1)) || self.peek(1) == '{' => {
                    self.i += 1;
                    let name = self.var_name();
                    match self.lookup(&name) {
                        Some(v) => w.text.push_str(&v),
                        None => {
                            w.text.push('$');
                            w.text.push_str(&name);
                            w.exact = false;
                        }
                    }
                }
                other => {
                    w.text.push(other);
                    self.i += 1;
                }
            }
        }
        w
    }

    fn single_quoted(&mut self, end_here: bool) -> Word {
        let mut w = Word { exact: true, ..Default::default() };
        while self.i < self.c.len() {
            let c = self.peek(0);
            if end_here {
                if c == '\n' && self.peek(1) == '\'' && self.peek(2) == '@' {
                    self.i += 3;
                    return w;
                }
            } else if c == '\'' {
                if self.peek(1) == '\'' {
                    w.text.push('\'');
                    self.i += 2;
                    continue;
                }
                self.i += 1;
                return w;
            }
            w.text.push(c);
            self.i += 1;
        }
        w
    }

    fn run(&mut self) {
        while self.i < self.c.len() {
            let c = self.peek(0);
            match c {
                ' ' | '\t' | '\r' => self.i += 1,
                '`' if matches!(self.peek(1), '\n' | '\r') => {
                    self.i += 2;
                    if self.peek(0) == '\n' {
                        self.i += 1;
                    }
                }
                '\n' => {
                    self.i += 1;
                    if !matches!(self.out.last(), Some(Tok::Pipe | Tok::Comma | Tok::Open | Tok::Sep) | None) {
                        self.out.push(Tok::Sep);
                    }
                }
                '#' => {
                    while self.i < self.c.len() && self.peek(0) != '\n' {
                        self.i += 1;
                    }
                }
                '<' if self.peek(1) == '#' => {
                    while self.i < self.c.len() && !(self.peek(0) == '#' && self.peek(1) == '>') {
                        self.i += 1;
                    }
                    self.i += 2;
                }
                ';' => {
                    self.i += 1;
                    self.out.push(Tok::Sep);
                }
                '|' => {
                    self.i += 1;
                    if self.peek(0) == '|' {
                        self.i += 1;
                        self.out.push(Tok::Sep);
                    } else {
                        self.out.push(Tok::Pipe);
                    }
                }
                '&' => {
                    self.i += 1;
                    if self.peek(0) == '&' {
                        self.i += 1;
                        self.out.push(Tok::Sep);
                    } else if self.at_command_start() {
                        self.out.push(Tok::Call);
                    } else {
                        self.out.push(Tok::Sep); // background job
                    }
                }
                '.' if self.at_command_start() && matches!(self.peek(1), ' ' | '\t' | '(' | '"' | '\'' | '$') => {
                    self.i += 1;
                    self.out.push(Tok::Call);
                }
                '(' | '{' => {
                    self.i += 1;
                    self.out.push(Tok::Open);
                }
                ')' | '}' => {
                    self.i += 1;
                    self.out.push(Tok::Close);
                    // .Method() or [index] straight after a group: part of the expression.
                    while matches!(self.peek(0), '.' | '[') && (ident(self.peek(1)) || self.peek(0) == '[') {
                        while self.i < self.c.len() && !matches!(self.peek(0), ' ' | '\t' | '\n' | ';' | '|' | '(' | ')' | '{' | '}') {
                            self.i += 1;
                        }
                    }
                }
                ',' => {
                    self.i += 1;
                    self.out.push(Tok::Comma);
                }
                '@' if matches!(self.peek(1), '(' | '{') => {
                    self.i += 2;
                    self.out.push(Tok::Open);
                }
                '@' if self.peek(1) == '\'' && matches!(self.peek(2), '\n' | '\r') => {
                    self.i += 2;
                    while matches!(self.peek(0), '\r' | '\n') {
                        self.i += 1;
                    }
                    let w = self.single_quoted(true);
                    self.out.push(Tok::Word(w, Kind::Quoted));
                }
                '@' if self.peek(1) == '"' && matches!(self.peek(2), '\n' | '\r') => {
                    self.i += 2;
                    while matches!(self.peek(0), '\r' | '\n') {
                        self.i += 1;
                    }
                    let w = self.double_quoted(true);
                    self.out.push(Tok::Word(w, Kind::Quoted));
                }
                '$' if self.peek(1) == '(' => {
                    self.i += 2;
                    self.out.push(Tok::Open);
                }
                '$' if ident(self.peek(1)) || matches!(self.peek(1), '{' | '?' | '$' | '^') => {
                    self.i += 1;
                    let name = self.var_name();
                    // Property access, indexing and static members ride along.
                    let mut tail = String::new();
                    while matches!(self.peek(0), '.' | '[' | ':') && (ident(self.peek(1)) || self.peek(0) == '[' || self.peek(1) == ':') {
                        while self.i < self.c.len() && !matches!(self.peek(0), ' ' | '\t' | '\n' | ';' | '|' | '(' | ')' | '{' | '}' | ',' | '=') {
                            tail.push(self.peek(0));
                            self.i += 1;
                        }
                    }
                    let known = if tail.is_empty() { self.lookup(&name) } else { None };
                    // A path glued to a variable: $dir\file.txt
                    let mut rest = String::new();
                    if tail.is_empty() {
                        while self.i < self.c.len() && !matches!(self.peek(0), ' ' | '\t' | '\n' | ';' | '|' | '(' | ')' | '{' | '}' | ',' | '=' | '"' | '\'') {
                            rest.push(self.peek(0));
                            self.i += 1;
                        }
                    }
                    let w = match known {
                        Some(v) => Word { text: format!("{v}{rest}"), exact: true, ..Default::default() },
                        None => Word { text: format!("${name}{tail}{rest}"), exact: false, ..Default::default() },
                    };
                    self.out.push(Tok::Word(w, Kind::Var));
                }
                '"' => {
                    self.i += 1;
                    let w = self.double_quoted(false);
                    self.out.push(Tok::Word(w, Kind::Quoted));
                }
                '\'' => {
                    self.i += 1;
                    let w = self.single_quoted(false);
                    self.out.push(Tok::Word(w, Kind::Quoted));
                }
                '[' if self.at_command_start() || matches!(self.out.last(), Some(Tok::Word(..) | Tok::Comma)) && (self.peek(1).is_alphabetic()) => {
                    let start = self.i;
                    let mut depth = 0;
                    while self.i < self.c.len() {
                        match self.peek(0) {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    self.i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.i += 1;
                    }
                    while self.i < self.c.len() && (ident(self.peek(0)) || matches!(self.peek(0), ':' | '.')) {
                        self.i += 1;
                    }
                    let text: String = self.c[start..self.i.min(self.c.len())].iter().collect();
                    self.out.push(Tok::Word(Word { text, exact: false, ..Default::default() }, Kind::Type));
                }
                '=' if self.peek(1) != '=' => {
                    self.i += 1;
                    self.out.push(Tok::Assign);
                }
                '+' | '-' | '*' | '/' | '%' if self.peek(1) == '=' && !self.at_command_start() => {
                    self.i += 2;
                    self.out.push(Tok::Assign);
                }
                '>' => {
                    let mut op = String::from(">");
                    self.i += 1;
                    if self.peek(0) == '>' {
                        op.push('>');
                        self.i += 1;
                    }
                    if self.peek(0) == '&' {
                        self.i += 2; // >&1
                        continue;
                    }
                    self.out.push(Tok::Redirect(op));
                }
                _ => {
                    // Bare word. `2>`, `*>` and friends are redirections.
                    if matches!(c, '1'..='6' | '*') && self.peek(1) == '>' {
                        self.i += 1;
                        continue;
                    }
                    let mut w = Word { exact: true, ..Default::default() };
                    while self.i < self.c.len() {
                        let c = self.peek(0);
                        match c {
                            ' ' | '\t' | '\n' | '\r' | ';' | '|' | '(' | ')' | '{' | '}' | ',' => break,
                            '>' => break,
                            '=' if w.text.starts_with('$') => break,
                            '`' => {
                                self.i += 1;
                                if matches!(self.peek(0), '\n' | '\r') {
                                    break;
                                }
                                if self.peek(0).is_alphanumeric() {
                                    w.obf = true;
                                }
                                w.text.push(self.peek(0));
                                self.i += 1;
                            }
                            '"' => {
                                self.i += 1;
                                let q = self.double_quoted(false);
                                w.text.push_str(&q.text);
                                w.exact &= q.exact;
                            }
                            '\'' => {
                                self.i += 1;
                                let q = self.single_quoted(false);
                                w.text.push_str(&q.text);
                            }
                            '$' if ident(self.peek(1)) || self.peek(1) == '{' => {
                                self.i += 1;
                                let name = self.var_name();
                                match self.lookup(&name) {
                                    Some(v) => w.text.push_str(&v),
                                    None => {
                                        w.text.push('$');
                                        w.text.push_str(&name);
                                        w.exact = false;
                                    }
                                }
                            }
                            '*' | '?' => {
                                w.glob = true;
                                w.text.push(c);
                                self.i += 1;
                            }
                            _ => {
                                w.text.push(c);
                                self.i += 1;
                            }
                        }
                    }
                    if w.text.is_empty() {
                        self.i += 1;
                    } else {
                        self.out.push(Tok::Word(w, Kind::Bare));
                    }
                }
            }
        }
    }
}

const KEYWORDS: &[&str] = &["if", "else", "elseif", "foreach", "for", "while", "do", "switch", "try", "catch", "finally", "param", "return", "break", "continue", "throw", "exit", "begin", "process", "end", "in", "until", "trap", "class", "using", "default", "data", "dynamicparam", "enum", "hidden", "static", "workflow", "parallel", "sequence", "inlinescript", "clean"];

fn alias(name: &str) -> &str {
    match name {
        "rm" | "del" | "erase" | "rd" | "ri" | "rmdir" => "remove-item",
        "gci" | "ls" | "dir" => "get-childitem",
        "gc" | "cat" | "type" => "get-content",
        "select" => "select-object",
        "sls" => "select-string",
        "where" | "?" => "where-object",
        "foreach" | "%" => "foreach-object",
        "sort" => "sort-object",
        "measure" => "measure-object",
        "echo" | "write" => "write-output",
        "ps" | "gps" => "get-process",
        "cd" | "sl" | "chdir" => "set-location",
        "pushd" => "push-location",
        "popd" => "pop-location",
        "pwd" | "gl" => "get-location",
        "sleep" => "start-sleep",
        "sc" => "set-content",
        "ac" => "add-content",
        "ni" | "mkdir" | "md" => "new-item",
        "cp" | "copy" | "cpi" => "copy-item",
        "mv" | "move" | "mi" => "move-item",
        "ren" | "rni" => "rename-item",
        "kill" | "spps" => "stop-process",
        "start" | "saps" => "start-process",
        "iex" => "invoke-expression",
        "iwr" | "curl" | "wget" => "invoke-webrequest",
        "irm" => "invoke-restmethod",
        "icm" => "invoke-command",
        "ii" => "invoke-item",
        "tee" => "tee-object",
        "gi" => "get-item",
        "gp" => "get-itemproperty",
        "sp" => "set-itemproperty",
        "rp" => "remove-itemproperty",
        "clc" => "clear-content",
        "cli" => "clear-item",
        "gcm" => "get-command",
        "gm" => "get-member",
        "ft" => "format-table",
        "fl" => "format-list",
        "group" => "group-object",
        "diff" | "compare" => "compare-object",
        "man" | "help" => "get-help",
        "cls" | "clear" => "clear-host",
        "sv" | "set" => "set-variable",
        "gv" => "get-variable",
        "ipmo" => "import-module",
        "sajb" => "start-job",
        "gwmi" => "get-wmiobject",
        "iwmi" => "invoke-wmimethod",
        "gcim" => "get-ciminstance",
        "sasv" => "start-service",
        "spsv" => "stop-service",
        "gsv" => "get-service",
        "ogv" => "out-gridview",
        "oh" => "out-host",
        "epcsv" => "export-csv",
        "ipcsv" => "import-csv",
        "nal" => "new-alias",
        "sal" => "set-alias",
        "rvpa" => "resolve-path",
        "history" | "h" | "ghy" => "get-history",
        other => other,
    }
}

static VERB_NOUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z]+-[a-z][a-z0-9]*$").unwrap());

/// Things worth flagging wherever they appear in the text (method calls and
/// .NET types are not commands, so the token walk does not see them).
static DOTNET: LazyLock<Vec<(Regex, Flag, Risk, &'static str)>> = LazyLock::new(|| {
    let r = |p: &str| Regex::new(&format!("(?i){p}")).unwrap();
    vec![
        (r(r"\biex\b|invoke-expression|\[scriptblock\]\s*::\s*create|\$executioncontext\.invoke(command|script)|\.invokescript\(|invoke-command\s+-scriptblock\s+\(\["), Flag::DynamicCommand, Risk::Risky, "builds a command from text and runs it"),
        (r(r"-e(nc|ncodedcommand|c)?\s+[A-Za-z0-9+/=]{24,}"), Flag::Obfuscated, Risk::Risky, "runs a PowerShell command hidden in encoded text"),
        (r(r"reflection\.assembly\]\s*::\s*load|\[appdomain\]|virtualalloc|createremotethread|writeprocessmemory|createthread|ntprotectvirtualmemory|marshal\]\s*::\s*(copy|getdelegateforfunctionpointer)|amsiutils|amsiinitfailed"), Flag::HiddenExec, Risk::Risky, "loads or injects code into memory"),
        (r(r"\[wmiclass\]|win32_process[^|;]*\bcreate\b|invoke-(wmi|cim)method"), Flag::HiddenExec, Risk::Risky, "starts a program through Windows management (hard to trace)"),
        (r(r"frombase64string|-bxor|\[char\]\s*\d+\s*\+\s*\[char\]|-join\s*\(?\s*\(?\s*\d+\s*,|\[convert\]::to(char|byte)|-replace\s+'[^']{1,3}'\s*,\s*''\s*\)?\s*\|\s*iex|tostring\(\s*\d+\s*,\s*\d+\s*\)"), Flag::Obfuscated, Risk::Caution, "decodes or assembles hidden text"),
        (r(r"net\.webclient|net\.http\.httpclient|net\.sockets|net\.webrequest|downloadstring|downloadfile|downloaddata|msxml2\.xmlhttp|winhttp\.winhttprequest|system\.net\.dns"), Flag::Network, Risk::Caution, "uses the network from .NET code"),
        (r(r"uploadstring|uploadfile|uploaddata|uploadvalues"), Flag::SendsData, Risk::Caution, "uploads data"),
        (r(r"io\.(file|directory)\]\s*::\s*delete|\.delete\(\s*(\$true)?\s*\)|filesystem\]\s*::\s*delete(file|directory)"), Flag::Delete, Risk::Risky, "deletes files from .NET code (cannot tell which)"),
        (r(r"io\.(file|directory)\]\s*::\s*(writeall\w*|appendall\w*|create\w*|move|copy|replace|open\w*|set\w*)|\.save\(|\.saveas\(|io\.streamwriter|io\.filestream|\.copyto\(|\.moveto\("), Flag::WritesFile, Risk::Caution, "writes files from .NET code (cannot tell where)"),
        (r(r"environment\]\s*::\s*setenvironmentvariable\([^)]*,\s*['\x22]?(machine|user)"), Flag::SystemChange, Risk::Risky, "changes environment variables permanently"),
        (r(r"microsoft\.win32\.registry|\[registry"), Flag::SystemChange, Risk::Risky, "reads or changes the Windows registry from .NET code"),
        (r(r"diagnostics\.process\]\s*::\s*start|new-object\s+[\w.]*diagnostics\.process(startinfo)?"), Flag::RunsCode, Risk::Caution, "starts another program"),
        (r(r"windows\.forms\.sendkeys|keybd_event|sendinput|mouse_event|setcursorpos"), Flag::RunsCode, Risk::Caution, "sends keystrokes or mouse input to other windows"),
        (r(r"wscript\.shell|shell\.application|schedule\.service|adodb\.stream"), Flag::HiddenExec, Risk::Risky, "drives Windows scripting components that can run programs"),
        (r(r"get-credential|convertfrom-securestring|credentialmanager|passwordvault|get-storedcredential|dpapi|protecteddata\]\s*::\s*unprotect|\bcmdkey\b|lsass|sekurlsa|get-azkeyvaultsecret|export-pfxcertificate"), Flag::SensitivePath, Risk::Risky, "reads stored passwords or keys"),
        (r(r"-windowstyle\s+hidden|-w\s+hidden\b"), Flag::Background, Risk::Caution, "runs in a hidden window"),
        (r(r"(drop\s+(table|database)|truncate\s+table)\b"), Flag::Database, Risk::Risky, "contains SQL that drops or empties tables"),
    ]
});

const SWITCHES: &[&str] = &["recurse", "force", "whatif", "confirm", "verbose", "nonewline", "raw", "passthru", "wait", "noprofile", "usebasicparsing", "asjob", "append", "noclobber", "file", "directory", "hidden", "readonly", "system", "leaf", "container", "isvalid", "ashashtable", "compress", "notypeinformation", "wrap", "autosize", "unique", "descending", "casesensitive", "simplematch", "quiet", "list", "allmatches", "notmatch", "nonewwindow", "debug", "all", "asbytestream", "skipcertificatecheck", "allowunencryptedauthentication", "disablekeepalive", "ascending", "stream", "expand", "norecurse", "followsymlink", "nameonly", "keeptemplate", "sta", "noexit", "nologo", "noninteractive"];

struct PsArgs {
    named: Vec<(String, Option<Word>)>,
    pos: Vec<Word>,
}

impl PsArgs {
    /// PowerShell accepts any unambiguous prefix of a parameter name.
    fn has(&self, full: &str) -> bool {
        self.named.iter().any(|(n, _)| !n.is_empty() && full.starts_with(n.as_str()))
    }
    fn get(&self, names: &[&str]) -> Option<Word> {
        self.named.iter().find(|(n, v)| v.is_some() && n.len() >= 1 && names.iter().any(|f| f.starts_with(n.as_str()) && (n.len() >= 2 || f.len() == 1 || names.len() == 1))).and_then(|(_, v)| v.clone())
    }
}

fn ps_args(args: &[Word]) -> PsArgs {
    let mut out = PsArgs { named: vec![], pos: vec![] };
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        let is_param = w.text.starts_with('-') && w.text.len() > 1 && w.text[1..].chars().next().is_some_and(|c| c.is_alphabetic());
        if is_param {
            let body = w.text[1..].to_lowercase();
            if let Some((n, v)) = body.split_once(':') {
                let raw_value = &w.text[w.text.find(':').unwrap() + 1..];
                let _ = v;
                out.named.push((n.to_string(), Some(Word { text: raw_value.to_string(), ..w.clone() })));
            } else if SWITCHES.iter().any(|s| s.starts_with(body.as_str())) {
                // `-r`, `-fo`: PowerShell accepts any short prefix of a switch. Reading such
                // a prefix as "takes a value" would swallow the path that follows it.
                out.named.push((body, None));
            } else {
                let next = args.get(i + 1).filter(|n| !(n.text.starts_with('-') && n.text.len() > 1 && n.text[1..].chars().next().is_some_and(|c| c.is_alphabetic())));
                match next {
                    Some(v) => {
                        out.named.push((body, Some(v.clone())));
                        i += 1;
                    }
                    None => out.named.push((body, None)),
                }
            }
        } else {
            out.pos.push(w.clone());
        }
        i += 1;
    }
    out
}

fn is_registry(w: &Word) -> bool {
    let t = w.text.to_lowercase();
    t.starts_with("hklm:") || t.starts_with("hkcu:") || t.starts_with("hkcr:") || t.starts_with("hku:") || t.starts_with("registry::") || t.starts_with("hkey_")
}

fn is_ps_drive(w: &Word) -> bool {
    let t = w.text.to_lowercase();
    ["env:", "variable:", "function:", "alias:", "cert:", "wsman:"].iter().any(|d| t.starts_with(d))
}

struct State {
    vars: HashMap<String, String>,
    cwd: Cwd,
    functions: Vec<String>,
}

/// Analyse PowerShell text, adding findings to the builder in `cx`.
pub(crate) fn analyze_into(cx: &mut Cx, text: &str, cwd: &Cwd) {
    if cx.depth > 4 {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "commands are nested unusually deep".to_string());
        return;
    }
    cx.depth += 1;
    for (re, flag, risk, note) in DOTNET.iter() {
        if re.is_match(text) {
            cx.b.find(*flag, *risk, note.to_string());
            if *flag == Flag::Network {
                cx.b.remote_network = true;
            }
            if *flag == Flag::SensitivePath {
                Builder::push_unique(&mut cx.b.secret_reads, "stored credentials");
            }
        }
    }
    let lower = text.to_lowercase();
    if (lower.contains("frombase64string") || lower.contains("-bxor") || lower.contains("[char]")) && (lower.contains("iex") || lower.contains("invoke-expression") || lower.contains("scriptblock") || lower.contains(".invoke(")) {
        cx.b.find(Flag::Obfuscated, Risk::Risky, "decodes hidden text and runs it".to_string());
    }
    if (lower.contains("downloadstring") || lower.contains("invoke-webrequest") || lower.contains("iwr ") || lower.contains("irm ") || lower.contains("invoke-restmethod") || lower.contains("downloaddata")) && (lower.contains("iex") || lower.contains("invoke-expression")) {
        cx.b.find(Flag::RemoteCode, Risk::Risky, "downloads something and runs it without showing it".to_string());
    }

    let mut vars: HashMap<String, String> = HashMap::new();
    for (k, v) in &cx.ctx.env {
        vars.insert(format!("env:{}", k.to_lowercase()), v.clone());
    }
    if !cx.ctx.home.is_empty() {
        vars.insert("home".into(), cx.ctx.home.clone());
        vars.entry("env:userprofile".into()).or_insert(cx.ctx.home.clone());
    }
    if let Some(c) = cwd.as_ref().and_then(|c| c.first()) {
        vars.insert("pwd".into(), c.clone());
    }
    vars.insert("null".into(), "$null".into());
    let mut st = State { vars, cwd: cwd.clone(), functions: vec![] };

    // Lex statement by statement so `$x = 'literal'` is known to later statements.
    let mut lexer_vars = st.vars.clone();
    let toks = {
        let mut lx = Lexer { c: text.chars().collect(), i: 0, vars: &lexer_vars, out: vec![] };
        lx.run();
        lx.out
    };
    // First pass: top-level literal assignments, then lex again with them known.
    let mut depth = 0i32;
    let mut i = 0;
    let mut conditional = false;
    while i < toks.len() {
        match &toks[i] {
            Tok::Open => depth += 1,
            Tok::Close => depth -= 1,
            Tok::Word(w, Kind::Bare) if depth == 0 && KEYWORDS.contains(&w.text.to_lowercase().as_str()) => conditional = true,
            Tok::Word(w, Kind::Var) if depth == 0 && !conditional && matches!(toks.get(i + 1), Some(Tok::Assign)) => {
                let name = w.text.trim_start_matches('$').to_lowercase();
                let value_is_literal = matches!(toks.get(i + 2), Some(Tok::Word(v, Kind::Quoted | Kind::Bare)) if v.exact) && matches!(toks.get(i + 3), None | Some(Tok::Sep));
                if value_is_literal && !lexer_vars.contains_key(&name) || value_is_literal && !name.starts_with("env:") {
                    if let Some(Tok::Word(v, _)) = toks.get(i + 2) {
                        lexer_vars.insert(name, v.text.clone());
                    }
                } else {
                    lexer_vars.remove(&name);
                }
            }
            _ => {}
        }
        i += 1;
    }
    st.vars = lexer_vars.clone();
    let toks = {
        let mut lx = Lexer { c: text.chars().collect(), i: 0, vars: &lexer_vars, out: vec![] };
        lx.run();
        lx.out
    };

    // Split into commands.
    let mut cmd: Vec<Tok> = vec![];
    let mut piped = false;
    let mut upstream: Vec<String> = vec![];
    let mut stack: Vec<(bool, Vec<String>)> = vec![];
    for t in toks {
        match t {
            Tok::Sep => {
                command(cx, &mut st, std::mem::take(&mut cmd), piped, &mut upstream);
                piped = false;
                upstream.clear();
            }
            Tok::Pipe => {
                command(cx, &mut st, std::mem::take(&mut cmd), piped, &mut upstream);
                piped = true;
            }
            Tok::Open => {
                // The group is an argument of the command so far; what is inside is analysed on its own.
                cmd.push(Tok::Word(Word { text: "(…)".into(), exact: false, subst: true, ..Default::default() }, Kind::Var));
                stack.push((piped, upstream.clone()));
                let outer = std::mem::take(&mut cmd);
                stack.last_mut().unwrap().1.push(String::new());
                OUTER.with(|o| o.borrow_mut().push(outer));
                piped = false;
                upstream.clear();
            }
            Tok::Close => {
                command(cx, &mut st, std::mem::take(&mut cmd), piped, &mut upstream);
                if let Some((p, mut u)) = stack.pop() {
                    u.pop();
                    piped = p;
                    upstream = u;
                    cmd = OUTER.with(|o| o.borrow_mut().pop()).unwrap_or_default();
                }
            }
            other => cmd.push(other),
        }
    }
    command(cx, &mut st, cmd, piped, &mut upstream);
    while let Some(outer) = OUTER.with(|o| o.borrow_mut().pop()) {
        // Unbalanced brackets: still classify what was started.
        let mut none = vec![];
        command(cx, &mut st, outer, false, &mut none);
        cx.b.parsed = false;
    }
    cx.depth -= 1;
}

thread_local! {
    static OUTER: std::cell::RefCell<Vec<Vec<Tok>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn command(cx: &mut Cx, st: &mut State, toks: Vec<Tok>, piped: bool, upstream: &mut Vec<String>) {
    let mut toks = toks;
    if toks.is_empty() {
        return;
    }
    // `$x = ...`: what matters is the right-hand side.
    if let (Some(Tok::Word(w, Kind::Var)), Some(Tok::Assign)) = (toks.first(), toks.get(1)) {
        let name = w.text.trim_start_matches('$').to_lowercase();
        if let Some(env_name) = name.strip_prefix("env:") {
            let value = match toks.get(2) {
                Some(Tok::Word(v, _)) => v.clone(),
                _ => Word::default(),
            };
            classify::env_var(cx, &st.cwd.clone(), env_name, &value);
        }
        toks.drain(..2);
        if toks.is_empty() {
            return;
        }
    }
    let mut called = false;
    if toks.first() == Some(&Tok::Call) {
        called = true;
        toks.remove(0);
    }
    let Some(Tok::Word(head, kind)) = toks.first().cloned() else { return };
    let lower = head.text.to_lowercase();

    if called {
        if !head.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the program to run is only decided when it runs".to_string());
            return;
        }
    } else {
        match kind {
            Kind::Type | Kind::Var | Kind::Quoted => return, // an expression, covered by the text scan
            Kind::Bare => {}
        }
        if head.text.starts_with(['-', '+', '!', '@']) || head.text.chars().next().is_some_and(|c| c.is_ascii_digit()) && !head.text.contains(['\\', '/']) {
            return;
        }
        if KEYWORDS.contains(&lower.as_str()) {
            if lower == "trap" || lower == "throw" || lower == "return" || lower == "exit" {
                return;
            }
            // `if (...) { ... }`: the groups were handled separately; anything left is expression glue.
            return;
        }
        if lower == "function" || lower == "filter" {
            if let Some(Tok::Word(n, _)) = toks.get(1) {
                st.functions.push(n.text.to_lowercase());
            }
            return;
        }
    }
    if head.obf {
        cx.b.find(Flag::Obfuscated, Risk::Risky, format!("the command name is written in a disguised way (it is “{}”)", head.text));
    }

    // Build the invocation.
    let mut argv = vec![head.clone()];
    let mut redirects = vec![];
    let mut it = toks.into_iter().skip(1).peekable();
    while let Some(t) = it.next() {
        match t {
            Tok::Word(w, _) => argv.push(w),
            Tok::Redirect(op) => {
                if let Some(Tok::Word(target, _)) = it.next() {
                    redirects.push(Redirect { op, target });
                }
            }
            Tok::Assign => argv.push(Word::lit("=")),
            _ => {}
        }
    }
    let name = alias(&lower).to_string();
    let inv = Invocation { argv, redirects, stdin_piped: piped, upstream: upstream.clone(), cwd: st.cwd.clone(), ..Default::default() };
    upstream.push(name.clone());
    if st.functions.contains(&lower) {
        return;
    }
    let is_cmdlet = VERB_NOUN.is_match(&name) && !name.contains('.') && !matches!(name.as_str(), "docker-compose" | "apt-get" | "svelte-check" | "ts-node" | "http-server" | "pre-commit" | "lint-staged" | "gh-pages" | "eas-cli" | "wasm-pack" | "nix-env" | "cross-env" | "wait-on" | "ssh-keygen" | "ssh-add" | "ssh-copy-id" | "redis-cli" | "sentry-cli" | "linode-cli" | "electron-builder" | "drizzle-kit" | "ansible-playbook" | "clickhouse-client" | "firewall-cmd" | "update-alternatives" | "clippy-driver" | "wasm-bindgen" | "sphinx-build" | "semantic-release" | "tsc-watch" | "svelte-kit" | "react-scripts" | "vue-cli-service" | "manage-bde" | "xdg-open");
    if is_cmdlet && !head.text.contains(['\\', '/']) {
        cmdlet(cx, st, &name, &inv);
    } else {
        // A native program: same rules as in Bash. Redirections included.
        let mut inv = inv;
        if lower != name && !is_cmdlet {
            inv.argv[0].text = name;
        }
        classify::invocation(cx, &inv);
    }
}

fn paths_of(a: &PsArgs, names: &[&str], positional: Option<usize>) -> Vec<Word> {
    let mut out: Vec<Word> = vec![];
    if let Some(v) = a.get(names) {
        out.push(v);
    }
    if out.is_empty() {
        if let Some(i) = positional {
            if let Some(p) = a.pos.get(i) {
                out.push(p.clone());
            }
        }
    }
    out
}

fn cmdlet(cx: &mut Cx, st: &mut State, name: &str, inv: &Invocation) {
    let cwd = st.cwd.clone();
    let args = &inv.argv[1..];
    let a = ps_args(args);
    let sub: Vec<Word> = a.named.iter().map(|(n, _)| Word::lit(&format!("-{n}"))).collect();
    shape(cx, name, "", &sub);
    for r in &inv.redirects {
        write(cx, &cwd, &r.target, if r.op.contains(">>") { Write::Append } else { Write::Truncate });
    }
    let what_if = a.has("whatif");
    let risky = |cx: &mut Cx, flag: Flag, note: &str| cx.b.find(flag, Risk::Risky, note.to_string());
    let caution = |cx: &mut Cx, flag: Flag, note: &str| cx.b.find(flag, Risk::Caution, note.to_string());
    let (verb, noun) = name.split_once('-').unwrap_or((name, ""));

    match name {
        "remove-item" | "clear-content" | "clear-item" | "remove-itemproperty" | "clear-itemproperty" => {
            if what_if {
                return;
            }
            let mut targets = a.pos.clone();
            targets.extend(a.get(&["path", "literalpath"]));
            let strong = a.has("recurse") || a.has("force") || name != "remove-item" || args.iter().any(|w| w.text.to_lowercase().starts_with("-r") || w.text.to_lowercase().starts_with("-fo"));
            if targets.is_empty() && inv.stdin_piped {
                risky(cx, Flag::Delete, "deletes whatever the earlier command lists");
            } else if targets.is_empty() {
                // Fail safe: a delete whose target Atoll could not find is treated as unknown.
                risky(cx, Flag::UnresolvedPath, "deletes something Atoll could not identify");
            }
            for t in &targets {
                if is_registry(t) {
                    risky(cx, Flag::SystemChange, "deletes Windows registry keys or values");
                } else if is_ps_drive(t) {
                    caution(cx, Flag::WritesFile, "removes a session variable, alias or function");
                } else {
                    delete(cx, &cwd, t, strong);
                }
            }
        }
        "clear-recyclebin" => risky(cx, Flag::Delete, "empties the Recycle Bin for good"),
        "set-content" | "add-content" | "out-file" | "tee-object" | "export-csv" | "export-clixml" | "start-transcript" | "set-clipboard" | "out-printer" => {
            if name == "set-clipboard" || name == "out-printer" {
                return;
            }
            let targets = paths_of(&a, &["path", "filepath", "literalpath"], Some(0));
            let how = if name == "add-content" || a.has("append") { Write::Append } else if a.has("noclobber") { Write::Create } else { Write::Truncate };
            for t in &targets {
                if is_ps_drive(t) || is_registry(t) {
                    if is_registry(t) {
                        risky(cx, Flag::SystemChange, "changes the Windows registry");
                    }
                    continue;
                }
                write(cx, &cwd, t, how);
            }
            if targets.is_empty() && name != "tee-object" {
                cx.b.find(Flag::UnresolvedPath, Risk::Caution, "writes a file (cannot tell where)".to_string());
            }
        }
        "new-item" => {
            let base = a.get(&["path"]).or_else(|| a.pos.first().cloned());
            let leaf = a.get(&["name"]);
            let kind = a.get(&["itemtype", "type"]).map(|k| k.text.to_lowercase()).unwrap_or_default();
            if let Some(b) = base {
                if is_registry(&b) {
                    risky(cx, Flag::SystemChange, "creates Windows registry keys");
                } else if !is_ps_drive(&b) {
                    let target = match leaf {
                        Some(l) => Word { text: format!("{}/{}", b.text.trim_end_matches(['/', '\\']), l.text), exact: b.exact && l.exact, ..Default::default() },
                        None => b,
                    };
                    write(cx, &cwd, &target, if a.has("force") && kind != "directory" { Write::Truncate } else { Write::Create });
                }
            }
        }
        "copy-item" | "move-item" | "rename-item" | "expand-archive" | "compress-archive" => {
            let sources = paths_of(&a, &["path", "literalpath"], Some(0));
            let dest = a.get(&["destination", "destinationpath", "newname"]).or_else(|| a.pos.get(1).cloned());
            for s in &sources {
                if name == "move-item" || name == "rename-item" {
                    write(cx, &cwd, s, Write::Truncate);
                } else {
                    read(cx, &cwd, s);
                }
            }
            match (name, dest) {
                ("rename-item", _) => {}
                (_, Some(d)) => write(cx, &cwd, &d, Write::Truncate),
                (_, None) => write(cx, &cwd, &Word::lit("."), Write::Truncate),
            }
        }
        "get-content" | "import-csv" | "import-clixml" | "get-item" | "get-filehash" | "select-string" | "get-acl" | "get-itemproperty" | "get-itempropertyvalue" => {
            let p = if name == "select-string" { a.get(&["path", "literalpath"]).or_else(|| a.pos.get(1).cloned()) } else { a.get(&["path", "literalpath"]).or_else(|| a.pos.first().cloned()) };
            if let Some(p) = p {
                read(cx, &cwd, &p);
            }
        }
        "get-childitem" => {
            if a.pos.iter().chain(a.get(&["path"]).iter()).any(|p| p.text.to_lowercase().starts_with("env:")) {
                Builder::push_unique(&mut cx.b.secret_reads, "environment variables");
            }
        }
        "set-location" | "push-location" => {
            if let Some(p) = a.get(&["path", "literalpath"]).or_else(|| a.pos.first().cloned()) {
                let r = if p.exact { paths::resolve(&p.text, &st.cwd, cx.ctx) } else { vec![] };
                // The cd may fail and the script carries on: both folders are possible.
                st.cwd = match (&st.cwd, r.is_empty()) {
                    (Some(old), false) => {
                        let mut all = old.clone();
                        all.extend(r);
                        all.dedup();
                        if all.len() > 4 { None } else { Some(all) }
                    }
                    _ => None,
                };
            }
        }
        "pop-location" => st.cwd = None,
        "invoke-webrequest" | "invoke-restmethod" | "start-bitstransfer" | "test-netconnection" | "test-connection" | "resolve-dnsname" | "new-pssession" | "enter-pssession" | "send-mailmessage" | "connect-wsman" => {
            let target = a.get(&["uri", "source", "computername", "name", "targetname", "smtpserver"]).or_else(|| a.pos.first().cloned());
            network(cx, target.as_ref(), "talks to");
            let method = a.get(&["method"]).map(|m| m.text.to_lowercase()).unwrap_or_default();
            if matches!(method.as_str(), "post" | "put" | "patch" | "delete") || a.has("body") || a.has("infile") || a.has("form") || name == "send-mailmessage" {
                caution(cx, Flag::SendsData, "sends data to the server");
            }
            if let Some(f) = a.get(&["infile", "attachments"]) {
                read(cx, &cwd, &f);
            }
            if let Some(f) = a.get(&["outfile", "destination"]) {
                write(cx, &cwd, &f, Write::Truncate);
            }
            if matches!(name, "new-pssession" | "enter-pssession") {
                caution(cx, Flag::RunsCode, "opens a remote PowerShell session");
            }
        }
        "invoke-expression" => {
            risky(cx, Flag::DynamicCommand, "builds a command from text and runs it (Invoke-Expression)");
            if inv.stdin_piped {
                reads_code_from_stdin(cx, inv, "PowerShell");
            }
            if let Some(code) = a.get(&["command"]).or_else(|| a.pos.first().cloned()) {
                if code.exact && !code.text.is_empty() {
                    analyze_into(cx, &code.text.clone(), &cwd);
                }
            }
        }
        "invoke-command" => {
            if a.has("computername") || a.has("session") || a.has("hostname") {
                network(cx, a.get(&["computername", "hostname"]).as_ref(), "runs commands on");
                caution(cx, Flag::RunsCode, "runs a command on another machine");
            }
            if let Some(f) = a.get(&["filepath"]) {
                read(cx, &cwd, &f);
                caution(cx, Flag::RunsCode, "runs a PowerShell script file");
            }
        }
        "start-process" => {
            let verb_arg = a.get(&["verb"]).map(|v| v.text.to_lowercase()).unwrap_or_default();
            if verb_arg == "runas" {
                risky(cx, Flag::Privilege, "starts a program with administrator rights");
            }
            if a.get(&["windowstyle"]).is_some_and(|w| w.text.eq_ignore_ascii_case("hidden")) {
                caution(cx, Flag::Background, "starts a program in a hidden window");
            }
            match a.get(&["filepath"]).or_else(|| a.pos.first().cloned()) {
                Some(p) if p.exact => {
                    let mut argv = vec![p.clone()];
                    let list = a.get(&["argumentlist"]).or_else(|| a.pos.get(1).cloned());
                    if let Some(l) = list {
                        if l.exact {
                            argv.extend(l.text.split_whitespace().map(Word::lit));
                        } else {
                            argv.push(l);
                        }
                    }
                    caution(cx, Flag::RunsCode, &format!("starts {}", short(&p.text)));
                    classify::invocation(cx, &Invocation { argv, cwd: cwd.clone(), ..Default::default() });
                }
                _ => risky(cx, Flag::DynamicCommand, "starts a program that is only decided when it runs"),
            }
        }
        "invoke-item" => caution(cx, Flag::RunsCode, "opens a file with its default program"),
        "stop-process" => caution(cx, Flag::KillsProcess, "stops running programs"),
        "start-job" | "start-threadjob" => caution(cx, Flag::Background, "runs commands in the background"),
        "add-type" => {
            if a.has("typedefinition") || a.has("memberdefinition") || a.pos.iter().any(|p| p.text.contains("DllImport") || p.text.contains("class ")) || a.has("path") {
                caution(cx, Flag::InlineCode, "compiles and loads C# code written inside the command");
            }
        }
        "import-module" | "using-module" => {
            match a.get(&["name"]).or_else(|| a.pos.first().cloned()) {
                Some(m) if m.text.contains(['\\', '/']) || m.text.to_lowercase().ends_with(".psm1") || m.text.to_lowercase().ends_with(".dll") || m.text.to_lowercase().ends_with(".ps1") => {
                    read(cx, &cwd, &m);
                    caution(cx, Flag::RunsCode, &format!("loads code from {}", short(&m.text)));
                }
                _ => {}
            }
        }
        "new-object" => {
            if a.has("comobject") {
                caution(cx, Flag::RunsCode, "drives another Windows program (COM automation)");
            }
        }
        "set-itemproperty" | "new-itemproperty" | "set-item" => {
            match a.get(&["path", "literalpath"]).or_else(|| a.pos.first().cloned()) {
                Some(p) if is_registry(&p) => risky(cx, Flag::SystemChange, "changes the Windows registry"),
                Some(p) if p.text.to_lowercase().starts_with("wsman:") => risky(cx, Flag::SystemChange, "changes remote-management settings"),
                Some(p) if is_ps_drive(&p) => {
                    if let Some(env_name) = p.text.to_lowercase().strip_prefix("env:") {
                        let value = a.get(&["value"]).or_else(|| a.pos.get(1).cloned()).unwrap_or_default();
                        classify::env_var(cx, &cwd, env_name, &value);
                    }
                }
                Some(p) => write(cx, &cwd, &p, Write::Append),
                None => caution(cx, Flag::WritesFile, "changes a property of something earlier in the pipeline"),
            }
        }
        "set-executionpolicy" => risky(cx, Flag::SystemChange, "changes which PowerShell scripts are allowed to run"),
        "unblock-file" => caution(cx, Flag::SystemChange, "marks a downloaded file as safe to run"),
        "install-module" | "install-package" | "install-script" | "update-module" | "save-module" | "install-packageprovider" | "register-psrepository" | "set-psrepository" | "uninstall-module" | "uninstall-package" | "install-windowsfeature" | "add-windowscapability" | "enable-windowsoptionalfeature" | "disable-windowsoptionalfeature" | "add-appxpackage" | "remove-appxpackage" => {
            cx.b.find(Flag::Network, Risk::Caution, "talks to a package server".to_string());
            cx.b.remote_network = true;
            caution(cx, Flag::InstallsPackages, "installs or removes PowerShell modules or Windows features");
            risky(cx, Flag::SystemChange, "installs or changes software for the whole PC");
        }
        "find-module" | "find-package" | "find-script" => {
            cx.b.find(Flag::Network, Risk::Caution, "asks a package server for information".to_string());
            cx.b.remote_network = true;
        }
        _ => {
            // Known-dangerous families, then a default by verb.
            let n = noun;
            let machine = matches!(verb, "stop" | "restart" | "suspend" | "resume" | "start" | "set" | "new" | "remove") && matches!(n, "service" | "computer" | "vm" | "scheduledtask" | "netadapter" | "netfirewallrule" | "netfirewallprofile" | "localuser" | "localgroup" | "localgroupmember" | "mppreference" | "partition" | "volume" | "disk" | "smbshare" | "timezone" | "date" | "acl" | "aduser" | "adgroup" | "dnsclientserveraddress" | "netipaddress" | "netroute" | "psdrive" | "wmiinstance" | "ciminstance" | "eventlog" | "itempropert" | "bitlockervolume" | "vmswitch" | "netconnectionprofile" | "authenticodesignature" | "processmitigation" | "smbserverconfiguration" | "culture" | "winsystemlocale" | "winuserlanguagelist");
            let always = matches!(name, "stop-computer" | "restart-computer" | "format-volume" | "clear-disk" | "initialize-disk" | "clear-eventlog" | "limit-eventlog" | "remove-eventlog" | "register-scheduledtask" | "unregister-scheduledtask" | "register-scheduledjob" | "register-wmievent" | "register-cimindicationevent" | "set-wmiinstance" | "invoke-wmimethod" | "invoke-cimmethod" | "add-mppreference" | "remove-mppreference" | "add-localgroupmember" | "enable-psremoting" | "disable-netfirewallrule" | "enable-netfirewallrule" | "rename-computer" | "add-computer" | "remove-computer" | "checkpoint-computer" | "restore-computer" | "repair-volume" | "resize-partition" | "mount-diskimage" | "dismount-diskimage" | "enable-bitlocker" | "disable-bitlocker" | "enable-localuser" | "disable-localuser" | "import-certificate" | "import-pfxcertificate" | "export-certificate" | "disable-scheduledtask" | "enable-scheduledtask" | "start-scheduledtask" | "set-scheduledtask" | "new-scheduledtasktrigger" | "new-scheduledtaskaction" | "clear-history" | "wevtutil" | "set-strictmode-off" | "disable-windowserrorreporting" | "set-service" | "new-service" | "optimize-volume");
            if always || machine {
                if matches!(name, "new-scheduledtasktrigger" | "new-scheduledtaskaction" | "clear-history") {
                    caution(cx, Flag::SystemChange, "prepares a scheduled task or clears session history");
                } else {
                    risky(cx, Flag::SystemChange, &format!("changes this PC ({name})"));
                }
                return;
            }
            match verb {
                "get" | "test" | "select" | "where" | "foreach" | "sort" | "group" | "measure" | "format" | "out" | "write" | "convertto" | "convertfrom" | "compare" | "resolve" | "split" | "join" | "show" | "find" | "read" | "wait" | "receive" | "tee" | "trace" | "debug" | "confirm" | "assert" | "search" | "watch" | "skip" | "step" | "enter" | "exit" | "use" | "pop" | "push" => {}
                "set" | "new" | "add" | "clear" | "remove" if matches!(n, "variable" | "alias" | "strictmode" | "psdebug" | "psbreakpoint" | "timespan" | "guid" | "object" | "member" | "job" | "event" | "pssession" | "psreadlineoption" | "psreadlinekeyhandler" | "location" | "host" | "type" | "module" | "typedata" | "formatdata" | "psdrive" | "temporaryfile" | "itemproperty" | "content" | "history" | "error") => {}
                "start" if matches!(n, "sleep" | "transcript") => {}
                "stop" if matches!(n, "job" | "transcript") => {}
                "update" if matches!(n, "typedata" | "formatdata" | "help") => {}
                "import" | "export" if matches!(n, "csv" | "clixml" | "alias" | "localizeddata" | "powershelldatafile" | "modulemember" | "formatdata") => {}
                "remove" | "clear" | "uninstall" | "reset" | "unregister" | "disable" | "revoke" | "dismount" | "block" | "deny" | "unpublish" | "restart" | "suspend" | "stop" => {
                    risky(cx, Flag::UnknownProgram, &format!("runs {name}, a command Atoll does not know that removes, stops or resets something"));
                }
                _ => {
                    cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “{name}”, a PowerShell command Atoll does not know"));
                }
            }
        }
    }
}
