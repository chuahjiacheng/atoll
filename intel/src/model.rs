//! The structured result of analysing one tool request.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// How much care a request needs. `Unknown` = the analysis could not finish
/// (parse error, cut-off command, timeout); it is never eligible for auto-allow.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Risk {
    /// Only reads or prints.
    Safe,
    /// Changes files inside the project, uses the network, or runs code. Normal work.
    Caution,
    /// Could not be analysed.
    Unknown,
    /// Can destroy work, leave the project, escalate, run hidden or downloaded code.
    Risky,
}

/// One thing the analysis noticed. Several can apply to one command.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Flag {
    Delete,
    Overwrite,
    WritesFile,
    Network,
    SendsData,
    PipeToShell,
    RemoteCode,
    Privilege,
    Obfuscated,
    DynamicCommand,
    HiddenExec,
    OutsideProject,
    SensitivePath,
    Exfiltration,
    HistoryRewrite,
    Publishes,
    KillsProcess,
    SystemChange,
    InstallsPackages,
    RunsCode,
    InlineCode,
    Database,
    UnknownProgram,
    UnresolvedPath,
    Background,
    Unparsed,
    Truncated,
    TooLong,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub flag: Flag,
    pub risk: Risk,
    /// Short plain-language reason, e.g. "deletes node_modules and everything in it".
    pub note: String,
}

/// One program the command would start (wrappers such as `sudo` or `xargs` removed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub struct CommandShape {
    pub program: String,
    /// `test` in `npm test`, `status` in `git status`; empty when the program has none.
    pub sub: String,
    /// Option names used (values removed), sorted, e.g. ["--short", "-n"].
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Shell {
    Bash,
    PowerShell,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub risk: Risk,
    pub shell: Shell,
    /// False when the command could not be fully parsed.
    pub parsed: bool,
    pub flags: Vec<Flag>,
    pub findings: Vec<Finding>,
    pub commands: Vec<CommandShape>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub deletes: Vec<String>,
    /// Hosts or URLs the command talks to.
    pub hosts: Vec<String>,
    /// A rule may auto-allow this only when true (see `eligibility`).
    pub auto_allow_eligible: bool,
    /// Why it is not eligible, in plain words. Empty when eligible.
    pub ineligible_reason: String,
    pub elapsed_us: u64,
}

/// Where the command runs. All paths may be Windows (`C:\x`), mixed (`C:/x`) or Git Bash (`/c/x`).
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The project folder: writes and deletes inside it are normal work.
    pub project: String,
    /// The shell's current folder (often the project).
    pub cwd: String,
    pub home: String,
    /// Temp folders: scratch work there is low risk.
    pub temp: Vec<String>,
    /// Known environment variables used to resolve `$NAME` in paths.
    pub env: HashMap<String, String>,
}

impl Context {
    /// Build from this process's environment for a session whose folder is `cwd`.
    pub fn from_system(project: &str, cwd: &str) -> Context {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let home = if var("USERPROFILE").is_empty() { var("HOME") } else { var("USERPROFILE") };
        let mut env = HashMap::new();
        for k in ["TEMP", "TMP", "TMPDIR", "LOCALAPPDATA", "APPDATA", "USERPROFILE", "PROGRAMFILES", "SYSTEMROOT"] {
            let v = var(k);
            if !v.is_empty() {
                env.insert(k.to_string(), v);
            }
        }
        if !home.is_empty() {
            env.insert("HOME".into(), home.clone());
        }
        // Git Bash on Windows leaves TMPDIR unset; scripts that use it mean the temp folder.
        if !env.contains_key("TMPDIR") {
            if let Some(t) = env.get("TEMP").cloned() {
                env.insert("TMPDIR".into(), t);
            }
        }
        let temp = ["TEMP", "TMP", "TMPDIR"].iter().map(|k| var(k)).filter(|v| !v.is_empty()).collect();
        Context { project: project.to_string(), cwd: cwd.to_string(), home, temp, env }
    }
}

/// Collects findings while a command is analysed.
#[derive(Default)]
pub(crate) struct Builder {
    pub findings: Vec<Finding>,
    pub commands: Vec<CommandShape>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub deletes: Vec<String>,
    pub hosts: Vec<String>,
    /// Reads of secret-like files, for the exfiltration check.
    pub secret_reads: Vec<String>,
    /// Non-local network use that can carry data out.
    pub remote_network: bool,
    pub parsed: bool,
    /// Files fetched from the network by this command (normalised paths).
    pub downloaded: Vec<String>,
    /// Files this command writes, with their text when the command shows it.
    pub created: Vec<(String, Option<String>)>,
}

impl Builder {
    pub fn find(&mut self, flag: Flag, risk: Risk, note: impl Into<String>) {
        let note = note.into();
        if !self.findings.iter().any(|f| f.flag == flag && f.note == note) {
            self.findings.push(Finding { flag, risk, note });
        }
    }
    pub fn push_unique(list: &mut Vec<String>, value: &str) {
        if !value.is_empty() && list.len() < 40 && !list.iter().any(|v| v == value) {
            list.push(value.to_string());
        }
    }
}
