//! Well-known file locations (Windows only).

use std::path::PathBuf;

fn env_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| ".".into()))
}

/// `%LOCALAPPDATA%\Crest` - machine-local state (runtime file, bridge exe).
pub fn local_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("Crest")
}

/// `%USERPROFILE%\.atoll` - files the bridge must find, whoever launches it.
///
/// Not under AppData on purpose: the Claude desktop app is an MSIX package, and
/// Windows gives packaged apps (and every hook they run) a private copy of
/// AppData. A runtime file in AppData written by the installed Atoll is then
/// hidden behind a stale private copy, and the Code tab's hooks can't reach the
/// island. The user profile root is not virtualized, so every process sees the
/// same file here.
pub fn shared_dir() -> PathBuf {
    env_dir("USERPROFILE").join(".atoll")
}

/// Port + token for the bridge. Recreated on every launch, deleted on exit.
pub fn runtime_file() -> PathBuf {
    shared_dir().join("runtime.json")
}

/// Where versions up to 1.0.2 kept the runtime file (removed on start).
pub fn legacy_runtime_file() -> PathBuf {
    local_dir().join("runtime.json")
}

/// Stable location the hooks point at, so rebuilding the app never breaks them.
pub fn installed_bridge() -> PathBuf {
    shared_dir().join("bin").join("crest-bridge.exe")
}

/// Where versions up to 1.0.2 put the bridge the hooks pointed at.
pub fn legacy_bridge_dir() -> PathBuf {
    local_dir().join("bin")
}

/// The freshly built bridge, next to the running app exe (target\debug or target\release).
pub fn built_bridge() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("crest-bridge.exe"))
}

/// Claude Code user settings: `%USERPROFILE%\.claude\settings.json`.
pub fn claude_settings() -> PathBuf {
    claude_dir().join("settings.json")
}

/// Claude Code's folder (settings, session logs): CLAUDE_CONFIG_DIR, else ~/.claude.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| env_dir("USERPROFILE").join(".claude"))
}
