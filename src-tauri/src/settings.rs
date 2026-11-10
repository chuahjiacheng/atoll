//! User settings: one JSON file at `%APPDATA%\Crest\settings.json`.
//!
//! Rust owns the settings. The dashboard sends a full or partial object,
//! we merge it, clamp every value to a safe range, save it (debounced) and
//! broadcast the result to every window so the island updates live.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CURRENT_VERSION: u32 = 5;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub appearance: Appearance,
    pub modules: Modules,
    pub hotkeys: Hotkeys,
    pub alerts: Alerts,
    pub usage: Usage,
    pub chat_watcher: ChatWatcher,
    pub spotify: Spotify,
    pub clipboard: ClipboardSettings,
    pub whiteboard: Whiteboard,
    pub mascot: Mascot,
    pub claude: Claude,
    pub focus: Focus,
    pub calendar: Calendar,
    /// Tab order in the expanded island (module keys). Home is always first.
    pub module_order: Vec<String>,
    // --- recorder-forecast ---
    pub recorder: Recorder,
    // --- end recorder-forecast ---
}

// --- recorder-forecast: flight recorder (docs/contracts/event-store.md §6) ---
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Recorder {
    /// Keep a local, redacted history of every Claude Code event.
    pub enabled: bool,
    /// Delete history older than this many days; 0 = keep forever. 0..=3650.
    pub retention_days: u32,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder { enabled: true, retention_days: 30 }
    }
}
// --- end recorder-forecast ---

/// Focus timer (module switch: `modules.focus`). The timer itself runs in
/// `focus.rs`; the sound at the end of a session is `alerts.timer`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Focus {
    /// One focus session, 5..=90 minutes.
    pub focus_min: u32,
    /// Short break after each session, 1..=30 minutes.
    pub break_min: u32,
    /// Long break, 5..=60 minutes ...
    pub long_min: u32,
    /// ... after every 2..=8 sessions.
    pub long_every: u32,
    /// Sessions run until you end them (the timer counts up).
    pub open_ended: bool,
    /// Do not disturb: during a focus session Atoll's own alerts follow the
    /// three rules below instead of interrupting you.
    pub dnd: bool,
    /// Finished tasks, chat replies and usage warnings:
    /// "hold" (shown together at the break) | "show" | "silent".
    pub finished: String,
    /// Errors: "hold" | "show" | "silent".
    pub errors: String,
    /// Permission requests (Claude is waiting): "quiet" (no sound or
    /// notification) | "show". They always appear, since Claude can't go on.
    pub permissions: String,
    /// Count it, and nudge you, when a listed app comes to the front. Never blocks.
    pub nudge: bool,
    /// App names or words in window titles, matched case-insensitively.
    pub nudge_apps: Vec<String>,
    /// Play Spotify on this PC when a session starts, pause it at the break.
    pub music: bool,
}

impl Default for Focus {
    fn default() -> Self {
        Focus {
            focus_min: 25,
            break_min: 5,
            long_min: 15,
            long_every: 4,
            open_ended: false,
            dnd: true,
            finished: "hold".into(),
            errors: "show".into(),
            permissions: "quiet".into(),
            nudge: false,
            nudge_apps: ["YouTube", "Netflix", "Discord", "Reddit", "Instagram", "TikTok"].iter().map(|s| s.to_string()).collect(),
            music: false,
        }
    }
}

impl Focus {
    fn sanitize(&mut self) {
        self.focus_min = self.focus_min.clamp(5, 90);
        self.break_min = self.break_min.clamp(1, 30);
        self.long_min = self.long_min.clamp(5, 60);
        self.long_every = self.long_every.clamp(2, 8);
        if !["hold", "show", "silent"].contains(&self.finished.as_str()) {
            self.finished = "hold".into();
        }
        if !["hold", "show", "silent"].contains(&self.errors.as_str()) {
            self.errors = "show".into();
        }
        if self.permissions != "show" {
            self.permissions = "quiet".into();
        }
        let mut seen: Vec<String> = Vec::new();
        self.nudge_apps = std::mem::take(&mut self.nudge_apps)
            .into_iter()
            .map(|a| a.trim().chars().take(60).collect::<String>())
            .filter(|a| {
                let key = a.to_lowercase();
                let fresh = !a.is_empty() && !seen.contains(&key);
                if fresh {
                    seen.push(key);
                }
                fresh
            })
            .take(30)
            .collect();
    }
}

/// What the Claude Code module shows and does (module switch: `modules.claude`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Claude {
    /// Show what the agent is doing (file names, commands). Off = status only.
    pub live_activity: bool,
    /// Ask in the island before Claude Code runs a tool. Off = Claude Code asks itself.
    pub ask_permissions: bool,
    /// Play the "Task done" sound when a turn finishes.
    pub sound_on_done: bool,
    /// Pop the "finished" peek open when a turn finishes. Off = "Done" in the pill only.
    pub popup_on_done: bool,
    /// An error or "needs you" message clears itself after this many minutes
    /// (0 = stays until you clear it). "Done" already fades from the pill.
    pub clear_after_min: u32,
}

impl Default for Claude {
    fn default() -> Self {
        Claude { live_activity: true, ask_permissions: true, sound_on_done: true, popup_on_done: true, clear_after_min: 15 }
    }
}

pub fn default_module_order() -> Vec<String> {
    ["claude", "spotify", "calendar", "clipboard", "whiteboard", "fileTray", "note", "focus", "mascot"].iter().map(|s| s.to_string()).collect()
}

/// Clipboard history (module switch: `modules.clipboard`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipboardSettings {
    /// Keep unpinned history after Crest restarts (pinned items are always kept).
    pub persist: bool,
    /// 10..=200
    pub max_items: u32,
    pub images: bool,
    pub files: bool,
    /// Clicking an item pastes it into the app you were using (otherwise copy only).
    pub click_paste: bool,
    /// New copies are not recorded while paused.
    pub paused: bool,
    /// Never record copies that password managers mark as private.
    pub skip_private: bool,
    /// Store history and images encrypted with Windows DPAPI (your account only).
    pub encrypt: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        ClipboardSettings { persist: false, max_items: 50, images: true, files: true, click_paste: true, paused: false, skip_private: true, encrypt: false }
    }
}

/// Whiteboard (module switch: `modules.whiteboard`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Whiteboard {
    /// Every Ctrl+C also lands on the board.
    pub auto_add_copies: bool,
    /// Clicking a card pastes it into the app you were using (otherwise copy only).
    pub click_paste: bool,
}

impl Default for Whiteboard {
    fn default() -> Self {
        Whiteboard { auto_add_copies: false, click_paste: true }
    }
}

/// Spotify module (module switch: `modules.spotify`). The refresh token is
/// NOT stored here; it lives in Windows Credential Manager.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Spotify {
    /// Read Spotify on this PC through Windows' media session (the Connect button).
    pub connected: bool,
    /// Client ID of your own Spotify developer app (public; PKCE needs no secret).
    /// Only needed for the optional device switching.
    pub client_id: String,
    pub lyrics: bool,
    /// Show what's playing in the collapsed island when Claude is idle.
    pub show_in_pill: bool,
    /// Thin progress line along the bottom of the collapsed island.
    pub progress_line: bool,
    /// What the collapsed music pill says: "track" (title · artist) or
    /// "lyrics" (the current synced line; falls back to the track).
    pub pill_text: String,
    /// Who gets the collapsed pill when Claude Code works while music plays:
    /// "claude" or "spotify". Needs-you and errors always show.
    pub pill_priority: String,
    /// Fixed 127.0.0.1 port for the sign-in redirect. Spotify's dashboard wants
    /// an explicit port, so the redirect URI you register must use this one.
    pub callback_port: u16,
}

pub const DEFAULT_CALLBACK_PORT: u16 = 43117;

impl Default for Spotify {
    fn default() -> Self {
        Spotify {
            connected: false,
            client_id: String::new(),
            lyrics: true,
            show_in_pill: true,
            progress_line: true,
            pill_text: "track".into(),
            pill_priority: "claude".into(),
            callback_port: DEFAULT_CALLBACK_PORT,
        }
    }
}

/// Google Calendar module (module switch: `modules.calendar`). The client
/// secret and refresh token are NOT stored here; they live in Windows
/// Credential Manager.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Calendar {
    /// Client ID of your own Google Cloud OAuth client ("Desktop app").
    pub client_id: String,
    /// Your on/off choice per calendar id; calendars not listed follow
    /// Google's own "show in my list" choice.
    pub shown: BTreeMap<String, bool>,
    /// How many days the island lists, today included (1..=14).
    pub days_ahead: u32,
    /// List events you declined (dimmed).
    pub show_declined: bool,
    /// Show the next event in the collapsed island this many minutes before
    /// it starts (0 = never).
    pub pill_minutes: u32,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar { client_id: String::new(), shown: BTreeMap::new(), days_ahead: 7, show_declined: false, pill_minutes: 10 }
    }
}

/// How Atoll tells you something happened (module switch: `modules.alerts`).
/// Each event has its own sound, volume and Windows notification.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Alerts {
    /// Pulse the island in the event's colour.
    pub flash: bool,
    pub done: EventAlert,
    pub error: EventAlert,
    pub permission: EventAlert,
    pub usage: EventAlert,
    pub chat: EventAlert,
    pub timer: EventAlert,
    /// Warn once per window when the 5-hour usage passes this percentage.
    pub usage_threshold: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EventAlert {
    /// "chime" | "pop" | "bell" | "wood" | "none"
    pub sound: String,
    /// 0..=100
    pub volume: u32,
    /// Show a Windows notification.
    pub notify: bool,
}

impl EventAlert {
    fn new(sound: &str, volume: u32, notify: bool) -> Self {
        EventAlert { sound: sound.into(), volume, notify }
    }
    fn sanitize(&mut self) {
        if !SOUNDS.contains(&self.sound.as_str()) {
            self.sound = "chime".into();
        }
        self.volume = self.volume.min(100);
    }
}

impl Default for EventAlert {
    fn default() -> Self {
        EventAlert::new("chime", 70, true)
    }
}

pub const SOUNDS: [&str; 5] = ["chime", "pop", "bell", "wood", "none"];

/// Usage ring (module switch: `modules.usage_ring`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Usage {
    pub show_in_pill: bool,
    /// "fiveHour" or "sevenDay": which window the pill ring shows.
    pub ring_window: String,
    /// Ask Anthropic for the usage percentages with Claude Code's own sign-in
    /// (the desktop app has no status line). Off = status line only.
    pub from_sign_in: bool,
}

/// Experimental Chat tab watcher (module switch: `modules.chat_watcher`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatWatcher {
    /// Only alert while the Claude window is not the active window.
    pub only_when_unfocused: bool,
}

impl Default for Alerts {
    fn default() -> Self {
        // The design's defaults; usage and chat replies are Atoll's own events.
        Alerts {
            flash: true,
            done: EventAlert::new("chime", 70, true),
            error: EventAlert::new("bell", 80, true),
            permission: EventAlert::new("pop", 80, true),
            usage: EventAlert::new("bell", 60, true),
            chat: EventAlert::new("pop", 60, true),
            timer: EventAlert::new("chime", 60, false),
            usage_threshold: 90,
        }
    }
}

impl Default for Usage {
    fn default() -> Self {
        Usage { show_in_pill: true, ring_window: "fiveHour".into(), from_sign_in: true }
    }
}

impl Default for ChatWatcher {
    fn default() -> Self {
        ChatWatcher { only_when_unfocused: true }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct General {
    pub start_with_windows: bool,
    /// Show Atoll's icon in the taskbar's notification area.
    pub show_tray: bool,
    /// Hide the island while a fullscreen app (game, video) is in front.
    pub hide_fullscreen: bool,
    /// Interface language. English only for now.
    pub language: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    /// "compact" | "standard" | "large" | "custom"
    pub preset: String,
    /// Overall size of the island, 25..=100 % (the whole island zooms, text stays sharp).
    pub scale: u32,
    /// Horizontal position of the island's centre, 0..=100 % of the screen width (50 = centre).
    pub position_x: u32,
    /// Gap between the top of the screen and the island, 0..=400 logical px.
    pub position_y: u32,
    /// Collapsed island with a status showing (logical px). Idle is one third of the width.
    pub collapsed_width: u32,
    pub collapsed_height: u32,
    /// Expanded panel (logical px).
    pub expanded_width: u32,
    pub expanded_height: u32,
    /// 0..=100 % of a full pill.
    pub roundness: u32,
    /// Glass opacity, 50..=100 %.
    pub opacity: u32,
    /// Background blur, 0..=48 px (0 = off, or clear glass when Liquid).
    pub blur: u32,
    /// "liquid" (refracting glass drawn by Atoll) | "frosted" (Windows' blur).
    pub glass: String,
    /// Liquid only: how far the rim bends what lies behind it, 0..=100 %.
    pub distortion: u32,
    /// Liquid only: rim light, colour split and saturation, 0..=100 %.
    pub liquid: u32,
    /// "follow" (Windows) | "dark" | "light"
    pub theme: String,
    pub accent: ColorChoice,
    pub busy: ColorChoice,
    pub done: ColorChoice,
    pub error: ColorChoice,
    /// 50..=200 %; durations are divided by this.
    pub animation_speed: u32,
    /// Replace movement with short fades.
    pub reduce_motion: bool,
    /// "primary" or a monitor name from the monitor list.
    pub monitor: String,
}

/// A status or accent colour: a named preset, or a custom hex the UI has
/// checked for 4.5:1 contrast.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorChoice {
    pub preset: String,
    pub custom: String,
    pub custom_on: bool,
}

impl ColorChoice {
    fn preset(id: &str) -> Self {
        ColorChoice { preset: id.into(), custom: String::new(), custom_on: false }
    }
}

impl Default for ColorChoice {
    fn default() -> Self {
        ColorChoice::preset("")
    }
}

/// Pip, the mascot (module switch: `modules.mascot`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Mascot {
    /// "peri" | "mint" | "peach" | "lilac"
    pub tone: String,
    pub react_music: bool,
    pub react_pokes: bool,
}

impl Default for Mascot {
    fn default() -> Self {
        Mascot { tone: "peri".into(), react_music: true, react_pokes: true }
    }
}

/// One switch per module. Modules that are not built yet keep their switch
/// so the choice is remembered for when they ship.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Modules {
    pub claude: bool,
    pub alerts: bool,
    pub usage_ring: bool,
    pub chat_watcher: bool,
    pub spotify: bool,
    pub clipboard: bool,
    pub whiteboard: bool,
    pub file_tray: bool,
    pub note: bool,
    pub focus: bool,
    pub calendar: bool,
    pub mascot: bool,
    pub screen_question: bool,
}

/// Global hotkeys as strings like "Ctrl+Alt+Space" (key part = KeyboardEvent.code).
/// Empty string = not set.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Hotkeys {
    pub toggle_island: String,
    pub expand_island: String,
    pub open_settings: String,
    /// Per-module hotkeys, keyed by the module's field name in `Modules`.
    pub modules: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: CURRENT_VERSION,
            general: General::default(),
            appearance: Appearance::default(),
            modules: Modules::default(),
            hotkeys: Hotkeys::default(),
            alerts: Alerts::default(),
            usage: Usage::default(),
            chat_watcher: ChatWatcher::default(),
            spotify: Spotify::default(),
            clipboard: ClipboardSettings::default(),
            whiteboard: Whiteboard::default(),
            mascot: Mascot::default(),
            claude: Claude::default(),
            focus: Focus::default(),
            calendar: Calendar::default(),
            module_order: default_module_order(),
            recorder: Recorder::default(), // recorder-forecast
        }
    }
}

impl Default for General {
    fn default() -> Self {
        General { start_with_windows: false, show_tray: true, hide_fullscreen: true, language: "en-GB".into() }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            preset: "standard".into(),
            scale: 85,
            position_x: 50,
            position_y: 8,
            collapsed_width: 384,
            collapsed_height: 40,
            expanded_width: 720,
            expanded_height: 440,
            roundness: 100,
            opacity: 85,
            blur: 24,
            glass: "liquid".into(),
            distortion: 60,
            liquid: 60,
            theme: "follow".into(),
            accent: ColorChoice::preset("peri"),
            busy: ColorChoice::preset("amber"),
            done: ColorChoice::preset("green"),
            error: ColorChoice::preset("red"),
            animation_speed: 100,
            reduce_motion: false,
            monitor: "primary".into(),
        }
    }
}

impl Default for Modules {
    fn default() -> Self {
        Modules {
            claude: true,
            alerts: true,
            usage_ring: true,
            chat_watcher: false, // experimental, opt-in
            spotify: true,
            clipboard: true,
            whiteboard: true,
            file_tray: true,
            note: true,
            focus: true,
            calendar: true,
            mascot: true,
            screen_question: true,
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            toggle_island: "Ctrl+Alt+Space".into(),
            expand_island: "Ctrl+Alt+Period".into(),
            open_settings: "Ctrl+Alt+KeyS".into(),
            modules: default_module_hotkeys(),
        }
    }
}

/// Design defaults: clipboard V, note N, focus F, music P (all Ctrl+Alt).
fn default_module_hotkeys() -> BTreeMap<String, String> {
    [
        ("clipboard", "Ctrl+Alt+KeyV"),
        ("note", "Ctrl+Alt+KeyN"),
        ("focus", "Ctrl+Alt+KeyF"),
        ("spotify", "Ctrl+Alt+KeyP"),
        // Answer a permission request from anywhere (the island never takes focus).
        ("allowPermission", "Ctrl+Alt+KeyY"),
        ("denyPermission", "Ctrl+Alt+KeyX"),
        // Clear whatever message the island is showing.
        ("dismissMessage", "Ctrl+Alt+KeyD"),
    ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn fix_choice(c: &mut ColorChoice, presets: &[&str]) {
    if !presets.contains(&c.preset.as_str()) {
        c.preset = presets[0].into();
    }
    c.custom = c.custom.trim().chars().take(7).collect();
    if !is_hex_color(&c.custom) {
        c.custom_on = false;
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}


impl Settings {
    /// Clamp every value into its allowed range so a bad file or a bug in
    /// the UI can never make the island unusable.
    pub fn sanitize(&mut self) {
        let a = &mut self.appearance;
        if !["compact", "standard", "large", "custom"].contains(&a.preset.as_str()) {
            a.preset = "custom".into();
        }
        a.scale = a.scale.clamp(25, 100);
        a.position_x = a.position_x.min(100);
        a.position_y = a.position_y.min(400);
        a.collapsed_width = a.collapsed_width.clamp(160, 560);
        a.collapsed_height = a.collapsed_height.clamp(28, 56);
        a.expanded_width = a.expanded_width.clamp(480, 960);
        a.expanded_height = a.expanded_height.clamp(280, 640);
        a.roundness = a.roundness.min(100);
        a.opacity = a.opacity.clamp(50, 100);
        a.blur = a.blur.min(48);
        if !["liquid", "frosted"].contains(&a.glass.as_str()) {
            a.glass = "liquid".into();
        }
        a.distortion = a.distortion.min(100);
        a.liquid = a.liquid.min(100);
        if !["follow", "dark", "light"].contains(&a.theme.as_str()) {
            a.theme = "follow".into();
        }
        fix_choice(&mut a.accent, &["peri", "teal", "orchid", "coral"]);
        fix_choice(&mut a.busy, &["amber", "orange"]);
        fix_choice(&mut a.done, &["green", "teal"]);
        fix_choice(&mut a.error, &["red", "pink"]);
        a.animation_speed = a.animation_speed.clamp(50, 200);
        if a.monitor.trim().is_empty() {
            a.monitor = "primary".into();
        }
        if !["peri", "mint", "peach", "lilac"].contains(&self.mascot.tone.as_str()) {
            self.mascot.tone = "peri".into();
        }
        let known = default_module_order();
        self.module_order.retain(|m| known.contains(m));
        self.module_order.dedup();
        for m in known {
            if !self.module_order.contains(&m) {
                self.module_order.push(m);
            }
        }
        self.hotkeys.modules.retain(|_, v| !v.trim().is_empty());
        for e in [
            &mut self.alerts.done,
            &mut self.alerts.error,
            &mut self.alerts.permission,
            &mut self.alerts.usage,
            &mut self.alerts.chat,
            &mut self.alerts.timer,
        ] {
            e.sanitize();
        }
        self.general.language = "en-GB".into();
        self.alerts.usage_threshold = self.alerts.usage_threshold.clamp(50, 100);
        // Spotify client IDs are 32 hex chars; keep only safe characters.
        self.spotify.client_id = self.spotify.client_id.trim().chars().filter(|c| c.is_ascii_alphanumeric()).take(64).collect();
        self.clipboard.max_items = self.clipboard.max_items.clamp(10, 200);
        if self.spotify.callback_port < 1024 {
            self.spotify.callback_port = DEFAULT_CALLBACK_PORT;
        }
        if self.spotify.pill_text != "lyrics" {
            self.spotify.pill_text = "track".into();
        }
        if self.spotify.pill_priority != "spotify" {
            self.spotify.pill_priority = "claude".into();
        }
        if self.usage.ring_window != "sevenDay" {
            self.usage.ring_window = "fiveHour".into();
        }
        self.claude.clear_after_min = self.claude.clear_after_min.min(24 * 60);
        self.focus.sanitize();
        // Google client IDs look like 1234-abc.apps.googleusercontent.com.
        self.calendar.client_id =
            self.calendar.client_id.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).take(200).collect();
        self.calendar.days_ahead = self.calendar.days_ahead.clamp(1, 14);
        self.calendar.pill_minutes = self.calendar.pill_minutes.min(60);
        self.calendar.shown.retain(|k, _| !k.is_empty() && k.len() <= 300);
        self.recorder.retention_days = self.recorder.retention_days.min(3650); // recorder-forecast
        self.version = CURRENT_VERSION;
    }

    /// Apply a (partial) JSON object on top of these settings.
    pub fn merged(&self, patch: &Value) -> Result<Settings, String> {
        let mut base = serde_json::to_value(self).map_err(|e| e.to_string())?;
        merge(&mut base, patch);
        let mut next: Settings = serde_json::from_value(base).map_err(|e| format!("invalid settings: {e}"))?;
        next.sanitize();
        Ok(next)
    }
}

/// Deep-merge `patch` into `base` (objects merge, everything else replaces).
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

pub fn path() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
        .join("Crest")
        .join("settings.json")
}

/// Load from disk; a missing or broken file gives defaults (a broken one is
/// kept aside as `settings.json.broken` so nothing is silently lost).
pub fn load() -> Settings {
    let p = path();
    let raw: Option<Value> = std::fs::read_to_string(&p).ok().and_then(|text| match serde_json::from_str::<Value>(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[atoll] settings.json unreadable ({e}); using defaults");
            let _ = std::fs::rename(&p, p.with_extension("json.broken"));
            None
        }
    });
    let mut s = match raw.as_ref().map(|v| serde_json::from_value::<Settings>(v.clone())) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            eprintln!("[atoll] settings.json has an unexpected shape ({e}); using defaults");
            let _ = std::fs::rename(&p, p.with_extension("json.broken"));
            Settings::default()
        }
        None => Settings::default(),
    };
    if s.version < 2 {
        migrate_v1(&mut s);
    }
    if s.version < 3 {
        // v3 adds the Allow / Deny hotkeys.
        s.hotkeys.modules.entry("allowPermission".into()).or_insert_with(|| "Ctrl+Alt+KeyY".into());
        s.hotkeys.modules.entry("denyPermission".into()).or_insert_with(|| "Ctrl+Alt+KeyX".into());
    }
    if s.version < 4 {
        if let Some(old) = raw.as_ref().and_then(|v| v.get("alerts")) {
            migrate_alerts_v3(&mut s.alerts, old);
        }
    }
    if s.version < 5 {
        // v5 adds the Dismiss hotkey, unless that combination is already yours.
        let taken = s.hotkeys.modules.values().chain([&s.hotkeys.toggle_island, &s.hotkeys.expand_island, &s.hotkeys.open_settings]).any(|v| v.eq_ignore_ascii_case("Ctrl+Alt+KeyD"));
        if !taken {
            s.hotkeys.modules.entry("dismissMessage".into()).or_insert_with(|| "Ctrl+Alt+KeyD".into());
        }
    }
    s.sanitize();
    s
}

/// Version 1 had a free background colour and transparency; version 2 uses
/// the design's opacity / blur / theme. Old hotkeys that were never changed
/// move to the design's defaults.
fn migrate_v1(s: &mut Settings) {
    s.appearance = Appearance::default();
    if s.hotkeys.open_settings == "Ctrl+Alt+Comma" {
        s.hotkeys.open_settings = "Ctrl+Alt+KeyS".into();
    }
    for (k, v) in default_module_hotkeys() {
        s.hotkeys.modules.entry(k).or_insert(v);
    }
}

/// Version 3 had one sound / toast switch for everything plus per-trigger
/// switches; version 4 has a sound, volume and notification per event.
fn migrate_alerts_v3(a: &mut Alerts, old: &Value) {
    let flag = |k: &str| old.get(k).and_then(Value::as_bool);
    let sound_on = flag("sound").unwrap_or(true);
    let toast_on = flag("toast").unwrap_or(true);
    if let Some(f) = flag("flash") {
        a.flash = f;
    }
    let volume = old.get("volume").and_then(Value::as_u64).map(|v| v.min(100) as u32);
    for (event, trigger) in [
        (&mut a.done, "onDone"),
        (&mut a.permission, "onAttention"),
        (&mut a.error, "onError"),
        (&mut a.usage, "onUsage"),
        (&mut a.chat, ""),
    ] {
        let wanted = trigger.is_empty() || flag(trigger).unwrap_or(true);
        if !sound_on || !wanted {
            event.sound = "none".into();
        }
        if !toast_on || !wanted {
            event.notify = false;
        }
        if let Some(v) = volume {
            event.volume = v;
        }
    }
}

pub fn save(s: &Settings) -> Result<(), String> {
    let p = path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_patch_merges_and_clamps() {
        let s = Settings::default();
        let next = s
            .merged(&json!({ "appearance": { "collapsedWidth": 5000, "opacity": 10, "theme": "neon", "busy": { "preset": "nope", "custom": "#ABCDEF", "customOn": true } } }))
            .unwrap();
        assert_eq!(next.appearance.collapsed_width, 560);
        assert_eq!(next.appearance.scale, 85);
        assert_eq!(next.appearance.opacity, 50);
        assert_eq!(next.appearance.theme, "follow");
        assert_eq!(next.appearance.busy.preset, "amber");
        assert!(next.appearance.busy.custom_on);
        assert_eq!(next.appearance.done.preset, "green"); // untouched
        assert_eq!(next.hotkeys.toggle_island, "Ctrl+Alt+Space");
    }

    #[test]
    fn unknown_pill_choices_fall_back() {
        let mut s: Settings = serde_json::from_value(json!({ "spotify": { "pillText": "karaoke", "pillPriority": "both" } })).unwrap();
        s.sanitize();
        assert_eq!((s.spotify.pill_text.as_str(), s.spotify.pill_priority.as_str()), ("track", "claude"));
        let mut s: Settings = serde_json::from_value(json!({ "spotify": { "pillText": "lyrics", "pillPriority": "spotify" } })).unwrap();
        s.sanitize();
        assert_eq!((s.spotify.pill_text.as_str(), s.spotify.pill_priority.as_str()), ("lyrics", "spotify"));
    }

    #[test]
    fn focus_choices_are_clamped_and_cleaned() {
        let s = Settings::default()
            .merged(&json!({ "focus": {
                "focusMin": 500, "breakMin": 0, "longMin": 2, "longEvery": 99,
                "finished": "later", "errors": "silent", "permissions": "loud",
                "nudgeApps": ["  YouTube ", "youtube", "", "Discord"]
            } }))
            .unwrap();
        let f = &s.focus;
        assert_eq!((f.focus_min, f.break_min, f.long_min, f.long_every), (90, 1, 5, 8));
        assert_eq!((f.finished.as_str(), f.errors.as_str(), f.permissions.as_str()), ("hold", "silent", "quiet"));
        assert_eq!(f.nudge_apps, vec!["YouTube".to_string(), "Discord".to_string()]);
        // Old settings files have no focus section: defaults fill it in.
        let old: Settings = serde_json::from_value(json!({ "appearance": {} })).unwrap();
        assert_eq!(old.focus, Focus::default());
    }

    #[test]
    fn v3_alert_switches_carry_over() {
        let mut a = Alerts::default();
        migrate_alerts_v3(&mut a, &json!({ "sound": true, "toast": false, "onError": false, "volume": 40 }));
        assert_eq!(a.done.sound, "chime");
        assert!(!a.done.notify);
        assert_eq!(a.error.sound, "none");
        assert_eq!(a.permission.volume, 40);
    }

    #[test]
    fn old_or_partial_file_fills_defaults() {
        let s: Settings = serde_json::from_value(json!({ "modules": { "spotify": false } })).unwrap();
        assert!(!s.modules.spotify);
        assert!(s.modules.claude);
        assert_eq!(s.appearance.expanded_width, 720);
        assert_eq!(s.module_order[0], "claude");
    }
}
