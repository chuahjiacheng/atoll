//! Programs with rules of their own: git, package managers, interpreters,
//! network tools, databases, cloud CLIs and system administration tools.

use std::sync::LazyLock;

use regex::Regex;

use crate::bash::{Invocation, Word};
use crate::classify::{delete, downloaded, runs_file, RunKind, host_of, invocation, is_local_host, is_opt, looks_like_url, network, prog_name, read, reads_code_from_stdin, shape, short, split, value_of, write, Cwd, Cx, Write};
use crate::model::{Builder, Flag, Risk};

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("valid pattern"));
    };
}

fn exact_text(w: Option<&Word>) -> Option<&str> {
    w.filter(|w| w.exact).map(|w| w.text.as_str())
}

/// Talks to a server the tool already knows (git remote, mail server): not this PC.
fn remote(cx: &mut Cx, note: &str) {
    cx.b.find(Flag::Network, Risk::Caution, note.to_string());
    cx.b.remote_network = true;
}

// ---------------------------------------------------------------- text tools

re!(SED_EXEC, r"(^|[;{}\n]|\d)\s*e(\s|$)|s(.).*?\x00");
// $(...), ${...}, $name and `...` in shell text.
re!(SHELL_FILL, r"\$\([^()]*(?:\([^()]*\)[^()]*)*\)|\$\{[^}]*\}|\$[A-Za-z_]\w*|`[^`]*`");
// Addresses followed by p, d or q only: prints lines, changes nothing.
re!(SED_PRINT_ONLY, r"^[\d,+~$!\s]*[pdq]?\s*$");
re!(SED_WRITE, r"(^|[;{}\n])\s*[0-9,$/]*\s*w\s+\S|/[gipmI0-9]*w\s+\S+\s*$");

fn sed_runs_commands(script: &str) -> bool {
    // GNU sed: an `e` command, or the `e` flag on a substitution (s/a/b/e).
    if Regex::new(r"(^|[;{}\n])\s*[0-9,$]*\s*e(\s|$)").unwrap().is_match(script) {
        return true;
    }
    let b: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let at_start = i == 0 || matches!(b[i - 1], ';' | '{' | '}' | '\n' | ' ' | '/' | '$') || b[i - 1].is_ascii_digit();
        if b[i] == 's' && at_start && i + 1 < b.len() && !b[i + 1].is_alphanumeric() && !b[i + 1].is_whitespace() {
            let d = b[i + 1];
            let mut j = i + 2;
            let mut seen = 0;
            while j < b.len() && seen < 2 {
                if b[j] == '\\' {
                    j += 1;
                } else if b[j] == d {
                    seen += 1;
                }
                j += 1;
            }
            if seen == 2 {
                let mut k = j;
                while k < b.len() && (b[k].is_ascii_alphanumeric()) {
                    if b[k] == 'e' {
                        return true;
                    }
                    if b[k] == 'w' {
                        break;
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn sed(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "sed", "", args);
    let _ = (&*SED_EXEC,);
    let a = split(args, &["-e", "--expression", "-f", "--file", "-l", "--line-length"]);
    let in_place = a.opts.iter().any(|o| (o.starts_with("-i") && !o.starts_with("--")) || o.starts_with("--in-place") || (!o.starts_with("--") && o[1..].contains('i') && !o.contains('=')));
    let mut scripts: Vec<Word> = vec![];
    for (i, w) in args.iter().enumerate() {
        if w.exact && matches!(w.text.as_str(), "-e" | "--expression") {
            if let Some(s) = args.get(i + 1) {
                scripts.push(s.clone());
            }
        }
    }
    let mut files = a.pos.clone();
    let has_script_file = a.any(&["-f", "--file"]);
    if scripts.is_empty() && !has_script_file && !files.is_empty() {
        scripts.push(files.remove(0));
    }
    if has_script_file {
        cx.b.find(Flag::RunsCode, Risk::Caution, "runs a sed script from a file".to_string());
    }
    for s in &scripts {
        if !s.exact {
            // `sed -n "$(grep -n x f | cut -d: -f1),+20p" f`: only line numbers are filled in.
            let filled = SHELL_FILL.replace_all(&s.text, "1");
            if !SED_PRINT_ONLY.is_match(&filled) {
                cx.b.find(Flag::DynamicCommand, Risk::Caution, "part of the sed script is filled in when it runs".to_string());
            }
        }
        if sed_runs_commands(&s.text) {
            cx.b.find(Flag::HiddenExec, Risk::Risky, "the sed script runs shell commands".to_string());
        }
        if SED_WRITE.is_match(&s.text) {
            cx.b.find(Flag::UnresolvedPath, Risk::Risky, "the sed script writes to a file named inside it".to_string());
        }
    }
    for f in &files {
        if in_place {
            write(cx, &cwd, f, Write::Truncate);
        } else {
            read(cx, &cwd, f);
        }
    }
}

re!(AWK_EXEC, r#"system\s*\(|\|\s*"|"\s*\|\s*getline|\|&|\|\s*getline"#);
re!(AWK_WRITE, r#"print[^;}\n]*>>?\s*("([^"]*)"|[A-Za-z_(])"#);

pub(crate) fn awk(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "awk", "", args);
    let a = split(args, &["-F", "-v", "-f", "--field-separator", "--assign", "--file"]);
    if a.any(&["-f", "--file"]) {
        cx.b.find(Flag::RunsCode, Risk::Caution, "runs an awk script from a file".to_string());
    }
    let mut pos = a.pos.clone();
    if !a.any(&["-f", "--file"]) && !pos.is_empty() {
        let script = pos.remove(0);
        if !script.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the awk program is only known when it runs".to_string());
        } else {
            if AWK_EXEC.is_match(&script.text) {
                cx.b.find(Flag::HiddenExec, Risk::Risky, "the awk program runs shell commands".to_string());
            }
            for c in AWK_WRITE.captures_iter(&script.text) {
                match c.get(2) {
                    Some(p) if !p.as_str().is_empty() => write(cx, &cwd, &Word::lit(p.as_str()), Write::Truncate),
                    _ => cx.b.find(Flag::UnresolvedPath, Risk::Risky, "the awk program writes to a file chosen when it runs".to_string()),
                }
            }
        }
    }
    for f in pos.iter().filter(|p| !p.text.contains('=')) {
        read(cx, &cwd, f);
    }
}

pub(crate) fn find(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    let (name, _) = prog_name(&inv.argv[0].text);
    shape(cx, &name, "", &[]);
    let is_expr = |w: &Word| w.exact && (w.text.starts_with('-') || matches!(w.text.as_str(), "(" | ")" | "!" | ","));
    let first_expr = args.iter().position(is_expr).unwrap_or(args.len());
    let mut roots: Vec<Word> = args[..first_expr].to_vec();
    if name.starts_with("fd") {
        // fd PATTERN [PATH...]; -x / -X run a command per result.
        roots = args.iter().filter(|w| !is_opt(w)).skip(1).cloned().collect();
    }
    if roots.is_empty() {
        roots.push(Word::lit("."));
    }
    let mut i = first_expr;
    if name.starts_with("fd") {
        i = 0;
    }
    while i < args.len() {
        let w = &args[i];
        let t = if w.exact { w.text.as_str() } else { "" };
        match t {
            "-delete" => {
                for r in &roots {
                    delete(cx, &cwd, r, true);
                }
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" | "-x" | "--exec" | "-X" | "--exec-batch" => {
                let end = args[i + 1..].iter().position(|e| e.exact && matches!(e.text.as_str(), ";" | "+" | "\\;")).map(|p| i + 1 + p).unwrap_or(args.len());
                let mut argv: Vec<Word> = args[i + 1..end].to_vec();
                let mut placed = false;
                for a in argv.iter_mut() {
                    if a.exact && a.text.contains("{}") {
                        // The match is somewhere under one of the start folders.
                        let root = roots[0].clone();
                        *a = Word { text: format!("{}/*", root.text.trim_end_matches('/')), exact: root.exact, glob: true, ..Default::default() };
                        placed = true;
                    }
                }
                if !placed && name.starts_with("fd") {
                    let root = roots[0].clone();
                    argv.push(Word { text: format!("{}/*", root.text.trim_end_matches('/')), exact: root.exact, glob: true, ..Default::default() });
                }
                if !argv.is_empty() {
                    let sub = Invocation { argv, cwd: cwd.clone(), ..Default::default() };
                    invocation(cx, &sub);
                }
                i = end;
            }
            "-fprint" | "-fprintf" | "-fls" | "-fprint0" => {
                if let Some(f) = args.get(i + 1) {
                    write(cx, &cwd, f, Write::Truncate);
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
}

pub(crate) fn tar(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "tar", "", args);
    let mut mode = ' ';
    let mut all_flags = String::new();
    for (i, w) in args.iter().enumerate() {
        if !w.exact {
            continue;
        }
        let t = w.text.as_str();
        if (i == 0 && !t.starts_with('-')) || (t.starts_with('-') && !t.starts_with("--")) {
            all_flags.push_str(t.trim_start_matches('-'));
        }
        match t {
            "--extract" | "--get" => mode = 'x',
            "--create" => mode = 'c',
            "--list" => mode = 't',
            "--append" | "--update" => mode = 'r',
            _ => {}
        }
        for bad in ["--to-command", "--checkpoint-action", "--use-compress-program", "--rsh-command", "--rmt-command", "--info-script", "--new-volume-script", "-I"] {
            if t == bad || t.starts_with(&format!("{bad}=")) {
                cx.b.find(Flag::HiddenExec, Risk::Risky, format!("tar is told to run another program ({bad})"));
            }
        }
        if t == "--remove-files" {
            cx.b.find(Flag::Delete, Risk::Risky, "deletes the files after adding them to the archive".to_string());
        }
        if t == "--absolute-names" || t == "-P" {
            cx.b.find(Flag::OutsideProject, Risk::Risky, "may write files anywhere on the disk (absolute names)".to_string());
        }
    }
    if mode == ' ' {
        mode = ['x', 'c', 't', 'r', 'u'].into_iter().find(|m| all_flags.contains(*m)).unwrap_or(' ');
    }
    let file = value_of(args, &["-f", "--file"]).or_else(|| if all_flags.contains('f') { args.iter().skip(1).find(|w| !is_opt(w)).cloned() } else { None });
    let dir = value_of(args, &["-C", "--directory"]);
    match mode {
        'x' => {
            let dest = dir.unwrap_or_else(|| Word::lit("."));
            write(cx, &cwd, &dest, Write::Truncate);
            if let Some(f) = &file {
                read(cx, &cwd, f);
            }
        }
        'c' | 'r' | 'u' => {
            if let Some(f) = &file {
                if f.text != "-" {
                    write(cx, &cwd, f, Write::Truncate);
                }
            }
            for p in split(args, &["-f", "--file", "-C", "--directory", "--exclude", "-T", "-X"]).pos.iter().skip(if args.first().is_some_and(|a| !a.text.starts_with('-')) { 1 } else { 0 }) {
                if Some(&p.text) != file.as_ref().map(|f| &f.text) {
                    read(cx, &cwd, p);
                }
            }
        }
        _ => {
            if let Some(f) = &file {
                read(cx, &cwd, f);
            }
        }
    }
}

pub(crate) fn archive(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    let a = split(args, &["-d", "-x", "-i", "-P", "-S", "--suffix", "-T", "-o", "-DestinationPath", "-Path", "-LiteralPath"]);
    match name {
        "unzip" | "unrar" | "expand-archive" => {
            if a.short('l') || a.short('t') || a.short('Z') || a.short('v') && name == "unzip" && a.pos.len() <= 1 && !a.short('d') {
                for p in &a.pos {
                    read(cx, &cwd, p);
                }
                return;
            }
            let dest = value_of(args, &["-d", "-DestinationPath"]).unwrap_or_else(|| Word::lit("."));
            write(cx, &cwd, &dest, if a.short('n') { Write::Create } else { Write::Truncate });
            if let Some(f) = a.pos.first() {
                read(cx, &cwd, f);
            }
        }
        "zip" | "rar" => {
            if a.any(&["-T", "-TT", "--unzip-command"]) && args.iter().any(|w| w.text == "-TT" || w.text == "--unzip-command") {
                cx.b.find(Flag::HiddenExec, Risk::Risky, "zip is told to run another program".to_string());
            }
            if a.short('m') || a.long("--move") {
                cx.b.find(Flag::Delete, Risk::Risky, "deletes the files after adding them to the archive".to_string());
            }
            if let Some((f, rest)) = a.pos.split_first() {
                write(cx, &cwd, f, Write::Truncate);
                for p in rest {
                    read(cx, &cwd, p);
                }
            }
        }
        "7z" | "7za" | "7zr" => {
            let sub = a.pos.first().map(|p| p.text.clone()).unwrap_or_default();
            match sub.as_str() {
                "l" | "t" | "i" | "h" => {}
                "d" => cx.b.find(Flag::Delete, Risk::Risky, "deletes files inside an archive".to_string()),
                "x" | "e" => {
                    let out = args.iter().find(|w| w.exact && w.text.starts_with("-o") && w.text.len() > 2).map(|w| Word { text: w.text[2..].to_string(), ..w.clone() });
                    write(cx, &cwd, &out.unwrap_or_else(|| Word::lit(".")), Write::Truncate);
                }
                _ => {
                    if let Some(f) = a.pos.get(1) {
                        write(cx, &cwd, f, Write::Truncate);
                    }
                }
            }
        }
        _ => {
            // gzip family: replaces each file with its (de)compressed form unless told to keep or print.
            if a.short('c') || a.short('l') || a.short('t') || a.long("--stdout") || a.long("--list") || a.long("--test") {
                for p in &a.pos {
                    read(cx, &cwd, p);
                }
            } else {
                for p in &a.pos {
                    write(cx, &cwd, p, Write::Truncate);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- network tools

const CURL_VALUED: &[&str] = &[
    "-o", "--output", "-d", "--data", "--data-raw", "--data-binary", "--data-urlencode", "--data-ascii", "-H", "--header", "-X", "--request", "-u", "--user",
    "-A", "--user-agent", "-e", "--referer", "-b", "--cookie", "-c", "--cookie-jar", "-F", "--form", "--form-string", "-T", "--upload-file", "-w", "--write-out",
    "-m", "--max-time", "--connect-timeout", "--retry", "--retry-delay", "--retry-max-time", "-x", "--proxy", "-K", "--config", "--cacert", "--cert", "--key",
    "-r", "--range", "-Y", "-y", "-z", "--resolve", "--url", "--json", "-E", "-t", "-Q", "--output-dir", "--max-redirs", "--limit-rate", "-C", "--continue-at",
    "--proxy-user", "--interface", "--dns-servers", "--unix-socket", "--oauth2-bearer", "--aws-sigv4", "--max-filesize", "--ciphers", "--tls-max", "-D", "--dump-header", "--trace", "--trace-ascii", "--stderr", "--netrc-file", "--connect-to", "--pinnedpubkey", "--capath", "--crlfile", "--egd-file", "--engine", "--hostpubmd5", "--keepalive-time", "--libcurl", "--local-port", "--mail-from", "--mail-rcpt", "--noproxy", "--pass", "--proto", "--proto-redir", "--proxy-header", "--pubkey", "--quote", "--random-file", "--speed-limit", "--speed-time", "--tftp-blksize", "--time-cond", "--tlsuser", "--tlspassword", "--expect100-timeout", "--happy-eyeballs-timeout-ms", "--abstract-unix-socket", "--alt-svc", "--hsts", "--request-target", "--service-name", "--socks5", "--socks4", "--socks4a", "--socks5-hostname", "--telnet-option", "--url-query", "--variable", "--rate", "--parallel-max",
];

pub(crate) fn curl(cx: &mut Cx, inv: &Invocation) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "curl", "", args);
    let a = split(args, CURL_VALUED);
    let mut urls: Vec<Word> = a.pos.clone();
    if let Some(u) = value_of(args, &["--url"]) {
        urls.push(u);
    }
    if urls.is_empty() {
        network(cx, None, "talks to");
    }
    for u in &urls {
        if u.exact && u.text.to_lowercase().starts_with("file:") {
            read(cx, &cwd, &Word::lit(u.text[5..].trim_start_matches("//")));
            continue;
        }
        network(cx, Some(u), "talks to");
    }
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        let t = if w.exact { w.text.as_str() } else { "" };
        let next = args.get(i + 1);
        match t {
            "-o" | "--output" | "-c" | "--cookie-jar" | "-D" | "--dump-header" | "--trace" | "--trace-ascii" | "--stderr" | "--libcurl" | "--hsts" | "--alt-svc" => {
                if let Some(f) = next {
                    if f.text != "-" {
                        write(cx, &cwd, f, Write::Truncate);
                        downloaded(cx, &cwd, f);
                    }
                }
            }
            "-O" | "--remote-name" | "--remote-name-all" | "-J" => write(cx, &cwd, &Word::lit("."), Write::Create),
            "-T" | "--upload-file" => {
                cx.b.find(Flag::SendsData, Risk::Caution, "uploads a file".to_string());
                if let Some(f) = next {
                    read(cx, &cwd, f);
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode" | "--data-ascii" | "--json" | "-F" | "--form" => {
                cx.b.find(Flag::SendsData, Risk::Caution, "sends data to the server".to_string());
                if let Some(v) = next {
                    let text = v.text.as_str();
                    let file = if t.starts_with("-F") || t == "--form" { text.split_once("=@").or_else(|| text.split_once("=<")).map(|p| p.1) } else { text.strip_prefix('@') };
                    if let Some(f) = file {
                        let f = f.split(';').next().unwrap_or(f);
                        read(cx, &cwd, &Word { text: f.to_string(), ..v.clone() });
                    }
                    if !v.exact && v.subst {
                        Builder::push_unique(&mut cx.b.secret_reads, "");
                    }
                }
            }
            "-X" | "--request" => {
                if let Some(m) = exact_text(next) {
                    match m.to_uppercase().as_str() {
                        "DELETE" => cx.b.find(Flag::SendsData, Risk::Caution, "asks the server to delete something".to_string()),
                        "POST" | "PUT" | "PATCH" => cx.b.find(Flag::SendsData, Risk::Caution, "sends data to the server".to_string()),
                        _ => {}
                    }
                }
            }
            "-K" | "--config" => {
                cx.b.find(Flag::HiddenExec, Risk::Caution, "takes more curl options from a file".to_string());
                if let Some(f) = next {
                    read(cx, &cwd, f);
                }
            }
            "-k" | "--insecure" => cx.b.find(Flag::Network, Risk::Caution, "skips the server identity check".to_string()),
            _ => {}
        }
        if CURL_VALUED.contains(&t) {
            i += 1;
        }
        i += 1;
    }
}

pub(crate) fn wget(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    let a = split(args, &["-O", "--output-document", "-o", "--output-file", "-P", "--directory-prefix", "--post-data", "--post-file", "--header", "-U", "--user-agent", "-t", "--tries", "-T", "--timeout", "-i", "--input-file", "-e", "--execute", "--body-data", "--body-file", "--method", "-d", "--dir", "-x"]);
    let urls: Vec<&Word> = a.pos.iter().filter(|p| !p.exact || looks_like_url(p) || p.text.contains('.') || p.text.contains(':')).collect();
    if urls.is_empty() {
        network(cx, None, "talks to");
    }
    for u in urls {
        network(cx, Some(u), "downloads from");
    }
    if a.any(&["--post-data", "--post-file", "--body-data", "--body-file"]) || (matches!(name, "http" | "https" | "xh" | "httpie") && a.pos.iter().any(|p| p.exact && matches!(p.text.to_uppercase().as_str(), "POST" | "PUT" | "PATCH" | "DELETE"))) {
        cx.b.find(Flag::SendsData, Risk::Caution, "sends data to the server".to_string());
    }
    for f in [value_of(args, &["--post-file"]), value_of(args, &["--body-file"])].into_iter().flatten() {
        read(cx, &cwd, &f);
    }
    if a.any(&["-e", "--execute", "--use-askpass"]) {
        cx.b.find(Flag::HiddenExec, Risk::Risky, "passes extra startup commands to wget".to_string());
    }
    if matches!(name, "wget" | "wget2" | "aria2c") {
        match value_of(args, &["-O", "--output-document"]) {
            Some(o) if o.text == "-" => {}
            Some(o) => {
                write(cx, &cwd, &o, Write::Truncate);
                downloaded(cx, &cwd, &o);
            }
            None => {
                let dir = value_of(args, &["-P", "--directory-prefix", "-d", "--dir"]).unwrap_or_else(|| Word::lit("."));
                if !a.long("--spider") {
                    write(cx, &cwd, &dir, Write::Create);
                }
            }
        }
    } else if let Some(o) = value_of(args, &["-o", "--output"]) {
        write(cx, &cwd, &o, Write::Truncate);
    }
}

fn remote_spec(w: &Word) -> Option<String> {
    if !w.exact {
        return None;
    }
    let t = w.text.as_str();
    if t.contains("://") {
        return host_of(t);
    }
    let (head, _) = t.split_once(':')?;
    // `C:\x` and `/c/x` are local.
    if head.len() <= 1 || head.contains('/') || head.contains('\\') {
        return None;
    }
    Some(head.rsplit('@').next().unwrap_or(head).to_lowercase())
}

pub(crate) fn ssh_family(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    for (i, w) in args.iter().enumerate() {
        if !w.exact {
            continue;
        }
        let opt = if w.text == "-o" { args.get(i + 1).map(|v| v.text.to_lowercase()).unwrap_or_default() } else if let Some(v) = w.text.strip_prefix("-o") { v.to_lowercase() } else { String::new() };
        if opt.starts_with("proxycommand") || opt.starts_with("localcommand") || opt.starts_with("permitlocalcommand") || opt.starts_with("knownhostscommand") {
            cx.b.find(Flag::HiddenExec, Risk::Risky, "tells ssh to run another program on this PC".to_string());
        }
        if opt.starts_with("stricthostkeychecking=no") || opt.starts_with("userknownhostsfile") {
            cx.b.find(Flag::Network, Risk::Caution, "skips the server identity check".to_string());
        }
        if name == "ssh" && matches!(w.text.as_str(), "-R" | "-D" | "-L" | "-w") {
            cx.b.find(Flag::Network, Risk::Risky, "opens a network tunnel through this PC".to_string());
        }
        if name == "rsync" {
            if w.text.starts_with("--delete") || w.text == "--remove-source-files" || w.text == "--del" {
                cx.b.find(Flag::Delete, Risk::Risky, "deletes files to make the two sides match".to_string());
            }
            if (w.text == "-e" || w.text == "--rsh") && args.get(i + 1).is_some_and(|v| !v.text.trim_start().starts_with("ssh")) {
                cx.b.find(Flag::HiddenExec, Risk::Risky, "tells rsync to run another program".to_string());
            }
        }
    }
    match name {
        "ssh" | "mosh" | "ssh-copy-id" => {
            let a = split(args, &["-p", "-i", "-o", "-l", "-F", "-J", "-L", "-R", "-D", "-b", "-c", "-E", "-e", "-m", "-O", "-Q", "-S", "-W", "-w", "-B", "-I"]);
            match a.pos.split_first() {
                Some((host, rest)) => {
                    let h = host.text.rsplit('@').next().unwrap_or("").to_string();
                    network(cx, Some(&Word { text: h.clone(), ..host.clone() }), "connects to");
                    if !rest.is_empty() {
                        cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs a command on the remote machine {h}"));
                    }
                    if name == "ssh-copy-id" {
                        cx.b.find(Flag::SensitivePath, Risk::Risky, "installs a login key on another machine".to_string());
                    }
                }
                None => network(cx, None, "connects to"),
            }
        }
        _ => {
            let a = split(args, &["-P", "-i", "-o", "-F", "-J", "-l", "-S", "-c", "-e", "--rsh", "--exclude", "--include", "--exclude-from", "--include-from", "--files-from", "--port", "--config", "--bwlimit", "-B", "--log-file", "--password-file", "--filter", "-f", "--max-size", "--min-size", "--partial-dir", "--temp-dir", "-T", "--chmod", "--chown", "--backup-dir", "--suffix"]);
            let pos: Vec<Word> = if name == "rclone" { a.pos.iter().skip(1).cloned().collect() } else { a.pos.clone() };
            let Some((dest, sources)) = pos.split_last() else {
                network(cx, None, "connects to");
                return;
            };
            let mut any_remote = false;
            for s in sources {
                match remote_spec(s) {
                    Some(h) => {
                        any_remote = true;
                        network(cx, Some(&Word::lit(&h)), "copies from");
                    }
                    None => read(cx, &cwd, s),
                }
            }
            match remote_spec(dest) {
                Some(h) => {
                    network(cx, Some(&Word::lit(&h)), "copies to");
                    cx.b.find(Flag::SendsData, Risk::Caution, format!("sends files to {h}"));
                }
                None => {
                    if !sources.is_empty() || name == "sftp" {
                        if name == "sftp" && sources.is_empty() {
                            network(cx, Some(dest), "connects to");
                        } else {
                            write(cx, &cwd, dest, Write::Truncate);
                        }
                    }
                    if !any_remote && name != "rsync" && name != "rclone" && !sources.is_empty() {
                        // scp between local paths: still only a copy.
                    }
                }
            }
        }
    }
}

pub(crate) fn raw_net(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    if name == "openssl" {
        let sub = exact_text(args.first()).unwrap_or("").to_string();
        shape(cx, "openssl", &sub, args);
        match sub.as_str() {
            "s_client" | "s_server" | "s_time" | "ocsp" => {
                let target = value_of(args, &["-connect"]);
                network(cx, target.as_ref(), "opens a raw connection to");
            }
            "enc" | "aes-256-cbc" | "base64" | "rsautl" | "pkeyutl" | "smime" | "cms" => {
                if let Some(o) = value_of(args, &["-out"]) {
                    write(cx, &cwd, &o, Write::Truncate);
                }
                if let Some(i) = value_of(args, &["-in"]) {
                    read(cx, &cwd, &i);
                }
            }
            "genrsa" | "genpkey" | "req" | "x509" | "pkcs12" | "ecparam" | "ca" | "dhparam" | "rsa" | "ec" | "pkey" => {
                for n in ["-out", "-keyout"] {
                    if let Some(o) = value_of(args, &[n]) {
                        // A new key file is expected output here, not a leak.
                        let zone_ok = Word { text: o.text.clone(), ..o.clone() };
                        cx.b.find(Flag::WritesFile, Risk::Caution, format!("writes the key or certificate {}", short(&zone_ok.text)));
                    }
                }
            }
            _ => {}
        }
        return;
    }
    shape(cx, name, "", args);
    let exec = args.iter().any(|w| w.exact && (matches!(w.text.as_str(), "-e" | "-c" | "--exec" | "--sh-exec" | "--lua-exec") || w.text.to_lowercase().starts_with("exec:") || w.text.to_lowercase().starts_with("system:") || w.text.to_lowercase().contains(",exec:") || w.text.to_lowercase().starts_with("exec ")));
    if exec && name != "ftp" && name != "lftp" && name != "telnet" {
        cx.b.find(Flag::HiddenExec, Risk::Risky, "connects a shell or program to the network (remote control)".to_string());
    }
    if name == "lftp" && args.iter().any(|w| w.exact && matches!(w.text.as_str(), "-e" | "-c")) {
        cx.b.find(Flag::SendsData, Risk::Caution, "runs file-transfer commands on a server".to_string());
    }
    let listen = args.iter().any(|w| w.exact && !w.text.starts_with("--") && w.text.starts_with('-') && w.text.contains('l')) && matches!(name, "nc" | "ncat" | "netcat");
    if listen {
        cx.b.find(Flag::Network, Risk::Risky, "opens a port that other machines can connect to".to_string());
        cx.b.remote_network = true;
        return;
    }
    let a = split(args, &["-p", "-s", "-w", "-x", "-X", "-i", "-q", "-e", "-c", "-u", "-P", "-T"]);
    let host = a.pos.iter().find(|p| !p.text.chars().all(|c| c.is_ascii_digit()));
    network(cx, host, "opens a raw connection to");
    if inv.stdin_piped || inv.redirects.iter().any(|r| r.op.starts_with('<')) || inv.stdin_text.is_some() {
        cx.b.find(Flag::SendsData, Risk::Caution, "sends data over a raw connection".to_string());
    }
}

// ---------------------------------------------------------------- git

const GIT_READ: &[&str] = &[
    "status", "log", "diff", "show", "rev-parse", "ls-files", "blame", "describe", "shortlog", "cat-file", "grep", "ls-tree", "merge-base", "check-ignore",
    "count-objects", "for-each-ref", "name-rev", "show-ref", "whatchanged", "version", "help", "rev-list", "var", "show-branch", "verify-commit", "cherry",
    "diff-tree", "diff-index", "diff-files", "range-diff", "annotate", "fsck", "check-attr", "check-ref-format", "get-tar-commit-id", "verify-tag", "verify-pack", "stripspace", "column", "check-mailmap", "interpret-trailers", "fmt-merge-msg", "patch-id", "merge-tree",
];
const GIT_WRITE: &[&str] = &[
    "add", "commit", "merge", "cherry-pick", "revert", "init", "mv", "apply", "am", "notes", "update-index", "symbolic-ref", "format-patch", "bundle",
    "sparse-checkout", "replace", "commit-tree", "write-tree", "read-tree", "hash-object", "mktag", "mktree", "pack-refs", "repack", "maintenance", "rerere", "merge-file", "checkout-index", "pack-objects", "unpack-objects", "index-pack", "multi-pack-index", "commit-graph", "fast-import", "stage",
];
const GIT_EXEC_KEYS: &[&str] = &[
    "core.sshcommand", "core.fsmonitor", "core.pager", "core.editor", "core.hookspath", "core.gitproxy", "core.askpass", "credential.helper", "diff.external",
    "protocol.ext.allow", "sequence.editor", "gpg.program", "gpg.ssh.program", "uploadpack.packobjectshook", "http.proxy", "core.attributesfile", "init.templatedir", "receive.procreceiverefs", "sendemail.smtpserver", "web.browser", "instaweb.httpd", "merge.tool", "diff.tool", "interactive.difffilter", "core.sparsecheckout", "safe.directory", "include.path", "url.",
];

fn git_exec_key(key: &str) -> bool {
    let k = key.to_lowercase();
    let k = k.split('=').next().unwrap_or("");
    GIT_EXEC_KEYS.iter().any(|e| k == *e || (e.ends_with('.') && k.starts_with(e)))
        || k.starts_with("alias.")
        || k.starts_with("filter.")
        || k.starts_with("pager.")
        || k.starts_with("includeif.")
        || (k.starts_with("diff.") && (k.ends_with(".textconv") || k.ends_with(".command")))
        || (k.starts_with("merge.") && k.ends_with(".driver"))
        || (k.starts_with("difftool.") && k.ends_with(".cmd"))
        || (k.starts_with("mergetool.") && k.ends_with(".cmd"))
        || (k.starts_with("remote.") && (k.ends_with(".uploadpack") || k.ends_with(".receivepack") || k.ends_with(".vcs") || k.ends_with(".proxy")))
        || (k.starts_with("submodule.") && k.ends_with(".update"))
        || k.starts_with("credential.")
        || k.starts_with("http.") && k.ends_with("sslverify")
}

pub(crate) fn git(cx: &mut Cx, inv: &Invocation) {
    let mut cwd = inv.cwd.clone();
    let all = &inv.argv[1..];
    // Global options before the subcommand.
    let mut i = 0;
    while i < all.len() {
        let w = &all[i];
        if !w.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "part of the git command is only known when it runs".to_string());
            shape(cx, "git", "", &[]);
            return;
        }
        match w.text.as_str() {
            "-C" => {
                if let Some(d) = all.get(i + 1) {
                    let r = if d.exact { crate::paths::resolve(&d.text, &cwd, cx.ctx) } else { vec![] };
                    cwd = if r.is_empty() { None } else { Some(r) };
                }
                i += 2;
            }
            "-c" | "--config-env" => {
                if let Some(kv) = all.get(i + 1) {
                    if git_exec_key(&kv.text) || !kv.exact {
                        cx.b.find(Flag::HiddenExec, Risk::Risky, format!("sets a git option that makes git run another program ({})", short(kv.text.split('=').next().unwrap_or(""))));
                    }
                }
                i += 2;
            }
            "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--super-prefix" => {
                if w.text == "--exec-path" {
                    cx.b.find(Flag::HiddenExec, Risk::Risky, "tells git to load its commands from another folder".to_string());
                }
                i += 2;
            }
            t if t.starts_with("--exec-path=") => {
                cx.b.find(Flag::HiddenExec, Risk::Risky, "tells git to load its commands from another folder".to_string());
                i += 1;
            }
            t if t.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let Some(sub_w) = all.get(i) else {
        shape(cx, "git", "", all);
        return;
    };
    if !sub_w.exact {
        cx.b.find(Flag::DynamicCommand, Risk::Risky, "the git command is only known when it runs".to_string());
        return;
    }
    let sub = sub_w.text.as_str();
    let args = &all[i + 1..];
    shape(cx, "git", sub, args);
    let a = split(args, &["-m", "--message", "-C", "-c", "-F", "--file", "--author", "--date", "-n", "--max-count", "--format", "--pretty", "-S", "-G", "--since", "--until", "-b", "-B", "--depth", "-o", "--origin", "-u", "--upstream", "--strategy", "-X", "--onto", "-s", "--source", "--output", "--reference", "--template", "-L", "--grep", "--exclude", "--before", "--after", "--diff-filter", "--abbrev", "--color", "--word-diff-regex", "--inter-hunk-context", "-U", "--unified", "--stat-width", "--config", "--separate-git-dir", "--branch", "-j", "--jobs", "--shallow-since", "--shallow-exclude", "--recurse-submodules", "--filter", "--cleanup", "--fixup", "--squash", "--reuse-message", "--reedit-message", "--gpg-sign", "--pathspec-from-file", "--trailer", "--push-option", "--receive-pack", "--exec", "--repo", "--signed", "--force-with-lease", "--committer-date-is-author-date", "--whitespace", "--directory", "-p", "--index-filter", "--tree-filter", "--msg-filter", "--env-filter", "--commit-filter", "--tag-name-filter", "--subdirectory-filter", "--parent-filter", "--state-branch", "-d", "--original", "--sort", "--contains", "--no-contains", "--merged", "--no-merged", "--points-at", "--list", "-x", "--extcmd", "-t", "--tool"]);
    let pos: Vec<&str> = a.pos.iter().filter(|p| p.exact).map(|p| p.text.as_str()).collect();
    let first = pos.first().copied().unwrap_or("");

    // Options that make git start another program, on any subcommand.
    for w in args {
        let t = w.text.as_str();
        if t.starts_with("--upload-pack") || t.starts_with("--receive-pack") || t.starts_with("--exec=") || t == "--exec" && sub != "rebase" || t.starts_with("ext::") || t.starts_with("--config-env") || t.starts_with("--upload-archive") {
            cx.b.find(Flag::HiddenExec, Risk::Risky, "passes git an option that runs another program".to_string());
        }
        if t.starts_with("--output") && GIT_READ.contains(&sub) {
            if let Some(o) = value_of(args, &["--output"]) {
                write(cx, &cwd, &o, Write::Truncate);
            }
        }
        if !w.exact && w.subst {
            // Text from another command, e.g. a commit message from a heredoc: fine.
        }
    }
    let risky = |cx: &mut Cx, flag: Flag, note: &str| cx.b.find(flag, Risk::Risky, note.to_string());
    let caution = |cx: &mut Cx, flag: Flag, note: &str| cx.b.find(flag, Risk::Caution, note.to_string());

    match sub {
        s if GIT_READ.contains(&s) => {}
        "ls-remote" => network(cx, a.pos.first(), "asks a git server at"),
        "branch" => {
            if a.short('D') || ((a.short('d') || a.long("--delete")) && (a.short('f') || a.long("--force"))) {
                risky(cx, Flag::HistoryRewrite, "force-deletes a branch, even if its work was never merged");
            } else if a.short('d') || a.long("--delete") {
                caution(cx, Flag::Delete, "deletes a branch that is already merged");
            } else if a.short('M') || a.short('C') || (a.short('f') || a.long("--force")) {
                risky(cx, Flag::HistoryRewrite, "moves or replaces a branch, discarding where it pointed");
            } else if !pos.is_empty() && !a.any(&["-l", "--list", "-a", "-r", "--all", "--remotes", "--show-current", "-v", "-vv", "--contains", "--merged", "--no-merged", "--points-at"]) || a.short('m') || a.short('c') || a.any(&["-u", "--set-upstream-to", "--unset-upstream", "--edit-description"]) {
                caution(cx, Flag::WritesFile, "creates or changes a branch");
            }
        }
        "tag" => {
            if a.short('d') || a.long("--delete") {
                caution(cx, Flag::Delete, "deletes a tag");
            } else if a.short('f') || a.long("--force") {
                risky(cx, Flag::HistoryRewrite, "moves an existing tag");
            } else if !pos.is_empty() && !a.any(&["-l", "--list", "-n", "--contains", "--points-at", "-v", "--verify"]) {
                caution(cx, Flag::WritesFile, "creates a tag");
            }
        }
        "remote" => match first {
            "" | "show" | "get-url" => {
                if first == "show" && pos.len() > 1 {
                    network(cx, None, "asks the git server about");
                }
            }
            _ => caution(cx, Flag::WritesFile, "changes which servers this repository talks to"),
        },
        "stash" => match first {
            "list" | "show" => {}
            "drop" | "clear" => risky(cx, Flag::Delete, "throws away stashed work for good"),
            _ => caution(cx, Flag::WritesFile, "moves uncommitted changes into or out of the stash"),
        },
        "config" => {
            let reading = a.any(&["--get", "--get-all", "--get-regexp", "--list", "-l", "--get-urlmatch", "--show-origin", "--show-scope", "--name-only", "--get-color", "--get-colorbool"]) || first == "get" || first == "list" || (pos.len() == 1 && !a.any(&["--unset", "--unset-all", "--remove-section", "--rename-section", "--add", "--replace-all", "-e", "--edit"]));
            if !reading {
                let key = if matches!(first, "set" | "unset" | "add" | "rename-section" | "remove-section" | "edit") { pos.get(1).copied().unwrap_or("") } else { first };
                if git_exec_key(key) {
                    risky(cx, Flag::HiddenExec, "sets a git option that makes git run another program");
                } else if a.any(&["--global", "--system"]) {
                    risky(cx, Flag::SystemChange, "changes git settings for every repository on this PC");
                } else {
                    caution(cx, Flag::WritesFile, "changes this repository's git settings");
                }
            }
        }
        "reflog" => {
            if matches!(first, "expire" | "delete" | "drop") {
                risky(cx, Flag::HistoryRewrite, "erases git's record of earlier states, which is how lost work is recovered");
            }
        }
        "worktree" => match first {
            "list" | "" => {}
            "remove" | "prune" => {
                if a.short('f') || a.long("--force") {
                    risky(cx, Flag::Delete, "force-removes a worktree, including uncommitted work in it");
                } else {
                    caution(cx, Flag::Delete, "removes a worktree folder");
                }
            }
            _ => {
                caution(cx, Flag::WritesFile, "adds or changes a worktree folder");
                if first == "add" {
                    if let Some(p) = a.pos.get(1) {
                        write(cx, &cwd, p, Write::Create);
                    }
                }
            }
        },
        "checkout" => {
            let dashdash = args.iter().any(|w| w.exact && w.text == "--");
            let pathish = pos.iter().any(|p| *p == "." || p.ends_with('/') || p.contains('*') || (p.contains('.') && !p.starts_with("origin/") && p.rsplit('.').next().is_some_and(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphabetic()))));
            if a.short('f') || a.long("--force") || dashdash || pathish || a.short('p') || a.any(&["--ours", "--theirs", "--merge", "-m"]) && !pos.is_empty() && a.any(&["--ours", "--theirs"]) {
                risky(cx, Flag::Overwrite, "discards uncommitted changes to files");
            } else {
                caution(cx, Flag::WritesFile, "switches branch (files change to match)");
            }
        }
        "switch" => {
            if a.short('f') || a.any(&["--force", "--discard-changes"]) {
                risky(cx, Flag::Overwrite, "switches branch and discards uncommitted changes");
            } else if a.short('C') || a.long("--force-create") {
                risky(cx, Flag::HistoryRewrite, "replaces an existing branch");
            } else {
                caution(cx, Flag::WritesFile, "switches branch (files change to match)");
            }
        }
        "restore" => {
            let staged_only = a.any(&["--staged", "-S"]) && !a.any(&["--worktree", "-W"]);
            if staged_only {
                caution(cx, Flag::WritesFile, "unstages files (their contents stay)");
            } else {
                risky(cx, Flag::Overwrite, "discards uncommitted changes to files");
            }
        }
        "reset" => {
            if a.long("--hard") {
                risky(cx, Flag::Overwrite, "discards all uncommitted changes and moves the branch (reset --hard)");
            } else if a.any(&["--merge", "--keep"]) {
                risky(cx, Flag::HistoryRewrite, "moves the branch and may discard changes");
            } else {
                caution(cx, Flag::WritesFile, "unstages changes or moves the branch pointer (files stay)");
            }
        }
        "clean" => {
            if a.short('n') || a.long("--dry-run") {
                // only lists
            } else {
                risky(cx, Flag::Delete, "deletes files git does not track, which cannot be brought back");
            }
        }
        "push" => {
            remote(cx, "uploads commits to the git server");
            cx.b.find(Flag::Publishes, Risk::Caution, "publishes commits to the git server".to_string());
            let force = a.short('f') || a.opts.iter().any(|o| o.starts_with("--force") || o == "--mirror" || o == "--delete" || o == "--prune") || a.short('d') || pos.iter().skip(1).any(|r| r.starts_with('+') || r.starts_with(':'));
            if force {
                risky(cx, Flag::HistoryRewrite, "can overwrite or delete history on the git server");
            }
        }
        "rm" => {
            let forced = a.short('f') || a.long("--force");
            let cached = a.long("--cached");
            for p in &a.pos {
                if cached {
                    cx.b.find(Flag::WritesFile, Risk::Caution, format!("stops tracking {} (the file stays)", short(&p.text)));
                } else {
                    delete(cx, &cwd, p, forced || a.short('r'));
                }
            }
        }
        "rebase" => {
            if matches!(first, "") && a.any(&["--abort", "--continue", "--skip", "--quit", "--edit-todo", "--show-current-patch"]) {
                caution(cx, Flag::WritesFile, "continues or stops a rebase in progress");
            } else {
                caution(cx, Flag::HistoryRewrite, "rewrites local commits onto another base");
            }
            for n in ["-x", "--exec"] {
                if let Some(c) = value_of(args, &[n]) {
                    if c.exact {
                        crate::nested_bash(cx, &c.text.clone(), &cwd);
                    } else {
                        risky(cx, Flag::DynamicCommand, "runs a command after each commit, only known when it runs");
                    }
                }
            }
        }
        "pull" | "fetch" => {
            remote(cx, "downloads commits from the git server");
            if sub == "pull" {
                caution(cx, Flag::WritesFile, "merges downloaded commits into the current branch");
            }
        }
        "clone" => {
            network(cx, a.pos.first(), "downloads a repository from");
            let dest = a.pos.get(1).cloned().unwrap_or_else(|| Word::lit("."));
            write(cx, &cwd, &dest, Write::Create);
        }
        "submodule" => {
            match first {
                "" | "status" | "summary" => {}
                "foreach" => {
                    let text = a.pos.iter().skip(1).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                    if a.pos.iter().skip(1).all(|w| w.exact) && !text.is_empty() {
                        crate::nested_bash(cx, &text, &None);
                    } else {
                        risky(cx, Flag::DynamicCommand, "runs a command in every submodule, only known when it runs");
                    }
                }
                _ => {
                    remote(cx, "downloads submodules from their git servers");
                    caution(cx, Flag::WritesFile, "adds or updates submodule folders");
                }
            }
        }
        "bisect" => {
            if first == "run" {
                let text = a.pos.iter().skip(1).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                if a.pos.iter().skip(1).all(|w| w.exact) && !text.is_empty() {
                    crate::nested_bash(cx, &text, &cwd);
                }
            }
            caution(cx, Flag::WritesFile, "checks out different commits to find a bug");
        }
        "gc" | "prune" => {
            if sub == "prune" || a.opts.iter().any(|o| o.starts_with("--prune")) || a.long("--aggressive") {
                risky(cx, Flag::HistoryRewrite, "permanently removes unreachable commits, which is how lost work is recovered");
            } else {
                caution(cx, Flag::WritesFile, "tidies the repository's storage");
            }
        }
        "filter-branch" | "filter-repo" => {
            risky(cx, Flag::HistoryRewrite, "rewrites the whole history of the repository");
            for n in ["--index-filter", "--tree-filter", "--msg-filter", "--env-filter", "--commit-filter", "--tag-name-filter", "--parent-filter"] {
                if let Some(c) = value_of(args, &[n]) {
                    if c.exact {
                        crate::nested_bash(cx, &c.text.clone(), &cwd);
                    }
                }
            }
        }
        "update-ref" => {
            if a.short('d') || a.long("--delete") {
                risky(cx, Flag::HistoryRewrite, "deletes a branch or other reference directly");
            } else {
                risky(cx, Flag::HistoryRewrite, "moves a branch or other reference directly");
            }
        }
        "archive" => {
            if a.any(&["--remote", "--exec"]) {
                risky(cx, Flag::HiddenExec, "passes git an option that runs another program");
            }
            if let Some(o) = value_of(args, &["-o", "--output"]) {
                write(cx, &cwd, &o, Write::Truncate);
            }
        }
        "difftool" | "mergetool" => {
            if a.any(&["-x", "--extcmd"]) {
                risky(cx, Flag::HiddenExec, "tells git to run another program as the diff tool");
            } else {
                caution(cx, Flag::RunsCode, "opens the configured diff or merge tool");
            }
        }
        "credential" | "credential-store" | "credential-cache" | "credential-manager" | "credential-manager-core" => {
            cx.b.find(Flag::SensitivePath, Risk::Risky, "reads or changes stored git passwords".to_string());
            Builder::push_unique(&mut cx.b.secret_reads, "git credentials");
        }
        "send-email" | "request-pull" | "imap-send" => {
            remote(cx, "sends email");
            caution(cx, Flag::SendsData, "sends patches by email");
        }
        "daemon" | "instaweb" | "http-backend" | "shell" | "upload-pack" | "receive-pack" | "upload-archive" | "remote-ext" | "remote-fd" | "cvsserver" => {
            risky(cx, Flag::Network, "starts a git server or low-level transfer program");
        }
        "lfs" => {
            if matches!(first, "ls-files" | "status" | "env" | "version" | "track" | "untrack") && first != "track" && first != "untrack" {
                // reads
            } else {
                remote(cx, "transfers large files with the git server");
                caution(cx, Flag::WritesFile, "changes large-file tracking or downloads large files");
            }
        }
        s if GIT_WRITE.contains(&s) => {
            let note = match s {
                "add" | "stage" => "stages files for the next commit",
                "commit" => "records a commit",
                "merge" => "merges another branch into this one",
                "cherry-pick" | "revert" => "adds commits that copy or undo earlier ones",
                "init" => "creates a git repository here",
                "mv" => "renames a tracked file",
                "apply" | "am" => "applies a patch to the files",
                _ => "changes the repository",
            };
            caution(cx, Flag::WritesFile, note);
            if s == "commit" && a.long("--amend") {
                caution(cx, Flag::HistoryRewrite, "replaces the last commit (amend)");
            }
            if matches!(s, "merge" | "cherry-pick" | "revert" | "am") && a.any(&["--abort", "--quit"]) {
                caution(cx, Flag::Overwrite, "abandons the operation in progress");
            }
        }
        other => {
            cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “git {other}”, which Atoll does not know (it may be an alias)"));
        }
    }
}

// ---------------------------------------------------------------- node / npm

const NPM_READ: &[&str] = &["ls", "list", "ll", "la", "why", "explain", "root", "prefix", "bin", "help", "fund", "query", "pkg", "version", "-v", "--version", "config", "get", "run-script-list", "licenses", "completion", "doctor", "cache-dir", "info-local", "bugs-local", "pm"];
const NPM_NET_READ: &[&str] = &["view", "info", "show", "v", "outdated", "audit", "ping", "search", "s", "find", "whoami", "dist-tags", "docs", "repo", "bugs", "home", "stars", "team-ls", "org-ls", "access-ls"];
const NPM_INSTALL: &[&str] = &["install", "i", "add", "ci", "update", "up", "upgrade", "remove", "rm", "uninstall", "un", "r", "dedupe", "ddp", "prune", "rebuild", "rb", "link", "ln", "unlink", "install-test", "it", "install-ci-test", "cit", "create", "init", "innit", "import", "patch", "patch-commit", "fetch", "upgrade-interactive", "set-script", "shrinkwrap", "isntall", "in", "ins", "inst", "insta", "instal", "isnt", "isnta", "isntal"];

pub(crate) fn node_pm(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    let a = split(args, &["--prefix", "-C", "--dir", "--cwd", "-w", "--workspace", "--filter", "-F", "--registry", "--cache", "--userconfig", "--loglevel", "-p", "--package", "-c", "--call", "--tag", "--otp", "--access", "--script-shell", "--node-options"]);
    for bad in ["--script-shell", "--node-options", "--registry", "--userconfig", "--globalconfig"] {
        if a.long(bad) && bad != "--registry" {
            cx.b.find(Flag::HiddenExec, Risk::Risky, format!("changes what npm runs ({bad})"));
        } else if a.long(bad) {
            cx.b.find(Flag::Network, Risk::Caution, "uses a package server other than the usual one".to_string());
        }
    }
    let global = a.short('g') || a.any(&["--global", "--location=global"]) || a.pos.first().is_some_and(|p| p.text == "global");
    let runner = matches!(name, "npx" | "pnpx" | "bunx") || (a.pos.first().is_some_and(|p| p.exact && matches!(p.text.as_str(), "exec" | "x" | "dlx")) && !matches!(name, "bun" if a.pos.first().is_some_and(|p| p.text == "x")));
    if runner {
        // npx <package> [args]: run a package's program, downloading it if needed.
        let skip = if matches!(name, "npx" | "pnpx" | "bunx") { 0 } else { 1 };
        let rest_at = args.iter().enumerate().filter(|(_, w)| !is_opt(w) && !(w.exact && w.text == "--")).map(|(i, _)| i).nth(skip);
        shape(cx, name, if skip == 1 { "exec" } else { "" }, &[]);
        if let Some(c) = value_of(args, &["-c", "--call"]) {
            if c.exact {
                crate::nested_bash(cx, &c.text.clone(), &cwd);
            } else {
                cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs a command that is only known when it runs".to_string());
            }
            return;
        }
        let Some(at) = rest_at else {
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs {name}"));
            return;
        };
        let pkg = &args[at];
        if !pkg.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the package to run is only known when it runs".to_string());
            return;
        }
        // foo@latest, @scope/foo@1.2 -> foo
        let mut bare = pkg.text.as_str();
        if let Some(i) = bare.rfind('@') {
            if i > 0 {
                bare = &bare[..i];
            }
        }
        let bare = bare.rsplit('/').next().unwrap_or(bare).to_string();
        let fetches = pkg.text.contains('@') || a.any(&["--yes", "-y", "-p", "--package"]) || pkg.text.contains("://") || pkg.text.starts_with("github:");
        if fetches || matches!(name, "bunx" | "pnpx") || skip == 1 {
            cx.b.find(Flag::InstallsPackages, Risk::Caution, format!("downloads the package {} if needed and runs it", short(&pkg.text)));
            cx.b.find(Flag::Network, Risk::Caution, "talks to the npm package server".to_string());
            cx.b.remote_network = true;
        }
        if pkg.text.contains("://") || pkg.text.starts_with("github:") || pkg.text.starts_with("git+") {
            cx.b.find(Flag::RemoteCode, Risk::Risky, "downloads code from an address and runs it".to_string());
        }
        let mut argv = vec![Word::lit(&bare)];
        argv.extend(args[at + 1..].iter().cloned());
        let sub = Invocation { argv, cwd: cwd.clone(), ..Default::default() };
        invocation(cx, &sub);
        return;
    }
    let sub = a.pos.first().filter(|p| p.exact).map(|p| p.text.clone()).unwrap_or_default();
    shape(cx, name, &sub, args);
    if name == "corepack" {
        cx.b.find(Flag::InstallsPackages, Risk::Caution, "sets up a package manager".to_string());
        return;
    }
    let second = a.pos.get(1).filter(|p| p.exact).map(|p| p.text.as_str()).unwrap_or("");
    match sub.as_str() {
        "" => {
            if a.opts.iter().any(|o| matches!(o.as_str(), "-v" | "--version" | "-h" | "--help")) {
                return;
            }
            if matches!(name, "yarn" | "pnpm" | "bun") && args.is_empty() || name == "yarn" {
                cx.b.find(Flag::InstallsPackages, Risk::Caution, "installs the project's packages".to_string());
                cx.b.find(Flag::Network, Risk::Caution, "talks to the npm package server".to_string());
                cx.b.remote_network = true;
            }
        }
        "config" | "c" | "get" | "set" | "pkg" => {
            let setting = matches!(sub.as_str(), "set") || matches!(second, "set" | "delete" | "rm" | "edit" | "fix");
            if setting {
                let key = a.pos.iter().skip(if sub == "set" { 1 } else { 2 }).map(|p| p.text.to_lowercase()).next().unwrap_or_default();
                if sub != "pkg" && (key.starts_with("script-shell") || key.starts_with("registry") || key.contains(":registry") || key.starts_with("node-options") || key.starts_with("prefix") || key.starts_with("ignore-scripts") || key.contains("_auth") || key.starts_with("git") || key.starts_with("shell")) {
                    cx.b.find(Flag::HiddenExec, Risk::Risky, "changes where packages come from or what npm runs".to_string());
                } else if sub == "pkg" {
                    cx.b.find(Flag::WritesFile, Risk::Caution, "edits package.json".to_string());
                } else {
                    cx.b.find(Flag::SystemChange, Risk::Risky, "changes npm settings for every project on this PC".to_string());
                }
            }
        }
        "audit" => {
            cx.b.find(Flag::Network, Risk::Caution, "talks to the npm package server".to_string());
            cx.b.remote_network = true;
            if second == "fix" || a.long("--fix") {
                cx.b.find(Flag::InstallsPackages, Risk::Caution, "changes installed packages to fix known problems".to_string());
            }
        }
        "cache" => {
            if matches!(second, "clean" | "clear" | "rm" | "delete") {
                cx.b.find(Flag::Delete, Risk::Caution, "clears the package download cache".to_string());
            }
        }
        "token" | "login" | "adduser" | "logout" | "profile" | "owner" | "access" | "team" | "org" | "hook" => {
            cx.b.find(Flag::SensitivePath, Risk::Risky, "changes or shows the npm account, tokens or who can publish".to_string());
            cx.b.remote_network = true;
        }
        "publish" | "dist-tag" | "deprecate" | "star" | "unstar" => {
            cx.b.find(Flag::Publishes, Risk::Caution, "publishes or changes a package on the public npm registry".to_string());
            cx.b.find(Flag::Network, Risk::Caution, "talks to the npm package server".to_string());
            cx.b.remote_network = true;
        }
        "unpublish" => {
            cx.b.find(Flag::Delete, Risk::Risky, "removes a published package from the npm registry".to_string());
            cx.b.remote_network = true;
        }
        s if NPM_READ.contains(&s) => {}
        s if NPM_NET_READ.contains(&s) => {
            cx.b.find(Flag::Network, Risk::Caution, "asks the npm package server for information".to_string());
            cx.b.remote_network = true;
        }
        s if NPM_INSTALL.contains(&s) => {
            cx.b.find(Flag::InstallsPackages, Risk::Caution, "adds, removes or updates packages (their install scripts run)".to_string());
            cx.b.find(Flag::Network, Risk::Caution, "talks to the npm package server".to_string());
            cx.b.remote_network = true;
            if global {
                cx.b.find(Flag::SystemChange, Risk::Risky, "installs or removes a program for the whole PC, not just this project".to_string());
            }
            for p in a.pos.iter().skip(1) {
                if !p.exact {
                    cx.b.find(Flag::DynamicCommand, Risk::Risky, "the package to install is only known when it runs".to_string());
                } else if p.text.contains("://") || p.text.starts_with("git+") || p.text.starts_with("github:") || p.text.ends_with(".tgz") && p.text.contains("http") {
                    cx.b.find(Flag::RemoteCode, Risk::Risky, format!("installs code straight from an address ({})", short(&p.text)));
                }
            }
        }
        "run" | "run-script" | "rum" | "urn" | "test" | "t" | "tst" | "start" | "stop" | "restart" | "dev" | "build" | "lint" | "format" | "typecheck" | "check" | "preview" | "serve" | "watch" | "tauri" | "app" => {
            let script = if matches!(sub.as_str(), "run" | "run-script" | "rum" | "urn") { second.to_string() } else { sub.clone() };
            if script.is_empty() {
                return; // `npm run` alone lists the scripts
            }
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the project's “{script}” script"));
        }
        other => {
            if matches!(name, "yarn" | "pnpm" | "bun") {
                // These run a script or a local program by bare name; bun also runs files.
                if name == "bun" && (other.contains('.') || other.contains('/')) {
                    cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(other)));
                } else {
                    cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the project's “{other}” script or tool"));
                }
            } else {
                cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “npm {other}”, which Atoll does not know"));
            }
        }
    }
}

// ---------------------------------------------------------------- inline code

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Js,
    Python,
    Other,
}

re!(JS_DELETE, r"rmSync|rmdirSync|unlinkSync|\bfs\.rm\b|fs\.promises\.rm|\.unlink\(|\.rmdir\(|rimraf|\.rm\(|fs\.truncate|emptyDir|removeSync");
re!(JS_EXEC, r#"child_process|execSync|spawnSync|execFile|(?:^|[^.\w])(?:exec|spawn|eval|fork)\(|new\s+Function|process\.binding|require\(\s*['"]vm['"]|process\.dlopen|Deno\.(run|Command)|Bun\.spawn|\$`"#);
re!(JS_NET, r#"\bfetch\(|https?\.(get|request)|require\(\s*['"](node:)?(https?|net|dgram|tls|dns)['"]|axios|XMLHttpRequest|WebSocket|from\s+['"](node:)?(https?|net)['"]|\bgot\(|undici"#);
re!(PY_DELETE, r"os\.remove|os\.unlink|shutil\.rmtree|os\.rmdir|\.unlink\(|rmtree|os\.removedirs|send2trash|\.rmdir\(|os\.truncate|\.truncate\(");
// `re.compile(`, `pattern.exec(`: methods, not the built-ins that run code.
re!(PY_EXEC, r"os\.system|subprocess|os\.popen|os\.exec|os\.spawn|\bpty\.|(?:^|[^.\w])(?:eval|exec|compile)\(|__import__|ctypes|importlib|pickle\.loads?|marshal\.loads|runpy|code\.interact|os\.startfile|webbrowser|__builtins__");
re!(PY_NET, r"urllib|requests\.|http\.client|\bsocket\b|httpx|aiohttp|ftplib|smtplib|paramiko|telnetlib|xmlrpc|websockets?\b|pycurl|boto3");
re!(ANY_EXEC, r"\bsystem\s*\(|\bexec\s*\(|`[^`]+`|\bpopen\b|IO\.popen|Kernel\.|%x[\{\(\[]|\bqx\b|shell_exec|passthru|proc_open|\beval\b|os\.execute|io\.popen|do shell script|Process\.spawn|Open3|\bspawn\b");
re!(ANY_DELETE, r"\bunlink\b|rmtree|File\.delete|FileUtils\.rm|\brmdir\b|rm_rf|remove_tree|os\.remove");
re!(ANY_NET, r"\bsocket\b|Net::|LWP|HTTP::|TCPSocket|open-uri|curl_|fsockopen|file_get_contents\(\s*['\x22]https?");
re!(ANY_WRITE, r#"open\s*\(?[^;]*['"]\s*>|File\.(write|open)|file_put_contents|\bfopen\b|io\.open|:w\b|>>?\s*['"]"#);
re!(DECODE, r"b64decode|base64\.decode|fromhex|codecs\.decode|\batob\(|Buffer\.from\([^)]*['\x22](base64|hex)['\x22]|unhexlify|decode\(\s*['\x22](base64|hex|rot13)|String\.fromCharCode|chr\(\s*\d+\s*\)\s*[+.]|\\x[0-9a-fA-F]{2}\\x[0-9a-fA-F]{2}\\x[0-9a-fA-F]{2}|pack\(\s*['\x22][Hh]|zlib\.decompress|unpack\(|MIME::Base64|base64_decode|gzinflate|str_rot13");
re!(SQL_DROP, r"(?i)\bdrop\s+(table|database|schema|index|view|user|role|column|trigger|function|sequence|type|extension|owned|materialized)\b|\btruncate\s+(table\s+)?[\w.\x22`\[]|\balter\s+table\b[^;]*\bdrop\b");
re!(SQL_EXEC, r"(?i)\bcopy\b[^;]*\b(to|from)\s+program\b|^\s*\\!|\\!\s|\.shell\b|\.system\b|xp_cmdshell|load_extension|into\s+(out|dump)file|lo_export|lo_import|pg_read_file|pg_ls_dir|\\copy\b[^;]*\bto\b|\.output\s|\.once\s|\.import\s|\bsystem\s+\w|\\o\s|\.load\s");
re!(SQL_CHANGE, r"(?i)\b(insert\s+into|update\s+[\w.\x22`\[]+\s+set|delete\s+from|alter\s+(table|database|user|role|system)|create\s+(table|database|index|user|role|schema|view|trigger|function|extension|or\s+replace)|grant\s|revoke\s|replace\s+into|vacuum|reindex|attach\s|detach\s|merge\s+into|upsert|rename\s+to|comment\s+on|set\s+(global|password)|flush\s|kill\s+\d|call\s+\w|do\s+\$)");

/// SQL text: what it does to the data.
pub(crate) fn sql(cx: &mut Cx, text: &str) {
    if SQL_EXEC.is_match(text) {
        cx.b.find(Flag::HiddenExec, Risk::Risky, "the SQL runs programs or reads and writes files on the machine".to_string());
    }
    let lower = text.to_lowercase();
    let mut unbounded_delete = false;
    for stmt in lower.split(';') {
        if let Some(i) = stmt.find("delete from") {
            if !stmt[i..].contains("where") {
                unbounded_delete = true;
            }
        }
        if let Some(i) = stmt.find("update ") {
            if stmt[i..].contains(" set ") && !stmt[i..].contains("where") {
                unbounded_delete = true;
            }
        }
    }
    if SQL_DROP.is_match(text) || unbounded_delete {
        cx.b.find(Flag::Database, Risk::Risky, "the SQL drops, empties or rewrites whole tables".to_string());
        cx.b.find(Flag::Delete, Risk::Risky, "deletes data from a database".to_string());
    } else if SQL_CHANGE.is_match(text) {
        cx.b.find(Flag::Database, Risk::Caution, "the SQL changes data or structure in a database".to_string());
    }
}

/// Code passed on the command line (`python -c`, `node -e`, a heredoc...).
/// It is pattern-matched, not parsed, so it is never eligible for auto-allow.
pub(crate) fn inline_code(cx: &mut Cx, cwd: &Cwd, lang: Lang, what: &str, code: &str) {
    cx.b.find(Flag::InlineCode, Risk::Caution, format!("runs {what} code written inside the command"));
    let (del, exec, net): (&Regex, &Regex, &Regex) = match lang {
        Lang::Js => (&JS_DELETE, &JS_EXEC, &JS_NET),
        Lang::Python => (&PY_DELETE, &PY_EXEC, &PY_NET),
        Lang::Other => (&ANY_DELETE, &ANY_EXEC, &ANY_NET),
    };
    if del.is_match(code) {
        cx.b.find(Flag::Delete, Risk::Risky, format!("the {what} code deletes files"));
    }
    let execs = exec.is_match(code);
    if execs {
        cx.b.find(Flag::HiddenExec, Risk::Risky, format!("the {what} code starts other programs or builds code to run"));
    }
    if DECODE.is_match(code) {
        if execs {
            cx.b.find(Flag::Obfuscated, Risk::Risky, format!("the {what} code decodes hidden text and runs it"));
        } else {
            cx.b.find(Flag::Obfuscated, Risk::Caution, format!("the {what} code decodes encoded text"));
        }
    }
    if net.is_match(code) {
        let host = URL_IN_TEXT.captures(code).and_then(|c| c.get(0)).and_then(|m| host_of(m.as_str()));
        match host {
            Some(h) if is_local_host(&h) => cx.b.find(Flag::Network, Risk::Safe, format!("the {what} code talks to this computer ({h})")),
            Some(h) => {
                Builder::push_unique(&mut cx.b.hosts, &h);
                cx.b.find(Flag::Network, Risk::Caution, format!("the {what} code talks to {h}"));
                cx.b.remote_network = true;
            }
            None => {
                cx.b.find(Flag::Network, Risk::Caution, format!("the {what} code uses the network"));
                cx.b.remote_network = true;
            }
        }
    }
    code_writes(cx, cwd, lang, what, code);
    // Secrets read by the code, for the "read a secret and use the network" check.
    for m in SECRET_IN_TEXT.find_iter(code) {
        let name = if m.as_str().contains("environ") || m.as_str().contains("process.env") { "the environment variables" } else { m.as_str() };
        cx.b.find(Flag::SensitivePath, Risk::Caution, format!("the {what} code reads {name}, which can hold secrets"));
        Builder::push_unique(&mut cx.b.secret_reads, name);
    }
    if SQL_DROP.is_match(code) {
        sql(cx, code);
    }
}

re!(URL_IN_TEXT, r#"https?://[^\s'"`)<>]+"#);
re!(SECRET_IN_TEXT, r"\.ssh/[\w.]+|\.credentials\.json|id_rsa|id_ed25519|\.aws/credentials|\.env\b|\.npmrc|\.netrc|\.git-credentials|/etc/shadow|\.kube/config|Login Data|os\.environ\b(?:\s*\)|\s*$|\.copy|\.items)|process\.env\s*\)|JSON\.stringify\(\s*process\.env");

pub(crate) fn node(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    let valued = ["-r", "--require", "--import", "--loader", "--experimental-loader", "--env-file", "--inspect-port", "--max-old-space-size", "--conditions", "-C", "--input-type", "--title", "--watch-path", "--test-name-pattern", "--test-reporter", "--allow-read", "--allow-write", "--allow-net", "--allow-env", "--allow-run", "--config", "-c", "--import-map", "--lock"];
    let mut i = 0;
    let deno_sub = if name == "deno" { exact_text(args.first()).unwrap_or("").to_string() } else { String::new() };
    if name == "deno" {
        match deno_sub.as_str() {
            "run" | "test" | "bench" | "task" | "serve" | "eval" | "repl" => i = 1,
            "fmt" | "lint" | "check" | "doc" | "info" | "types" | "--version" | "-V" | "completions" => {
                if deno_sub == "fmt" && !args.iter().any(|w| w.text == "--check") {
                    cx.b.find(Flag::WritesFile, Risk::Caution, "reformats source files in place".to_string());
                }
                return;
            }
            "install" | "add" | "remove" | "cache" | "upgrade" | "compile" | "publish" | "uninstall" => {
                cx.b.find(Flag::InstallsPackages, Risk::Caution, "installs or caches packages".to_string());
                cx.b.find(Flag::Network, Risk::Caution, "talks to a package server".to_string());
                cx.b.remote_network = true;
                return;
            }
            _ => {}
        }
    }
    while i < args.len() {
        let w = &args[i];
        if !w.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("what {name} runs is only known when it runs"));
            return;
        }
        let t = w.text.as_str();
        match t {
            "-e" | "--eval" | "-p" | "--print" | "-pe" => {
                match args.get(i + 1) {
                    Some(code) if code.exact => inline_code(cx, &cwd, Lang::Js, "JavaScript", &code.text.clone()),
                    Some(code) => {
                        inline_code(cx, &cwd, Lang::Js, "JavaScript", &code.text.clone());
                        if code.subst {
                            cx.b.find(Flag::DynamicCommand, Risk::Risky, "part of the JavaScript comes from another command".to_string());
                        }
                    }
                    None => {}
                }
                return;
            }
            "--check" | "-c" if name != "deno" => return,
            "-v" | "--version" | "-h" | "--help" | "--v8-options" | "--completion-bash" => return,
            "--test" => {
                cx.b.find(Flag::RunsCode, Risk::Caution, "runs the project's tests".to_string());
                return;
            }
            "-" => break,
            "-r" | "--require" | "--import" | "--loader" | "--experimental-loader" => {
                cx.b.find(Flag::RunsCode, Risk::Caution, "loads an extra module before the script".to_string());
                i += 2;
                continue;
            }
            _ if t.starts_with("--inspect") => {
                cx.b.find(Flag::Network, Risk::Caution, "opens a debugging port".to_string());
            }
            _ if t.starts_with("--eval=") || t.starts_with("--print=") => {
                inline_code(cx, &cwd, Lang::Js, "JavaScript", t.split_once('=').map(|p| p.1).unwrap_or(""));
                return;
            }
            _ if t.starts_with('-') => {
                if valued.contains(&t) {
                    i += 1;
                }
            }
            _ => {
                if looks_like_url(w) {
                    network(cx, Some(w), "downloads code from");
                    cx.b.find(Flag::RemoteCode, Risk::Risky, "downloads a script and runs it".to_string());
                } else {
                    read(cx, &cwd, w);
                    cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(t)));
                    runs_file(cx, &cwd, w, RunKind::Js);
                }
                return;
            }
        }
        i += 1;
    }
    if let Some(text) = &inv.stdin_text {
        inline_code(cx, &cwd, Lang::Js, "JavaScript", &text.clone());
    } else if inv.stdin_piped {
        reads_code_from_stdin(cx, inv, name);
    } else {
        cx.b.find(Flag::RunsCode, Risk::Caution, format!("starts {name}"));
    }
}

pub(crate) fn python(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, "python", "", args);
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        if !w.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "what Python runs is only known when it runs".to_string());
            return;
        }
        let t = w.text.as_str();
        match t {
            "-c" => {
                if let Some(code) = args.get(i + 1) {
                    inline_code(cx, &cwd, Lang::Python, "Python", &code.text.clone());
                    if code.subst {
                        cx.b.find(Flag::DynamicCommand, Risk::Risky, "part of the Python code comes from another command".to_string());
                    }
                }
                return;
            }
            "-m" => {
                let module = exact_text(args.get(i + 1)).unwrap_or("").to_string();
                let rest = args.get(i + 2..).unwrap_or(&[]);
                match module.as_str() {
                    "pip" | "ensurepip" | "pipx" | "uv" => {
                        let mut argv = vec![Word::lit("pip")];
                        argv.extend(rest.iter().cloned());
                        python_pm(cx, &Invocation { argv, cwd: cwd.clone(), ..Default::default() }, "pip");
                    }
                    "http.server" | "SimpleHTTPServer" | "smtpd" | "pydoc" if module != "pydoc" || rest.iter().any(|r| r.text == "-p" || r.text == "-b") => {
                        cx.b.find(Flag::Network, Risk::Caution, "starts a web server that shares this folder".to_string());
                        if rest.iter().any(|r| r.exact && (r.text == "0.0.0.0" || r.text == "::")) || !rest.iter().any(|r| r.text == "--bind" || r.text == "-b") {
                            cx.b.find(Flag::Network, Risk::Caution, "other machines on the network can reach it".to_string());
                        }
                    }
                    "json.tool" | "py_compile" | "compileall" | "tokenize" | "ast" | "site" | "sysconfig" | "platform" | "calendar" | "this" | "zipfile" | "tarfile" | "base64" | "timeit" | "tabnanny" => {}
                    "venv" | "virtualenv" => {
                        if let Some(d) = rest.iter().find(|r| !is_opt(r)) {
                            write(cx, &cwd, d, Write::Create);
                        }
                    }
                    "" => cx.b.find(Flag::DynamicCommand, Risk::Risky, "the Python module to run is only known when it runs".to_string()),
                    other => cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the Python module {other}")),
                }
                return;
            }
            "-V" | "--version" | "-h" | "--help" | "-VV" | "-0" | "--list" | "-0p" => return,
            "-" => break,
            "-W" | "-X" | "--check-hash-based-pycs" => i += 1,
            _ if t.starts_with('-') => {}
            _ => {
                read(cx, &cwd, w);
                cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(t)));
                runs_file(cx, &cwd, w, RunKind::Python);
                return;
            }
        }
        i += 1;
    }
    if let Some(text) = &inv.stdin_text {
        inline_code(cx, &cwd, Lang::Python, "Python", &text.clone());
    } else if inv.stdin_piped {
        reads_code_from_stdin(cx, inv, name);
    } else {
        cx.b.find(Flag::RunsCode, Risk::Caution, "starts Python".to_string());
    }
}

/// perl, ruby, php and other interpreters.
pub(crate) fn script_runner(cx: &mut Cx, inv: &Invocation, name: &str) -> bool {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    let a = split(args, &[]);
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        let t = w.text.as_str();
        let inline = w.exact && t.starts_with('-') && !t.starts_with("--") && (t[1..].contains('e') || t[1..].contains('E') || (name == "php" && t == "-r") || (name == "osascript" && t == "-e")) && !(name == "php" && t != "-r") || (w.exact && matches!(t, "--eval" | "-e" | "--execute"));
        if inline {
            match args.get(i + 1) {
                Some(code) => {
                    inline_code(cx, &cwd, Lang::Other, name, &code.text.clone());
                    if !code.exact {
                        cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("part of the {name} code is only known when it runs"));
                    }
                }
                None => {}
            }
            if name == "perl" && (t[1..].contains('i')) {
                for f in a.pos.iter().skip(1) {
                    write(cx, &cwd, f, Write::Truncate);
                }
            }
            return true;
        }
        if w.exact && (t == "-v" || t == "--version" || t == "-h" || t == "--help") {
            return true;
        }
        if !is_opt(w) {
            if !w.exact {
                cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("what {name} runs is only known when it runs"));
            } else {
                read(cx, &cwd, w);
                cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs the script {}", short(t)));
                runs_file(cx, &cwd, w, RunKind::Other);
            }
            return true;
        }
        i += 1;
    }
    if let Some(text) = &inv.stdin_text {
        inline_code(cx, &cwd, Lang::Other, name, &text.clone());
    } else if inv.stdin_piped {
        reads_code_from_stdin(cx, inv, name);
    } else {
        cx.b.find(Flag::RunsCode, Risk::Caution, format!("starts {name}"));
    }
    true
}

pub(crate) fn python_pm(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    let a = split(args, &["-r", "--requirement", "-i", "--index-url", "--extra-index-url", "-t", "--target", "-c", "--constraint", "-e", "--editable", "--python", "-p", "-n", "--name", "--prefix", "-f", "--find-links", "--with", "--from", "--directory", "--project"]);
    let sub = a.pos.first().filter(|p| p.exact).map(|p| p.text.clone()).unwrap_or_default();
    shape(cx, name, &sub, args);
    let second = a.pos.get(1).filter(|p| p.exact).map(|p| p.text.as_str()).unwrap_or("");
    let net = |cx: &mut Cx| {
        cx.b.find(Flag::Network, Risk::Caution, "talks to a Python package server".to_string());
        cx.b.remote_network = true;
    };
    match sub.as_str() {
        "list" | "show" | "freeze" | "check" | "--version" | "-V" | "help" | "inspect" | "debug" | "tree" | "info" | "env" | "version" | "hash" | "" => {
            if matches!(name, "conda" | "mamba") && sub == "env" && matches!(second, "remove" | "create" | "update") {
                cx.b.find(Flag::InstallsPackages, Risk::Caution, "creates, changes or removes a Python environment".to_string());
            }
        }
        "config" | "cache" | "index" | "search" | "download" | "wheel" | "lock" | "export" | "sync" | "pip" | "venv" | "build" | "init" | "new" | "self" | "tool" | "python" | "publish" | "add" | "remove" | "update" | "upgrade" | "install" | "uninstall" | "create" | "activate" | "deactivate" | "clean" | "ensurepath" | "inject" | "reinstall" | "reinstall-all" | "upgrade-all" | "uninstall-all" => {
            match sub.as_str() {
                "search" | "index" => net(cx),
                "config" => {
                    if matches!(second, "set" | "unset" | "edit" | "--add" | "--set" | "--remove") {
                        cx.b.find(Flag::SystemChange, Risk::Risky, "changes Python packaging settings for every project".to_string());
                    }
                }
                "cache" | "clean" => {
                    if matches!(second, "purge" | "remove" | "clean" | "prune" | "--all" | "-a" | "") && sub == "clean" || matches!(second, "purge" | "remove" | "clean" | "prune") {
                        cx.b.find(Flag::Delete, Risk::Caution, "clears the package download cache".to_string());
                    }
                }
                "publish" => {
                    net(cx);
                    cx.b.find(Flag::Publishes, Risk::Caution, "publishes a package to a public registry".to_string());
                }
                "venv" | "init" | "new" | "build" | "lock" | "export" | "wheel" | "download" | "activate" | "deactivate" => {
                    cx.b.find(Flag::WritesFile, Risk::Caution, "creates files for the Python project or environment".to_string());
                    if matches!(sub.as_str(), "lock" | "download" | "wheel" | "build") {
                        net(cx);
                    }
                }
                "pip" => {
                    let mut argv = vec![Word::lit("pip")];
                    argv.extend(args.iter().skip_while(|w| w.text != "pip").skip(1).cloned());
                    python_pm(cx, &Invocation { argv, cwd: cwd.clone(), ..Default::default() }, "pip");
                }
                _ => {
                    net(cx);
                    cx.b.find(Flag::InstallsPackages, Risk::Caution, "adds, removes or updates Python packages (their setup code runs)".to_string());
                    if matches!(name, "pipx") || a.any(&["--user", "--system", "--break-system-packages", "--global", "-g"]) || matches!(sub.as_str(), "self" | "tool" | "python" | "ensurepath") {
                        cx.b.find(Flag::SystemChange, Risk::Risky, "installs or removes a program for the whole PC, not just this project".to_string());
                    }
                    if a.any(&["-i", "--index-url", "--extra-index-url", "--trusted-host", "-f", "--find-links"]) {
                        cx.b.find(Flag::Network, Risk::Risky, "takes packages from a server other than the usual one".to_string());
                    }
                    for p in a.pos.iter().skip(1) {
                        if !p.exact {
                            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the package to install is only known when it runs".to_string());
                        } else if p.text.contains("://") || p.text.starts_with("git+") {
                            cx.b.find(Flag::RemoteCode, Risk::Risky, format!("installs code straight from an address ({})", short(&p.text)));
                        }
                    }
                    if let Some(t) = value_of(args, &["-t", "--target", "--prefix"]) {
                        write(cx, &cwd, &t, Write::Truncate);
                    }
                }
            }
        }
        "run" | "x" | "exec" | "runpip" | "shell" => {
            // uv run / poetry run / pipx run <program> [args]
            let at = args.iter().position(|w| w.exact && w.text == sub).map(|p| p + 1).unwrap_or(args.len());
            let rest: Vec<Word> = args[at..].iter().skip_while(|w| is_opt(w)).cloned().collect();
            if matches!(name, "pipx") || sub == "x" {
                net(cx);
                cx.b.find(Flag::InstallsPackages, Risk::Caution, "downloads a Python program if needed and runs it".to_string());
            }
            if rest.is_empty() {
                cx.b.find(Flag::RunsCode, Risk::Caution, format!("starts a shell or program through {name}"));
            } else {
                invocation(cx, &Invocation { argv: rest, cwd: cwd.clone(), stdin_piped: inv.stdin_piped, stdin_text: inv.stdin_text.clone(), upstream: inv.upstream.clone(), ..Default::default() });
            }
        }
        other => cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “{name} {other}”, which Atoll does not know")),
    }
}

pub(crate) fn rust_go(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    let skip_toolchain = args.first().is_some_and(|a| a.exact && a.text.starts_with('+'));
    let args = if skip_toolchain { &args[1..] } else { args };
    let a = split(args, &["-p", "--package", "--bin", "--example", "--test", "--bench", "--features", "-F", "--target", "--target-dir", "--manifest-path", "--profile", "-j", "--jobs", "--color", "--message-format", "--config", "-Z", "--limit", "--version", "--git", "--branch", "--tag", "--rev", "--path", "--root", "--registry", "--index", "-o", "-C", "-mod", "-tags", "-run", "-bench", "-count", "-timeout", "-ldflags", "-gcflags", "--toolchain", "--component", "-c", "-t"]);
    let sub = a.pos.first().filter(|p| p.exact).map(|p| p.text.clone()).unwrap_or_default();
    shape(cx, name, &sub, args);
    let second = a.pos.get(1).filter(|p| p.exact).map(|p| p.text.as_str()).unwrap_or("");
    let net = |cx: &mut Cx, what: &str| {
        cx.b.find(Flag::Network, Risk::Caution, what.to_string());
        cx.b.remote_network = true;
    };
    if args.iter().any(|w| w.exact && (w.text == "--config" || w.text.starts_with("--config="))) && name == "cargo" {
        cx.b.find(Flag::HiddenExec, Risk::Risky, "overrides cargo settings from the command line".to_string());
    }
    match (name, sub.as_str()) {
        (_, "") | (_, "version") | (_, "help") | ("cargo", "tree" | "metadata" | "locate-project" | "pkgid" | "verify-project" | "read-manifest" | "report") | ("go", "env" | "list" | "doc" | "vet") => {
            if name == "go" && sub == "env" && a.short('w') {
                cx.b.find(Flag::SystemChange, Risk::Risky, "changes Go settings for every project".to_string());
            }
        }
        ("rustup", "show" | "which" | "doc" | "completions" | "check") => {}
        ("rustup", "toolchain" | "target" | "component" | "override" | "default" | "update" | "self" | "install" | "uninstall" | "set" | "run") => {
            if matches!(second, "list" | "") && !matches!(sub.as_str(), "update" | "install" | "uninstall" | "default" | "run") {
                return;
            }
            if sub == "run" {
                let rest: Vec<Word> = a.pos.iter().skip(2).cloned().collect();
                if !rest.is_empty() {
                    invocation(cx, &Invocation { argv: rest, cwd: cwd.clone(), ..Default::default() });
                }
                return;
            }
            net(cx, "downloads Rust tools");
            cx.b.find(Flag::InstallsPackages, Risk::Caution, "installs, removes or switches Rust toolchains".to_string());
            if sub == "self" && matches!(second, "uninstall") {
                cx.b.find(Flag::Delete, Risk::Risky, "uninstalls Rust from this PC".to_string());
            }
        }
        ("cargo", "search" | "info") => net(cx, "asks the Rust package server for information"),
        ("cargo", "build" | "b" | "check" | "c" | "test" | "t" | "run" | "r" | "bench" | "clippy" | "doc" | "d" | "nextest" | "expand" | "tauri" | "rustc" | "rustdoc" | "miri" | "llvm-cov" | "tarpaulin" | "watch" | "make" | "xtask" | "deny" | "audit" | "outdated" | "udeps" | "machete" | "bloat" | "criterion" | "flamegraph" | "insta" | "leptos" | "dioxus" | "ndk" | "apk" | "lambda" | "shuttle" | "component" | "hack" | "semver-checks" | "msrv" | "bundle" | "zigbuild" | "pgrx" | "sqlx" | "sort" | "geiger" | "depgraph") => {
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("builds or runs this project's Rust code (cargo {sub})"));
            if let Some(d) = value_of(args, &["--target-dir"]) {
                write(cx, &cwd, &d, Write::Create);
            }
        }
        ("cargo", "fmt" | "fix") => {
            if !args.iter().any(|w| w.exact && w.text == "--check") {
                cx.b.find(Flag::WritesFile, Risk::Caution, "rewrites source files in place".to_string());
            }
            if sub == "fix" {
                cx.b.find(Flag::RunsCode, Risk::Caution, "builds this project's Rust code".to_string());
            }
        }
        ("cargo", "add" | "remove" | "rm" | "update" | "fetch" | "generate-lockfile" | "vendor" | "upgrade" | "new" | "init" | "generate") => {
            if !matches!(sub.as_str(), "new" | "init" | "remove" | "rm") {
                net(cx, "talks to the Rust package server");
            }
            cx.b.find(Flag::InstallsPackages, Risk::Caution, "changes the project's Rust packages or creates a project".to_string());
        }
        ("cargo", "install" | "uninstall" | "binstall") => {
            net(cx, "talks to the Rust package server");
            cx.b.find(Flag::InstallsPackages, Risk::Caution, "installs or removes a Rust program".to_string());
            cx.b.find(Flag::SystemChange, Risk::Risky, "installs or removes a program for the whole PC, not just this project".to_string());
            if a.any(&["--git"]) {
                cx.b.find(Flag::RemoteCode, Risk::Risky, "builds and installs code straight from a git address".to_string());
            }
        }
        ("cargo", "clean") => cx.b.find(Flag::Delete, Risk::Caution, "deletes the build output folder (it can be rebuilt)".to_string()),
        ("cargo", "publish" | "yank" | "owner" | "login" | "logout") => {
            net(cx, "talks to the Rust package server");
            if matches!(sub.as_str(), "publish" | "yank") {
                cx.b.find(Flag::Publishes, Risk::Caution, "publishes or withdraws a package on the public Rust registry".to_string());
            } else {
                cx.b.find(Flag::SensitivePath, Risk::Risky, "changes the registry account or who can publish".to_string());
            }
        }
        ("go", "build" | "test" | "run" | "generate" | "fmt" | "fix" | "tool" | "work" | "bug") => {
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("builds or runs this project's Go code (go {sub})"));
            if let Some(o) = value_of(args, &["-o"]) {
                write(cx, &cwd, &o, Write::Truncate);
            }
        }
        ("go", "get" | "install" | "mod") => {
            if sub == "mod" && matches!(second, "graph" | "why" | "verify" | "edit") && second != "edit" {
                return;
            }
            net(cx, "talks to Go package servers");
            cx.b.find(Flag::InstallsPackages, Risk::Caution, "changes the project's Go packages".to_string());
            if sub == "install" {
                cx.b.find(Flag::SystemChange, Risk::Risky, "installs a program for the whole PC, not just this project".to_string());
            }
        }
        ("go", "clean") => cx.b.find(Flag::Delete, Risk::Caution, "deletes Go build output or caches".to_string()),
        (_, other) => cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs “{name} {other}”, an extension Atoll does not know")),
    }
}

// ---------------------------------------------------------------- containers, databases, cloud

pub(crate) fn docker(cx: &mut Cx, inv: &Invocation, name: &str) {
    let args = &inv.argv[1..];
    let a = split(args, &["-f", "--file", "-p", "--project-name", "--context", "-H", "--host", "-c", "--config", "-l", "--log-level", "--name", "-e", "--env", "-v", "--volume", "-w", "--workdir", "-u", "--user", "--network", "--mount", "-t", "--tag", "--platform", "--entrypoint", "--env-file", "--target", "--build-arg", "-m", "--memory", "--cpus", "--restart", "--label", "--format", "--filter", "--since", "--until", "--tail", "-n", "--publish", "--add-host", "--cap-add", "--cap-drop", "--device", "--security-opt", "--pid", "--ipc", "--uts", "--userns", "--hostname", "-h", "--profile", "--project-directory", "--scale", "--timeout", "--output", "-o", "--cache-from", "--cache-to", "--secret", "--ssh", "--progress", "--pull", "--shm-size", "--ulimit", "--gpus", "--runtime", "--cidfile", "--log-driver", "--log-opt", "--dns", "--expose", "--ip", "--link", "--tmpfs", "--volumes-from", "--stop-signal", "--health-cmd", "--isolation", "--storage-opt", "--sysctl"]);
    let mut words: Vec<&str> = a.pos.iter().filter(|p| p.exact).map(|p| p.text.as_str()).collect();
    if name == "docker-compose" {
        words.insert(0, "compose");
    }
    let sub = words.first().copied().unwrap_or("");
    let second = words.get(1).copied().unwrap_or("");
    shape(cx, "docker", &if matches!(sub, "compose" | "image" | "container" | "volume" | "network" | "system" | "builder" | "buildx" | "context" | "stack" | "service" | "secret" | "config" | "plugin" | "node" | "swarm" | "manifest" | "trust") { format!("{sub} {second}") } else { sub.to_string() }, args);
    for w in args {
        if !w.exact {
            continue;
        }
        let t = w.text.to_lowercase();
        if t == "--privileged" || t.starts_with("--pid=host") || t.starts_with("--cap-add") || t.starts_with("--device") || t.starts_with("--security-opt") || t.starts_with("--userns=host") || t.starts_with("--ipc=host") || t.starts_with("--uts=host") || t.contains("docker.sock") || t.starts_with("--net=host") || t.starts_with("--network=host") {
            cx.b.find(Flag::Privilege, Risk::Risky, "gives a container control over this PC".to_string());
        }
    }
    for n in ["-v", "--volume", "--mount"] {
        let mut i = 0;
        while i < args.len() {
            let w = &args[i];
            let val = if w.exact && w.text == n { args.get(i + 1).map(|v| v.text.clone()) } else { w.text.strip_prefix(&format!("{n}=")).map(String::from) };
            if let Some(v) = val {
                let src = v.split(',').find_map(|p| p.strip_prefix("source=").or_else(|| p.strip_prefix("src="))).map(String::from).unwrap_or_else(|| {
                    // host:container[:mode], where the host part may itself be C:\...
                    let parts: Vec<&str> = v.split(':').collect();
                    if parts.len() >= 3 && parts[0].len() == 1 { format!("{}:{}", parts[0], parts[1]) } else { parts[0].to_string() }
                });
                let n = crate::paths::norm(&src);
                if crate::paths::is_abs(&n) {
                    let z = crate::paths::zone(&n, cx.ctx);
                    if matches!(z, crate::paths::Zone::System | crate::paths::Zone::Secret | crate::paths::Zone::Persistence) || n == "/" {
                        cx.b.find(Flag::Privilege, Risk::Risky, format!("lets a container read and write {}", short(&src)));
                    }
                }
            }
            i += 1;
        }
    }
    let group = matches!(sub, "compose" | "image" | "container" | "volume" | "network" | "system" | "builder" | "buildx" | "context" | "stack" | "service" | "secret" | "config" | "plugin" | "node" | "swarm");
    let verb = if group { second } else { sub };
    match verb {
        "ps" | "images" | "logs" | "inspect" | "version" | "info" | "stats" | "top" | "port" | "diff" | "history" | "events" | "ls" | "list" | "config" | "df" | "show" | "" | "--version" | "-v" | "help" | "wait" => {
            if verb == "config" && !group {
                // docker config create/rm handled below via second word
            }
        }
        "search" => network(cx, None, "searches"),
        "rm" | "rmi" | "prune" | "remove" => {
            cx.b.find(Flag::Delete, Risk::Risky, "deletes containers, images or volumes (their data goes too)".to_string());
        }
        "down" => {
            if a.short('v') || a.any(&["--volumes", "--rmi"]) {
                cx.b.find(Flag::Delete, Risk::Risky, "stops the services and deletes their data volumes or images".to_string());
            } else {
                cx.b.find(Flag::KillsProcess, Risk::Caution, "stops and removes the project's containers".to_string());
            }
        }
        "kill" | "stop" | "pause" | "restart" | "unpause" | "leave" => cx.b.find(Flag::KillsProcess, Risk::Caution, "stops or restarts containers".to_string()),
        "push" | "login" | "logout" => {
            network(cx, None, "talks to a container registry at");
            if verb == "push" {
                cx.b.find(Flag::Publishes, Risk::Caution, "uploads an image to a registry".to_string());
            } else {
                cx.b.find(Flag::SensitivePath, Risk::Caution, "changes saved registry logins".to_string());
            }
        }
        "pull" => network(cx, None, "downloads an image from"),
        _ => {
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("builds or runs containers (docker {verb})"));
            if matches!(verb, "build" | "run" | "up" | "create" | "bake") {
                cx.b.find(Flag::Network, Risk::Caution, "may download images".to_string());
                cx.b.remote_network = true;
            }
        }
    }
}

pub(crate) fn sql_client(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    shape(cx, name, "", args);
    cx.b.find(Flag::Database, Risk::Caution, format!("opens a database with {name}"));
    let host = value_of(args, &["-h", "--host", "-S", "--server", "--hostname"]).or_else(|| args.iter().find(|w| w.exact && w.text.contains("://")).cloned());
    match &host {
        Some(h) if h.exact && host_of(&h.text).is_some_and(|x| is_local_host(&x)) => {}
        Some(h) => network(cx, Some(h), "connects to a database at"),
        None => {}
    }
    match name {
        "dropdb" => {
            cx.b.find(Flag::Delete, Risk::Risky, "deletes a whole database".to_string());
            return;
        }
        "pg_restore" => {
            if args.iter().any(|w| w.exact && (w.text == "--clean" || w.text == "-c" || w.text == "--create" || w.text == "-C")) {
                cx.b.find(Flag::Delete, Risk::Risky, "drops existing tables before restoring".to_string());
            } else {
                cx.b.find(Flag::Database, Risk::Caution, "loads data into a database".to_string());
            }
        }
        "pg_dump" | "mysqldump" => {
            if let Some(f) = value_of(args, &["-f", "--file", "-r", "--result-file"]) {
                write(cx, &cwd, &f, Write::Truncate);
            }
        }
        "pg_ctl" => cx.b.find(Flag::SystemChange, Risk::Caution, "starts or stops a database server".to_string()),
        "redis-cli" => {
            let text = args.iter().map(|w| w.text.to_uppercase()).collect::<Vec<_>>().join(" ");
            if Regex::new(r"\b(FLUSHALL|FLUSHDB|SHUTDOWN|CONFIG\s+SET|DEBUG|MODULE\s+LOAD|SLAVEOF|REPLICAOF|SCRIPT\s+FLUSH|EVAL)\b").unwrap().is_match(&text) {
                cx.b.find(Flag::Delete, Risk::Risky, "wipes or reconfigures the Redis server".to_string());
            } else if Regex::new(r"\b(DEL|UNLINK|SET|HSET|LPUSH|RPUSH|SADD|ZADD|EXPIRE|RENAME|INCR|DECR|APPEND|MSET|XADD|PUBLISH)\b").unwrap().is_match(&text) {
                cx.b.find(Flag::Database, Risk::Caution, "changes data in Redis".to_string());
            }
            return;
        }
        _ => {}
    }
    let mut texts: Vec<Word> = vec![];
    for n in [["-c", "--command"], ["-e", "--execute"], ["-Q", "-q"], ["--eval", "--query"], ["-cmd", "--init-command"]] {
        let mut i = 0;
        while i < args.len() {
            if args[i].exact && n.contains(&args[i].text.as_str()) {
                if let Some(v) = args.get(i + 1) {
                    texts.push(v.clone());
                }
            } else if let Some(v) = n.iter().find_map(|k| if k.starts_with("--") { args[i].text.strip_prefix(&format!("{k}=")) } else { None }) {
                texts.push(Word { text: v.to_string(), ..args[i].clone() });
            }
            i += 1;
        }
    }
    if matches!(name, "sqlite3" | "duckdb") {
        let pos = split(args, &["-cmd", "-init", "-separator", "-newline", "-nullvalue", "-c"]).pos;
        texts.extend(pos.into_iter().skip(1));
        if let Some(i) = value_of(args, &["-init"]) {
            read(cx, &cwd, &i);
            cx.b.find(Flag::RunsCode, Risk::Caution, "runs SQL commands from a file".to_string());
        }
    }
    if let Some(t) = &inv.stdin_text {
        texts.push(Word::lit(t));
    }
    if let Some(f) = value_of(args, &["-f", "--file", "-i", "--input-file"]).filter(|_| !matches!(name, "pg_dump" | "mysqldump")) {
        read(cx, &cwd, &f);
        cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs SQL from the file {}", short(&f.text)));
    }
    for r in &inv.redirects {
        if r.op.starts_with('<') {
            cx.b.find(Flag::RunsCode, Risk::Caution, format!("runs SQL from the file {}", short(&r.target.text)));
        }
    }
    for t in &texts {
        if !t.exact {
            cx.b.find(Flag::DynamicCommand, Risk::Risky, "the SQL is only known when it runs".to_string());
        }
        sql(cx, &t.text);
    }
    if inv.stdin_piped && texts.is_empty() {
        cx.b.find(Flag::DynamicCommand, Risk::Risky, "runs SQL produced by another command".to_string());
    }
}

const CLOUD_DESTROY: &[&str] = &["delete", "destroy", "remove", "rm", "rb", "terminate", "purge", "drop", "uninstall", "kill", "revoke", "reset", "wipe", "erase", "deprovision", "teardown", "rollback", "del", "prune", "unpublish", "disable", "detach", "deregister", "release-address", "empty", "force-unlock", "taint", "delete-stack", "delete-bucket", "delete-object", "delete-objects", "delete-secret", "delete-function", "delete-table", "delete-db-instance", "terminate-instances", "stop-instances", "archive", "transfer", "leave", "nuke", "clean", "undeploy", "scale-down", "drain", "cordon", "evict"];
const CLOUD_CHANGE: &[&str] = &["deploy", "publish", "push", "release", "promote", "submit", "apply", "create", "update", "set", "put", "upload", "sync", "cp", "mv", "run", "exec", "merge", "close", "edit", "comment", "send", "invoke", "import", "add", "new", "up", "build", "start", "stop", "restart", "scale", "patch", "replace", "rename", "tag", "label", "annotate", "expose", "attach", "enable", "link", "unlink", "login", "logout", "init", "upgrade", "install", "migrate", "seed", "trigger", "dispatch", "rerun", "approve", "review", "reopen", "lock", "unlock", "pin", "fork", "clone", "checkout", "copy", "write", "store", "configure", "refresh", "rotate", "alias", "switch", "use", "open", "tunnel", "proxy", "port-forward", "ssh", "shell", "console", "connect", "secrets", "env", "domains", "dns", "certs", "billing", "token", "auth", "ready", "request", "pull"];
const CLOUD_READ: &[&str] = &["list", "ls", "get", "describe", "show", "view", "status", "logs", "log", "whoami", "version", "help", "diff", "plan", "validate", "check", "checks", "watch", "top", "explain", "inspect", "search", "info", "output", "fmt", "graph", "providers", "history", "events", "api-resources", "api-versions", "cluster-info", "doctor", "docs", "browse", "preview", "tail", "wait", "lint", "template", "dry-run", "config:get", "projects", "regions", "zones"];

pub(crate) fn cloud(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    let a = split(args, &["-R", "--repo", "-n", "--namespace", "--context", "--kubeconfig", "--profile", "--region", "--project", "-f", "--filename", "-o", "--output", "--jq", "-q", "--template", "-t", "-H", "--header", "--hostname", "-X", "--method", "-F", "--field", "-f", "--raw-field", "--input", "--token", "--scope", "-A", "--app", "-a", "--team", "--org", "-e", "--env", "--environment", "-var", "-var-file", "-target", "--limit", "-L", "--json", "--state", "-s", "--search", "--branch", "-b", "--title", "--body", "-m", "--message", "--platform", "-p", "--query", "--filter", "--format", "--selector", "-l", "--cwd", "--config", "-c", "--name", "--subscription", "-g", "--resource-group", "--zone", "--cluster", "--endpoint-url", "--bucket", "--key", "--stack-name", "--function-name", "--table-name", "--tail", "--since", "--container", "--kube-context", "--values", "--set", "--version", "--chart", "--timeout", "--scope", "--local-config", "--global-config"]);
    let words: Vec<String> = a.pos.iter().filter(|p| p.exact).take(4).map(|p| p.text.to_lowercase()).collect();
    let sub = words.first().cloned().unwrap_or_default();
    shape(cx, name, &words.iter().take(2).cloned().collect::<Vec<_>>().join(" "), args);
    if matches!(sub.as_str(), "" | "version" | "help" | "--version" | "completion") && !a.pos.iter().any(|p| !p.exact) {
        return;
    }
    cx.b.find(Flag::Network, Risk::Caution, format!("talks to an online service through {name}"));
    cx.b.remote_network = true;
    // Only the subcommand decides what happens; `gh api "repos/$r/..."` is still a read.
    if a.pos.first().is_some_and(|p| !p.exact) {
        cx.b.find(Flag::DynamicCommand, Risk::Risky, format!("what {name} does is only known when it runs"));
        return;
    }
    // gh api: the HTTP method decides.
    if matches!(name, "gh" | "glab") && sub == "api" {
        let method = value_of(args, &["-X", "--method"]).map(|m| m.text.to_uppercase()).unwrap_or_default();
        let sends = a.opts.iter().any(|o| matches!(o.as_str(), "-f" | "-F" | "--field" | "--raw-field" | "--input"));
        if method == "DELETE" {
            cx.b.find(Flag::Delete, Risk::Risky, "asks the service to delete something".to_string());
        } else if matches!(method.as_str(), "POST" | "PUT" | "PATCH") || sends {
            cx.b.find(Flag::SendsData, Risk::Caution, "changes something on the service".to_string());
            cx.b.find(Flag::Publishes, Risk::Caution, "changes something others can see".to_string());
        }
        if let Some(i) = value_of(args, &["--input"]) {
            read(cx, &cwd, &i);
        }
        return;
    }
    if matches!(name, "gh" | "glab") && (sub == "auth" || sub == "secret" || sub == "ssh-key" || sub == "gpg-key" || sub == "variable") {
        let second = words.get(1).map(String::as_str).unwrap_or("");
        if matches!(second, "status" | "list" | "ls") {
            return;
        }
        cx.b.find(Flag::SensitivePath, Risk::Risky, "shows or changes login tokens, keys or secrets".to_string());
        Builder::push_unique(&mut cx.b.secret_reads, "login token");
        return;
    }
    if matches!(name, "ngrok" | "cloudflared") && !matches!(sub.as_str(), "version" | "config" | "help") {
        cx.b.find(Flag::Network, Risk::Risky, "opens a tunnel that lets the internet reach this PC".to_string());
        return;
    }
    if matches!(name, "ansible" | "ansible-playbook" | "vagrant" | "packer") {
        cx.b.find(Flag::RunsCode, Risk::Caution, "runs automation against machines".to_string());
    }
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()) || w.split(['-', ':']).any(|part| list.contains(&part) && w.len() > part.len()));
    let delete_flag = args.iter().any(|w| w.exact && matches!(w.text.as_str(), "--delete" | "--force" | "--prod" | "--yes" | "-y" | "--auto-approve" | "--delete-branch" | "--purge" | "--all" | "--recursive"));
    if has(CLOUD_DESTROY) || (sub == "s3" && args.iter().any(|w| w.exact && w.text == "--delete")) {
        cx.b.find(Flag::Delete, Risk::Risky, "deletes or tears down something on the online service".to_string());
    } else if words.iter().any(|w| CLOUD_READ.contains(&w.as_str())) && !has(CLOUD_CHANGE) {
        // read-only
    } else if has(CLOUD_CHANGE) || words.len() <= 1 && matches!(name, "vercel" | "netlify" | "surge" | "gh-pages" | "eas" | "serverless" | "sls" | "cdk" | "pulumi") {
        cx.b.find(Flag::Publishes, Risk::Caution, "changes or deploys something on the online service".to_string());
        if matches!(sub.as_str(), "apply" | "up") && matches!(name, "terraform" | "tofu" | "pulumi" | "cdk") {
            cx.b.find(Flag::Publishes, Risk::Risky, "changes live infrastructure".to_string());
        }
        if matches!(sub.as_str(), "exec" | "ssh" | "shell" | "run" | "console") {
            cx.b.find(Flag::RunsCode, Risk::Caution, "runs a command on a remote machine".to_string());
        }
        for n in ["-f", "--filename", "--input", "-var-file", "--values"] {
            if let Some(f) = value_of(args, &[n]) {
                read(cx, &cwd, &f);
            }
        }
        if matches!(sub.as_str(), "cp" | "sync" | "mv" | "upload" | "put") {
            cx.b.find(Flag::SendsData, Risk::Caution, "copies files to or from the online service".to_string());
            for p in a.pos.iter().skip(1).filter(|p| p.exact && !p.text.contains("://") && !matches!(p.text.as_str(), "cp" | "sync" | "mv" | "upload" | "put")) {
                read(cx, &cwd, p);
            }
        }
        let _ = delete_flag;
    } else {
        cx.b.find(Flag::UnknownProgram, Risk::Caution, format!("runs “{name} {sub}”, which Atoll does not know"));
    }
}

pub(crate) fn system_pm(cx: &mut Cx, inv: &Invocation, name: &str) {
    let args = &inv.argv[1..];
    let a = split(args, &["--id", "--source", "-s", "--name", "--version", "-v", "--scope", "--override", "--location", "-l", "--log", "-o", "--architecture", "-a", "--locale", "-t", "--target-release", "-c", "--config-file", "--root", "-r", "/i", "/x", "/package"]);
    let sub = a.pos.first().filter(|p| p.exact).map(|p| p.text.to_lowercase()).unwrap_or_default();
    shape(cx, name, &sub, args);
    let listing = matches!(sub.as_str(), "list" | "search" | "show" | "info" | "policy" | "outdated" | "doctor" | "config" | "--prefix" | "--version" | "-v" | "help" | "leaves" | "deps" | "uses" | "home" | "desc" | "status" | "which" | "why" | "depends" | "rdepends" | "contents" | "environment" | "query" | "check" | "features" | "sources" | "source" | "export" | "bucket" | "cat" | "prefix" | "")
        || (matches!(name, "dpkg") && (a.short('l') || a.short('L') || a.short('s') || a.short('S') || a.any(&["--list", "--listfiles", "--status", "--search", "--get-selections", "--print-architecture"])))
        || (matches!(name, "rpm") && (a.short('q') || a.long("--query") || a.short('V')))
        || (matches!(name, "pacman") && (a.opts.iter().any(|o| o.starts_with("-Q") || o.starts_with("-Ss") || o.starts_with("-Si") || o.starts_with("-F"))))
        || (name == "apt-get" && a.short('s'))
        || (name == "gem" && matches!(sub.as_str(), "list" | "search" | "env" | "environment" | "which" | "contents" | "dependency" | "specification" | "query"))
        || (name == "nix" && matches!(sub.as_str(), "eval" | "flake" | "search" | "show-config" | "path-info" | "why-depends" | "log" | "doctor" | "repl"));
    if listing && !(matches!(sub.as_str(), "config" | "source" | "sources" | "bucket") && a.pos.len() > 1 && a.pos.get(1).is_some_and(|p| matches!(p.text.as_str(), "set" | "add" | "remove" | "rm" | "reset" | "delete" | "unset" | "import"))) {
        if matches!(sub.as_str(), "search" | "show" | "info" | "outdated" | "policy") {
            cx.b.find(Flag::Network, Risk::Caution, "asks a package server for information".to_string());
            cx.b.remote_network = true;
        }
        return;
    }
    cx.b.find(Flag::Network, Risk::Caution, "talks to a package server".to_string());
    cx.b.remote_network = true;
    let removing = matches!(sub.as_str(), "remove" | "uninstall" | "purge" | "autoremove" | "erase" | "rm" | "delete" | "autoclean" | "clean" | "cleanup" | "unlink" | "untap") || (name == "pacman" && a.opts.iter().any(|o| o.starts_with("-R"))) || (name == "msiexec" && args.iter().any(|w| w.text.eq_ignore_ascii_case("/x") || w.text.eq_ignore_ascii_case("/uninstall"))) || (name == "dpkg" && (a.short('r') || a.short('P') || a.any(&["--remove", "--purge"]))) || (name == "rpm" && a.short('e'));
    if removing {
        cx.b.find(Flag::SystemChange, Risk::Risky, "removes software from this PC".to_string());
        cx.b.find(Flag::Delete, Risk::Risky, "uninstalls programs or clears their files".to_string());
    } else {
        cx.b.find(Flag::InstallsPackages, Risk::Caution, "installs or updates software".to_string());
        cx.b.find(Flag::SystemChange, Risk::Risky, "installs or changes software for the whole PC".to_string());
    }
}

/// Tools that change the machine itself. Only clearly read-only uses pass as safe.
pub(crate) fn system(cx: &mut Cx, inv: &Invocation, name: &str) {
    let cwd = inv.cwd.clone();
    let args = &inv.argv[1..];
    // Git Bash users write Windows switches as `//list` so they are not taken for paths.
    let lower: Vec<String> = args.iter().map(|w| { let t = w.text.to_lowercase(); if t.starts_with("//") && t.len() > 2 && !t[2..].contains('/') { t[1..].to_string() } else { t } }).collect();
    let first = lower.first().map(String::as_str).unwrap_or("");
    let is = |list: &[&str]| lower.iter().any(|w| list.contains(&w.as_str()));
    shape(cx, name, first, args);
    let read_only = match name {
        "systemctl" => matches!(first, "status" | "is-active" | "is-enabled" | "is-failed" | "list-units" | "list-unit-files" | "show" | "cat" | "list-timers" | "list-sockets" | "list-dependencies" | "get-default" | "--version" | ""),
        "service" => lower.get(1).is_some_and(|s| s == "status") || first == "--status-all",
        "launchctl" => matches!(first, "list" | "print" | "blame" | "dumpstate" | "version" | "help" | ""),
        "sc" => matches!(first, "query" | "queryex" | "qc" | "qdescription" | "qfailure" | "getdisplayname" | "getkeyname" | "enumdepend" | "sdshow" | ""),
        "reg" => matches!(first, "query" | "compare" | "") || first == "/?",
        "regedit" => false,
        "net" | "net1" => args.len() <= 1 && matches!(first, "" | "view" | "time" | "statistics" | "user" | "localgroup" | "share" | "use" | "start" | "accounts" | "config" | "session" | "file" | "group" | "help"),
        "netsh" => is(&["show", "dump"]) && !is(&["set", "add", "delete", "reset", "import", "exec", "install"]),
        "schtasks" => is(&["/query"]) || args.is_empty(),
        "crontab" => is(&["-l"]) && !is(&["-r", "-e"]),
        "at" | "batch" => is(&["-l", "-c"]) || name == "at" && args.is_empty(),
        "wmic" => !is(&["call", "delete", "create", "set", "terminate", "uninstall", "install"]) && !lower.iter().any(|w| w.starts_with("/node") || w.starts_with("/format:") && w.contains("http") || w.contains(".xsl")),
        "setx" => false,
        "bcdedit" => args.is_empty() || is(&["/enum", "/v"]),
        "mount" => args.is_empty() || is(&["-l"]),
        "sysctl" => !is(&["-w", "--write", "-p", "--load", "--system"]) && !lower.iter().any(|w| w.contains('=')),
        "vssadmin" => first.starts_with("list"),
        "wevtutil" => matches!(first, "el" | "enum-logs" | "gl" | "get-log" | "qe" | "query-events" | "gli" | "get-loginfo" | "ep" | "enum-publishers" | "gp"),
        "certutil" => is(&["-hashfile", "-dump", "-store", "-verify", "-viewstore", "-?"]) && !is(&["-urlcache", "-decode", "-decodehex", "-encode", "-addstore", "-delstore", "-importpfx", "-exportpfx", "-f"]),
        "cipher" => !lower.iter().any(|w| w.starts_with("/w") || w.starts_with("/e") || w.starts_with("/d") || w.starts_with("/k") || w.starts_with("/r")),
        "fsutil" => is(&["query", "queryinfo", "info", "fsinfo", "list", "volumeinfo", "drives", "ntfsinfo", "statistics"]) && !is(&["set", "delete", "create", "setzerodata", "dirty", "createnew", "deletejournal"]),
        "iptables" | "ip6tables" | "nft" | "ufw" | "firewall-cmd" => is(&["-l", "--list", "-s", "--list-rules", "status", "list", "--state", "--list-all", "-n", "-v", "--get-active-zones"]) && !is(&["-a", "-d", "-i", "-f", "-x", "-p", "--flush", "enable", "disable", "allow", "deny", "reset", "add", "delete", "flush", "--add-port", "--add-service", "--reload", "--permanent", "-t"]) || name == "ufw" && first == "status",
        "defaults" => matches!(first, "read" | "read-type" | "domains" | "find" | "help" | "export"),
        "diskutil" => matches!(first, "list" | "info" | "activity" | "listfilesystems" | ""),
        "hdiutil" => matches!(first, "info" | "imageinfo" | "verify" | "help"),
        "csrutil" | "spctl" => is(&["status", "--status"]),
        "dscl" => is(&["-read", "-list", "read", "list", "-search", "search"]),
        "security" => matches!(first, "list-keychains" | "show-keychain-info" | "help" | "error" | "find-certificate" | "verify-cert" | "cms"),
        "cmdkey" => is(&["/list"]) && !is(&["/add", "/delete", "/generic"]),
        "gpg" => is(&["--list-keys", "-k", "--list-secret-keys", "--version", "--verify", "--fingerprint", "--list-sigs", "--list-packets", "-h", "--help"]) && !is(&["--export-secret-keys", "--delete-key", "--delete-secret-keys", "--import", "--gen-key", "--export-secret-subkeys", "-d", "--decrypt", "-c", "-e", "-s", "--sign", "--encrypt", "--recv-keys", "--send-keys", "--edit-key"]),
        "ssh-keygen" => is(&["-l", "-f", "-y", "-e", "-b", "-h"]) && is(&["-l"]) && !is(&["-r", "-p", "-c", "-t", "-a", "-m"]) || is(&["-h"]) && is(&["-f"]) && !is(&["-r", "-p", "-c", "-t"]) && is(&["-l", "-F"]),
        "ssh-add" => is(&["-l", "-L"]) && !is(&["-d", "-D", "-x", "-X"]),
        "keytool" => is(&["-list", "-printcert", "-help", "-showinfo"]),
        "powercfg" => is(&["/list", "/l", "/query", "/q", "/getactivescheme", "/a", "/availablesleepstates", "/batteryreport", "/energy", "/requests", "/lastwake", "/devicequery", "/waketimers"]),
        "tzutil" => is(&["/g", "/l"]),
        "w32tm" => is(&["/query", "/tz", "/stripchart", "/monitor"]),
        "pnputil" => is(&["/enum-devices", "/enum-drivers", "/enum-interfaces", "/?"]),
        "mountvol" | "subst" => args.is_empty(),
        "compact" | "manage-bde" => is(&["/q", "-status"]) || args.is_empty(),
        "dism" => is(&["/get-features", "/get-packages", "/get-capabilities", "/get-imageinfo", "/get-wiminfo", "/checkhealth", "/scanhealth", "/get-currentedition", "/get-drivers", "/?"]) && !is(&["/enable-feature", "/disable-feature", "/add-package", "/remove-package", "/restorehealth", "/apply-image", "/cleanup-image", "/add-driver", "/remove-driver", "/add-capability", "/remove-capability"]) || is(&["/checkhealth", "/scanhealth"]),
        "chkdsk" => !lower.iter().any(|w| matches!(w.as_str(), "/f" | "/r" | "/x" | "/b" | "/scan" | "/spotfix")),
        "sfc" => is(&["/verifyonly", "/verifyfile"]),
        "defrag" => is(&["/a"]),
        "gpupdate" | "secedit" => is(&["/analyze", "/export", "/validate"]) && name == "secedit",
        "update-alternatives" => is(&["--list", "--display", "--query", "--get-selections", "--config"]) && !is(&["--config", "--set", "--install", "--remove", "--auto", "--remove-all"]),
        "ldconfig" => is(&["-p", "--print-cache", "-v", "-n", "-N"]) && is(&["-p", "--print-cache"]),
        "tccutil" | "nvram" => name == "nvram" && is(&["-p", "-x"]) && args.len() <= 2,
        "modprobe" => is(&["-n", "--dry-run", "--show-depends", "-c", "--showconfig", "-l"]),
        "swapon" => is(&["-s", "--show", "--summary"]),
        "losetup" => args.is_empty() || is(&["-a", "-l", "--list", "-j"]),
        "dmsetup" | "cryptsetup" => matches!(first, "ls" | "info" | "status" | "table" | "deps" | "targets" | "version" | "luksdump" | "isluks" | "benchmark"),
        "chattr" | "setcap" | "setenforce" | "visudo" | "passwd" | "chsh" | "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel" | "su" | "newgrp" | "sudoedit" | "insmod" | "rmmod" | "swapoff" | "umount" | "shutdown" | "reboot" | "halt" | "poweroff" | "init" | "logoff" | "stop-computer" | "restart-computer" | "mkfs" | "fdisk" | "parted" | "wipefs" | "mkswap" | "sfdisk" | "gdisk" | "diskpart" | "format" | "rundll32" | "mshta" | "regsvr32" | "wscript" | "cscript" | "bitsadmin" | "installutil" | "regasm" | "odbcconf" | "forfiles" | "pcalua" | "scriptrunner" | "vaultcmd" | "devcon" | "bootrec" => {
            (matches!(name, "fdisk" | "parted" | "sfdisk" | "gdisk") && is(&["-l", "--list", "print"]) && args.len() <= 3 && !is(&["rm", "mkpart", "mklabel", "resizepart", "mkfs", "set"])) || (name == "visudo" && is(&["-c", "--check"])) || (name == "passwd" && is(&["-s", "--status"]) && false)
        }
        _ => false,
    };
    if read_only {
        if matches!(name, "cmdkey" | "security" | "gpg" | "ssh-add" | "keytool" | "vaultcmd") {
            cx.b.find(Flag::SensitivePath, Risk::Caution, "lists stored keys or logins".to_string());
        }
        return;
    }
    let (flag, note): (Flag, String) = match name {
        "shutdown" | "reboot" | "halt" | "poweroff" | "init" | "logoff" | "stop-computer" | "restart-computer" => (Flag::SystemChange, "shuts down, restarts or logs off this PC".into()),
        "mkfs" | "fdisk" | "parted" | "wipefs" | "mkswap" | "sfdisk" | "gdisk" | "diskpart" | "format" | "diskutil" | "hdiutil" | "bootrec" | "manage-bde" | "cryptsetup" | "dmsetup" | "losetup" | "mount" | "umount" | "mountvol" | "subst" | "chkdsk" | "defrag" | "compact" | "cipher" | "fsutil" | "swapoff" | "swapon" | "vssadmin" => (Flag::SystemChange, "changes disks, partitions or mounted volumes (data can be lost)".into()),
        "su" | "newgrp" | "sudoedit" | "visudo" | "passwd" | "chsh" | "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel" | "net" | "net1" | "dscl" | "setcap" | "setenforce" | "chattr" | "csrutil" | "spctl" | "tccutil" | "secedit" | "gpupdate" => (Flag::Privilege, "changes users, passwords, rights or security settings".into()),
        "crontab" | "at" | "batch" | "schtasks" | "launchctl" | "systemctl" | "service" | "sc" => (Flag::HiddenExec, "starts, stops or schedules programs that run by themselves later".into()),
        "reg" | "regedit" | "setx" | "defaults" | "bcdedit" | "sysctl" | "nvram" | "powercfg" | "tzutil" | "w32tm" | "dism" | "sfc" | "pnputil" | "devcon" | "update-alternatives" | "ldconfig" | "modprobe" | "insmod" | "rmmod" => (Flag::SystemChange, "changes system-wide settings".into()),
        "iptables" | "ip6tables" | "nft" | "ufw" | "firewall-cmd" | "netsh" => (Flag::SystemChange, "changes the firewall or network settings".into()),
        "wevtutil" => (Flag::Delete, "clears or changes Windows event logs".into()),
        "rundll32" | "mshta" | "regsvr32" | "wscript" | "cscript" | "installutil" | "regasm" | "odbcconf" | "forfiles" | "pcalua" | "scriptrunner" | "wmic" => (Flag::HiddenExec, format!("uses {name} to run other code in a roundabout way")),
        "bitsadmin" | "certutil" => (Flag::RemoteCode, format!("uses {name}, a Windows tool often misused to download or decode files")),
        "cmdkey" | "vaultcmd" | "security" | "gpg" | "ssh-keygen" | "ssh-add" | "keytool" => (Flag::SensitivePath, "creates, changes, exports or deletes keys or stored logins".into()),
        _ => (Flag::SystemChange, format!("changes this PC with {name}")),
    };
    // Making a new key inside the project or temp is routine developer work.
    if matches!(name, "ssh-keygen" | "gpg" | "keytool") {
        if let Some(f) = value_of(args, &["-f", "-o", "--output", "-keystore"]) {
            if name != "gpg" || is(&["--armor", "-a", "--export", "--output", "-o"]) && !is(&["--export-secret-keys", "--delete-key", "--delete-secret-keys"]) {
                let before = cx.b.findings.len();
                write(cx, &cwd, &f, Write::Create);
                let worst = cx.b.findings[before..].iter().map(|x| x.risk).max().unwrap_or(Risk::Safe);
                if worst <= Risk::Caution && is(&["-f", "-keystore", "-o", "--output"]) && !is(&["-r", "-p", "--delete-key", "-delete", "-importkeystore"]) {
                    return;
                }
            }
        }
    }
    if matches!(name, "certutil" | "bitsadmin") {
        if let Some(u) = args.iter().find(|w| looks_like_url(w)) {
            network(cx, Some(u), "downloads from");
        }
        if is(&["-decode", "-decodehex"]) {
            cx.b.find(Flag::Obfuscated, Risk::Risky, "decodes hidden text into a file".to_string());
        }
    }
    if name == "wmic" && lower.iter().any(|w| w.starts_with("/node")) {
        cx.b.find(Flag::Network, Risk::Risky, "runs a management command on another machine".to_string());
    }
    cx.b.find(flag, Risk::Risky, note);
}

// ---------------------------------------------------------------- where inline code writes

// `p = 'src/x.ts'`, `const f = "a.json";`
re!(VAR_STR, r#"(?m)(?:^|[;{(]|\b(?:const|let|var)\s)\s*([A-Za-z_]\w*)\s*=\s*[rRbBuU]?(['"`])([^'"`\n$]*)['"`]\s*(?:;|,|#|//|$)"#);
// `p = pathlib.Path("agent/llm.py")`
re!(VAR_PATH, r#"([A-Za-z_]\w*)\s*=\s*(?:pathlib\.)?Path\(\s*[rRbB]?['"]([^'"]+)['"]\s*\)"#);
// `for p in ['a.html', 'b.html']:` and `for (const p of ['a', 'b'])`
re!(FOR_LIST, r#"for\s*\(?\s*(?:const|let|var)?\s*([A-Za-z_]\w*)\s+(?:in|of)\s+[\[(]((?:\s*[rRbBuU]?['"`][^'"`\n]*['"`]\s*,?)+)\s*[\])]"#);
// Every way a name gets a value, to spot names that also get values Atoll cannot read.
re!(VAR_ANY, r#"(?m)(?:^|[;{(,]|\b(?:const|let|var)\s)\s*([A-Za-z_]\w*)\s*\+?=[^=]|for\s*\(?\s*(?:const|let|var)?\s*([A-Za-z_]\w*)\s+(?:in|of)\b|\bas\s+([A-Za-z_]\w*)"#);
// Function parameters: their values come from the caller.
re!(PARAMS, r#"\bdef\s+\w+\s*\(([^)]*)\)|\bfunction\s*\w*\s*\(([^)]*)\)|\(([^()]*)\)\s*=>|\b([A-Za-z_]\w*)\s*=>|\blambda\s+([^:]*):"#);
re!(QUOTED, r#"[rRbBuU]?['"`]([^'"`\n]*)['"`]"#);
re!(PATH_CALL, r#"^(?:pathlib\.)?Path\(\s*[rRbB]?['"]([^'"]+)['"]\s*\)$"#);
re!(LITERAL, r#"^[rRbBuU]?(['"`])([^'"`\n${}+]*)['"`]$"#);
// Calls that write a file, with the argument that names it.
re!(JS_WRITE_ARG, r#"(?:writeFileSync|writeFile|appendFileSync|appendFile|createWriteStream|mkdirSync|outputFile\w*|writeJson\w*|truncateSync|chmodSync)\s*\(\s*([^,()]+(?:\([^()]*\))?)"#);
re!(JS_COPY_ARG, r#"(?:copyFileSync|copyFile|renameSync|\brename|cpSync|symlinkSync|linkSync)\s*\(\s*[^,]+,\s*([^,()]+(?:\([^()]*\))?)"#);
re!(PY_WRITE_ARG, r#"\bopen\(\s*([^,()]+?(?:\([^()]*\))?)\s*,\s*(?:mode\s*=\s*)?[rRbBuU]?['"][rbt+]*[wax][bt+]*['"]|([A-Za-z_][\w.]*(?:\([^()]*\))?)\.(?:write_text|write_bytes)\(|shutil\.(?:copy\w*|move)\(\s*[^,]+,\s*([^,()]+(?:\([^()]*\))?)|os\.(?:rename|replace)\(\s*[^,]+,\s*([^,()]+(?:\([^()]*\))?)|\.to_(?:csv|json|parquet|excel)\(\s*([^,()]+)|savefig\(\s*([^,()]+)"#);
// The same calls, matched loosely, to notice ones whose argument the patterns above miss.
re!(PY_WRITE_ANY, r#"\bopen\([^\n]*?['"][rbt+]*[wax][bt+]*['"]|\.write_(?:text|bytes)\(|shutil\.(?:copy\w*|move)\(|os\.(?:rename|replace)\(|\.to_(?:csv|json|parquet|excel)\(|savefig\("#);
re!(JS_WRITE_ANY, r#"(?:writeFileSync|writeFile|appendFileSync|appendFile|createWriteStream|mkdirSync|outputFile\w*|writeJson\w*|truncateSync|chmodSync|copyFileSync|copyFile|renameSync|\brename|cpSync|symlinkSync|linkSync)\s*\("#);
// Places that make an unknown write dangerous when the code mentions them.
re!(SENSITIVE_PLACE, r#"(?i)\.bashrc|\.zshrc|\.bash_profile|\.profile\b|authorized_keys|[/\\]\.ssh\b|\.claude[/\\]|startup|crontab|\.git[/\\]hooks|expanduser|userprofile|homedir\(|path\.home|os\.homedir|appdata|/etc/|c:[/\\]windows|system32|\.gitconfig|\.npmrc|\.env\b"#);

/// Names given plain text values in the code. A name that also gets a value Atoll cannot
/// read (another assignment, a loop over something else, a function parameter) is left out.
fn code_vars(code: &str) -> std::collections::HashMap<String, Vec<String>> {
    use std::collections::HashMap;
    let mut vars: HashMap<String, Vec<String>> = HashMap::new();
    let mut literal_count: HashMap<String, usize> = HashMap::new();
    let add = |vars: &mut HashMap<String, Vec<String>>, name: &str, value: String| {
        let list = vars.entry(name.to_string()).or_default();
        if !list.contains(&value) {
            list.push(value);
        }
    };
    for c in VAR_STR.captures_iter(code) {
        add(&mut vars, &c[1], c[3].to_string());
        *literal_count.entry(c[1].to_string()).or_default() += 1;
    }
    for c in VAR_PATH.captures_iter(code) {
        add(&mut vars, &c[1], c[2].to_string());
        *literal_count.entry(c[1].to_string()).or_default() += 1;
    }
    for c in FOR_LIST.captures_iter(code) {
        for q in QUOTED.captures_iter(&c[2]) {
            add(&mut vars, &c[1], q[1].to_string());
        }
        *literal_count.entry(c[1].to_string()).or_default() += 1;
    }
    let mut all_count: HashMap<String, usize> = HashMap::new();
    for c in VAR_ANY.captures_iter(code) {
        if let Some(n) = c.get(1).or(c.get(2)).or(c.get(3)) {
            *all_count.entry(n.as_str().to_string()).or_default() += 1;
        }
    }
    let mut unknown: Vec<String> = all_count
        .iter()
        .filter(|(n, total)| **total > literal_count.get(*n).copied().unwrap_or(0))
        .map(|(n, _)| n.clone())
        .collect();
    for c in PARAMS.captures_iter(code) {
        for g in c.iter().skip(1).flatten() {
            for p in g.as_str().split(',') {
                let p = p.trim().trim_start_matches('*').split(['=', ':']).next().unwrap_or("").trim();
                if !p.is_empty() {
                    unknown.push(p.to_string());
                }
            }
        }
    }
    for n in unknown {
        vars.remove(&n);
    }
    vars
}

/// The paths a write call's argument can name, or None when Atoll cannot tell.
fn resolve_arg(arg: &str, vars: &std::collections::HashMap<String, Vec<String>>) -> Option<Vec<String>> {
    let a = arg.trim();
    if let Some(c) = LITERAL.captures(a) {
        return Some(vec![c[2].to_string()]);
    }
    if let Some(c) = PATH_CALL.captures(a) {
        return Some(vec![c[1].to_string()]);
    }
    if a.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return vars.get(a).cloned();
    }
    None
}

/// Files that inline code writes. Paths it names plainly are checked like any other write.
/// A write Atoll cannot pin down is a caution, unless the code mentions a sensitive place.
/// Inline code is never eligible for auto-allow either way.
fn code_writes(cx: &mut Cx, cwd: &Cwd, lang: Lang, what: &str, code: &str) {
    let (args, loose): (Vec<String>, usize) = match lang {
        Lang::Js => {
            let mut v: Vec<String> = JS_WRITE_ARG.captures_iter(code).map(|c| c[1].to_string()).collect();
            v.extend(JS_COPY_ARG.captures_iter(code).map(|c| c[1].to_string()));
            (v, JS_WRITE_ANY.find_iter(code).count())
        }
        Lang::Python => {
            let v: Vec<String> = PY_WRITE_ARG.captures_iter(code).filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str().to_string())).collect();
            (v, PY_WRITE_ANY.find_iter(code).count())
        }
        Lang::Other => (vec![], if ANY_WRITE.is_match(code) { 1 } else { 0 }),
    };
    let vars = code_vars(code);
    let mut unresolved = loose > args.len();
    for arg in &args {
        match resolve_arg(arg, &vars) {
            Some(paths) => {
                for p in paths {
                    write(cx, cwd, &Word::lit(&p), Write::Truncate);
                }
            }
            None => unresolved = true,
        }
    }
    if unresolved {
        if SENSITIVE_PLACE.is_match(code) {
            cx.b.find(Flag::UnresolvedPath, Risk::Risky, format!("the {what} code writes a file Atoll cannot pin down, and mentions a sensitive place"));
        } else {
            cx.b.find(Flag::UnresolvedPath, Risk::Caution, format!("the {what} code writes files Atoll cannot pin down"));
        }
    }
}
