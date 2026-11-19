//! Path handling: one comparable form for Windows, mixed and Git Bash paths,
//! and the "zone" a path falls in (project, temp, secret, system, ...).

use crate::model::Context;

/// Lower-case, forward slashes, `c:/` drive form, `.` and `..` folded.
pub fn norm(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/").to_lowercase();
    // Git Bash / Cygwin / WSL drive prefixes.
    for prefix in ["/cygdrive/", "/mnt/"] {
        if let Some(rest) = p.strip_prefix(prefix) {
            let b = rest.as_bytes();
            if !b.is_empty() && b[0].is_ascii_alphabetic() && (b.len() == 1 || b[1] == b'/') {
                p = format!("/{rest}");
            }
        }
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
        p = format!("{}:{}", b[1] as char, if b.len() == 2 { "/" } else { &p[2..] });
    }
    let unc = p.starts_with("//");
    let (root, rest) = if p.len() >= 2 && p.as_bytes()[1] == b':' {
        (format!("{}/", &p[..2]), p[2..].to_string())
    } else if p.starts_with('/') {
        (if unc { "//".to_string() } else { "/".to_string() }, p.clone())
    } else {
        (String::new(), p.clone())
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if root.is_empty() {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    if root.is_empty() {
        if joined.is_empty() { ".".into() } else { joined }
    } else {
        format!("{root}{joined}")
    }
}

pub fn is_abs(normed: &str) -> bool {
    normed.starts_with('/') || (normed.len() >= 3 && normed.as_bytes()[1] == b':' && normed.as_bytes()[2] == b'/')
}

/// True when `path` is `dir` or inside it (both already normalised).
pub fn within(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return false;
    }
    let dir = dir.trim_end_matches('/');
    path == dir || (path.starts_with(dir) && path.as_bytes().get(dir.len()) == Some(&b'/'))
}

/// Turn a shell word into absolute normalised candidates (one per possible
/// current folder). Empty = cannot tell.
pub fn resolve(text: &str, cwds: &Option<Vec<String>>, ctx: &Context) -> Vec<String> {
    let t = text.trim();
    if t.is_empty() {
        return vec![];
    }
    let expanded = if t == "~" {
        ctx.home.clone()
    } else if let Some(rest) = t.strip_prefix("~/").or_else(|| t.strip_prefix("~\\")) {
        format!("{}/{}", ctx.home, rest)
    } else {
        t.to_string()
    };
    let n = norm(&expanded);
    if is_abs(&n) {
        return vec![n];
    }
    match cwds {
        Some(list) => list.iter().map(|c| norm(&format!("{c}/{expanded}"))).collect(),
        None => vec![],
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    /// /dev/null and friends.
    Sink,
    Temp,
    Project,
    ProjectRoot,
    /// Inside the project's .git folder.
    GitInternals,
    /// Keys, tokens, .env files.
    Secret,
    /// Files that run later by themselves: shell profiles, git hooks, Claude settings, startup folders.
    Persistence,
    /// System folders, a drive root, the home folder itself, or a parent of the project.
    System,
    Elsewhere,
}

const SINKS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty", "nul", "/dev/stdin", "$null"];

pub fn is_sink(text: &str) -> bool {
    let n = text.trim().to_lowercase();
    SINKS.contains(&n.as_str()) || n.starts_with("/dev/fd/")
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Files that usually hold secrets.
pub fn secret_like(n: &str) -> bool {
    let b = base(n);
    let dirs = ["/.ssh", "/.aws", "/.gnupg", "/.kube", "/.azure", "/.config/gcloud", "/.docker"];
    if dirs.iter().any(|d| n.ends_with(d) || n.contains(&format!("{d}/"))) {
        return true;
    }
    if b.starts_with(".env") {
        return !(b.ends_with(".example") || b.ends_with(".sample") || b.ends_with(".template") || b.ends_with(".d.ts"));
    }
    matches!(b, ".npmrc" | ".netrc" | ".credentials.json" | ".pypirc" | ".git-credentials" | "credentials" | "credentials.json" | "shadow" | "sudoers" | "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa" | ".pgpass" | "secrets.json" | "secrets.yaml" | "secrets.yml" | ".htpasswd" | "login data" | "cookies" | "ntds.dit" | "sam")
        || [".pem", ".key", ".pfx", ".p12", ".keystore", ".jks", ".kdbx"].iter().any(|e| b.ends_with(e))
        || n.contains("/microsoft/credentials/")
        || n.contains("/microsoft/protect/")
}

/// Files that are run or trusted later without being asked again.
pub fn persistence_like(n: &str) -> bool {
    let b = base(n);
    matches!(b, ".bashrc" | ".bash_profile" | ".bash_login" | ".profile" | ".zshrc" | ".zprofile" | ".zshenv" | ".gitconfig" | "authorized_keys" | "known_hosts" | "hosts" | "crontab" | ".claude.json" | "microsoft.powershell_profile.ps1" | "profile.ps1" | ".inputrc" | ".curlrc" | ".wgetrc")
        && (b != "hosts" || n.ends_with("/etc/hosts"))
        || n.contains("/.git/hooks/")
        || n.ends_with("/.git/config")
        || n.contains("/.config/git/")
        // Claude Code's own settings, hooks, skills, agents and commands change what Claude does next.
        || (n.contains("/.claude/") && (b.starts_with("settings") && b.ends_with(".json") || ["/.claude/hooks/", "/.claude/skills/", "/.claude/agents/", "/.claude/commands/", "/.claude/plugins/"].iter().any(|d| n.contains(d))))
        || n.contains("/start menu/programs/startup")
        || n.contains("/etc/cron")
        || n.contains("/etc/profile")
        || n.contains("/etc/systemd/")
        || n.contains("/launchagents/")
        || n.contains("/launchdaemons/")
}

pub fn zone(n: &str, ctx: &Context) -> Zone {
    if is_sink(n) {
        return Zone::Sink;
    }
    if persistence_like(n) {
        return Zone::Persistence;
    }
    if secret_like(n) {
        return Zone::Secret;
    }
    let project = norm(&ctx.project);
    if !ctx.project.is_empty() && is_abs(&project) {
        if n == project {
            return Zone::ProjectRoot;
        }
        if within(n, &project) {
            let rel = &n[project.trim_end_matches('/').len()..];
            if rel == "/.git" || rel.starts_with("/.git/") {
                return Zone::GitInternals;
            }
            return Zone::Project;
        }
        // A parent of the project: deleting or replacing it takes the project with it.
        if within(&project, n) {
            return Zone::System;
        }
    }
    let temps = ctx.temp.iter().map(|t| norm(t)).chain(["/tmp".to_string(), "/var/tmp".to_string()]);
    for t in temps {
        if within(n, &t) && n != t {
            return Zone::Temp;
        }
    }
    let home = norm(&ctx.home);
    if !ctx.home.is_empty() && n == home {
        return Zone::System;
    }
    let system = [
        "/etc", "/usr", "/bin", "/sbin", "/boot", "/lib", "/lib64", "/sys", "/proc", "/dev", "/var", "/opt", "/root",
        "c:/windows", "c:/program files", "c:/program files (x86)", "c:/programdata",
    ];
    if n == "/" || (n.len() == 3 && n.ends_with(":/")) || system.iter().any(|s| within(n, s)) {
        return Zone::System;
    }
    Zone::Elsewhere
}

/// The most worrying zone among candidates (unknown folder = several candidates).
pub fn worst(zones: &[Zone]) -> Option<Zone> {
    let rank = |z: &Zone| match z {
        Zone::Sink => 0,
        Zone::Temp => 1,
        Zone::Project => 2,
        Zone::ProjectRoot => 3,
        Zone::Elsewhere => 4,
        Zone::GitInternals => 5,
        Zone::Secret => 6,
        Zone::System => 7,
        Zone::Persistence => 8,
    };
    zones.iter().max_by_key(|z| rank(z)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            project: "C:\\Users\\me\\proj".into(),
            cwd: "C:\\Users\\me\\proj".into(),
            home: "C:\\Users\\me".into(),
            temp: vec!["C:\\Users\\me\\AppData\\Local\\Temp".into()],
            env: Default::default(),
        }
    }

    #[test]
    fn norm_forms_agree() {
        assert_eq!(norm("C:\\Users\\Me\\proj\\"), "c:/users/me/proj");
        assert_eq!(norm("/c/Users/me/proj/./src/../x"), "c:/users/me/proj/x");
        assert_eq!(norm("/mnt/c/Users"), "c:/users");
        assert_eq!(norm("/c"), "c:/");
        assert_eq!(norm("/tmp/../etc/passwd"), "/etc/passwd");
        assert_eq!(norm("a/../../b"), "../b");
        assert_eq!(norm("/../.."), "/");
    }

    #[test]
    fn zones() {
        let c = ctx();
        let z = |p: &str| zone(&norm(p), &c);
        assert_eq!(z("/c/Users/me/proj/src/a.rs"), Zone::Project);
        assert_eq!(z("C:/Users/me/proj"), Zone::ProjectRoot);
        assert_eq!(z("C:/Users/me/proj/.git/index"), Zone::GitInternals);
        assert_eq!(z("C:/Users/me/proj/.git/hooks/pre-commit"), Zone::Persistence);
        assert_eq!(z("C:/Users/me/proj/.env.local"), Zone::Secret);
        assert_eq!(z("C:/Users/me/proj/.env.example"), Zone::Project);
        assert_eq!(z("C:/Users/me"), Zone::System);
        assert_eq!(z("C:/Users"), Zone::System);
        assert_eq!(z("/"), Zone::System);
        assert_eq!(z("C:/"), Zone::System);
        assert_eq!(z("/tmp/x.log"), Zone::Temp);
        assert_eq!(z("C:/Users/me/AppData/Local/Temp/claude/x"), Zone::Temp);
        assert_eq!(z("C:/Users/me/.ssh/id_rsa"), Zone::Secret);
        assert_eq!(z("C:/Users/me/.bashrc"), Zone::Persistence);
        assert_eq!(z("C:/Users/me/.claude/settings.json"), Zone::Persistence);
        assert_eq!(z("C:/Users/me/other/file"), Zone::Elsewhere);
        assert_eq!(z("/dev/null"), Zone::Sink);
        assert_eq!(z("C:/Users/me/projector/x"), Zone::Elsewhere);
    }

    #[test]
    fn resolve_uses_every_candidate_folder() {
        let c = ctx();
        let cwds = Some(vec!["c:/users/me/proj".to_string(), "/tmp/w".to_string()]);
        assert_eq!(resolve("a/b", &cwds, &c), vec!["c:/users/me/proj/a/b", "/tmp/w/a/b"]);
        assert_eq!(resolve("~/x", &cwds, &c), vec!["c:/users/me/x"]);
        assert!(resolve("a", &None, &c).is_empty());
        assert_eq!(resolve("..", &cwds, &c)[0], "c:/users/me");
    }
}

/// What a file that runs or is trusted later is, in plain words: (what it is, what that means).
pub fn runs_later(n: &str) -> (&'static str, &'static str) {
    let b = base(n);
    if matches!(b, ".bashrc" | ".bash_profile" | ".bash_login" | ".profile" | ".zshrc" | ".zprofile" | ".zshenv" | ".inputrc") {
        ("your shell startup file", "which runs every time a terminal opens")
    } else if b.ends_with("profile.ps1") {
        ("your PowerShell profile", "which runs every time PowerShell starts")
    } else if n.contains("/.git/hooks/") {
        ("a git hook", "which runs by itself on git commands in this repository")
    } else if b == ".gitconfig" || n.ends_with("/.git/config") || n.contains("/.config/git/") {
        ("your git settings", "which apply to every git command")
    } else if b == "authorized_keys" {
        ("the list of keys allowed to log in to this PC", "")
    } else if b == "known_hosts" {
        ("the list of servers SSH trusts", "")
    } else if b == "hosts" {
        ("the hosts file", "which decides where web addresses lead")
    } else if b == "crontab" || n.contains("/etc/cron") {
        ("a list of scheduled tasks", "which run by themselves")
    } else if n.contains("/.claude/") || b == ".claude.json" {
        ("Claude Code's own settings or skills", "which steer every later session")
    } else if n.contains("/start menu/programs/startup") {
        ("the Windows startup folder", "which runs at every sign-in")
    } else if n.contains("/etc/systemd/") || n.contains("/launchagents/") || n.contains("/launchdaemons/") || n.contains("/etc/profile") {
        ("a service or start-up script", "which starts by itself")
    } else {
        ("a file", "which runs or is trusted automatically later")
    }
}

/// What a secret-like file holds, in plain words.
pub fn secret_kind(n: &str) -> String {
    let b = base(n);
    if matches!(b, "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa") {
        "your SSH private key".into()
    } else if n.contains("/.ssh") {
        "your SSH keys or settings".into()
    } else if b.starts_with(".env") {
        format!("{b}, a file of passwords and keys")
    } else if n.contains("/.aws") {
        "your AWS credentials".into()
    } else if n.contains("/.kube") {
        "your Kubernetes credentials".into()
    } else if b == ".npmrc" || b == ".pypirc" {
        format!("your package-registry login ({b})")
    } else if matches!(b, ".netrc" | ".git-credentials" | ".pgpass" | ".htpasswd") {
        format!("saved passwords ({b})")
    } else if b == ".credentials.json" || b == "credentials.json" || b == "credentials" {
        format!("saved login tokens ({b})")
    } else if [".pem", ".key", ".pfx", ".p12", ".keystore", ".jks"].iter().any(|e| b.ends_with(e)) {
        format!("a private key file ({b})")
    } else if b == "login data" || b == "cookies" {
        "your browser's saved logins".into()
    } else {
        format!("{b}, which holds secrets")
    }
}
