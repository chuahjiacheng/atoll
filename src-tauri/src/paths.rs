//! Well-known file locations (Windows only).

use std::path::PathBuf;

fn env_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| ".".into()))
}

/// `%LOCALAPPDATA%\Crest` - machine-local state (runtime file, bridge exe).
pub fn local_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("Crest")
}

/// Port + token for the bridge. Recreated on every launch, deleted on exit.
pub fn runtime_file() -> PathBuf {
    local_dir().join("runtime.json")
}

/// Stable location the hooks point at, so rebuilding the app never breaks them.
pub fn installed_bridge() -> PathBuf {
    local_dir().join("bin").join("crest-bridge.exe")
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
