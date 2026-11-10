//! Focus mode: a focus / break timer that knows about your Claude Code sessions.
//!
//! One state machine (ready → focus → break → ready), kept here and pushed to
//! every window as a `focus` event. While a focus session runs:
//! - Atoll's own alerts follow Settings → Focus timer: `alerts::fire` asks
//!   `intercept`, and held alerts are collected into a digest shown at the break;
//! - optionally, a listed app coming to the front is counted and nudged
//!   (the foreground window, checked locally; nothing is ever blocked);
//! - optionally, Spotify on this PC plays at the start and pauses at the break.
//!
//! Focus time goes into a small history on this PC (today, streak, last 7 days).
//! Windows' own Do Not Disturb can't be switched by apps (no public API), so
//! "Do not disturb" here covers Atoll's own sounds, notifications and pop-ups.

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::alerts::{Alert, Kind};
use crate::media::{load_json, now_ms, save_json};
use crate::AppState;

const MINUTE: u64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// A focus block this long counts as a session in the history (and the streak).
const SESSION_MIN_MS: u64 = 5 * MINUTE;
/// How often the front window is checked while the nudge is on.
const NUDGE_POLL: Duration = Duration::from_millis(1500);
/// Held alerts kept for the digest (newest win).
const MAX_HELD: usize = 20;
/// Days of history kept.
const KEEP_DAYS: i64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Ready,
    Focus,
    Break,
}

/// One alert held back during focus, shown in the digest at the break.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub kind: Kind,
    pub session_id: Option<String>,
    pub title: String,
    pub body: String,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nudge {
    /// The list entry that matched ("YouTube").
    pub app: String,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayBar {
    /// "2026-11-07" (local date)
    pub date: String,
    /// "Sat"
    pub label: String,
    pub focus_ms: u64,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub today_ms: u64,
    pub today_sessions: u32,
    /// Days in a row with at least one session (today counts once it has one).
    pub streak: u32,
    /// The last 7 days, oldest first, ending today.
    pub week: Vec<DayBar>,
}

/// Everything the island and Settings show about focus.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusView {
    pub phase: Phase,
    pub running: bool,
    /// Length of this phase (ms). Ready: the next session's length. 0 = no time limit.
    pub length_ms: u64,
    /// Time run in this phase, measured at `at`; add the time since while `running`.
    pub elapsed_ms: u64,
    pub at: u64,
    /// "Focus 2 of 4"
    pub round: u32,
    pub rounds: u32,
    pub long_break: bool,
    pub intention: String,
    pub held: Vec<Held>,
    /// Show the held alerts now (the break started and you haven't dismissed them).
    pub digest_open: bool,
    pub distractions: u32,
    pub nudge: Option<Nudge>,
    pub stats: Stats,
}

struct Timer {
    phase: Phase,
    /// Planned length of this phase (ms); 0 = open-ended focus.
    length_ms: u64,
    /// Time run in this phase before `since` (pauses split it up).
    banked_ms: u64,
    /// Running since (ms since epoch); None = paused, or ready.
    since: Option<u64>,
    round: u32,
    long_break: bool,
    intention: String,
    held: Vec<Held>,
    digest_open: bool,
    distractions: u32,
    /// The distraction now in front, so each switch to it counts once.
    distracted_by: Option<String>,
    nudge: Option<Nudge>,
}

impl Timer {
    const fn new() -> Self {
        Timer {
            phase: Phase::Ready,
            length_ms: 0,
            banked_ms: 0,
            since: None,
            round: 1,
            long_break: false,
            intention: String::new(),
            held: Vec::new(),
            digest_open: false,
            distractions: 0,
            distracted_by: None,
            nudge: None,
        }
    }

    fn elapsed(&self, now: u64) -> u64 {
        self.banked_ms + self.since.map(|s| now.saturating_sub(s)).unwrap_or(0)
    }

    /// Start a phase now.
    fn begin(&mut self, phase: Phase, length_ms: u64, now: u64) {
        self.phase = phase;
        self.length_ms = length_ms;
        self.banked_ms = 0;
        self.since = Some(now);
    }

    /// Back to ready for the next round (after a break, or a skipped one).
    fn next_round(&mut self, rounds: u32) {
        self.round = if self.long_break || self.round >= rounds { 1 } else { self.round + 1 };
        self.phase = Phase::Ready;
        self.long_break = false;
        self.length_ms = 0;
        self.banked_ms = 0;
        self.since = None;
    }
}

static TIMER: Mutex<Timer> = Mutex::new(Timer::new());
static WAKE: Mutex<bool> = Mutex::new(false);
static WAKE_CV: Condvar = Condvar::new();
static TRAY_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();

/// Wake the timer thread (something changed).
fn poke() {
    *WAKE.lock().unwrap() = true;
    WAKE_CV.notify_all();
}

fn settings(app: &AppHandle) -> crate::settings::Settings {
    app.state::<AppState>().settings.lock().unwrap().clone()
}

/// Things to do after the timer lock is released (they may call back into this module).
#[derive(Default)]
struct After {
    alert: Option<(String, String, &'static str)>,
    music: Option<&'static str>,
    record: Option<(u64, u32)>,
}

impl After {
    fn run(self, app: &AppHandle) {
        if let Some((focus_ms, distractions)) = self.record {
            record(focus_ms, distractions);
        }
        if let Some((title, body, key)) = self.alert {
            crate::alerts::fire(app, Alert { kind: Kind::Timer, session_id: Some(key.into()), title, body }, false);
        }
        if let Some(action) = self.music {
            music(app, action);
        }
    }
}

/// Media tie-in: play Spotify at the start of a session, pause it at the break.
fn music(app: &AppHandle, action: &'static str) {
    let s = settings(app);
    if !(s.focus.music && s.modules.spotify && s.spotify.connected) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = crate::spotify_local::command(action, None) {
            eprintln!("[atoll] focus music {action}: {e}");
        }
    });
}

fn minutes(ms: u64) -> String {
    let m = ms / MINUTE;
    if m == 1 { "1 minute".into() } else { format!("{m} minutes") }
}

/// End the focus phase now: record it and start the break.
fn focus_to_break(t: &mut Timer, f: &crate::settings::Focus, now: u64, after: &mut After) {
    let ran = t.elapsed(now);
    after.record = Some((ran, t.distractions));
    let long = t.round >= f.long_every;
    let length = if long { f.long_min } else { f.break_min } as u64 * MINUTE;
    t.long_break = long;
    t.begin(Phase::Break, length, now);
    t.digest_open = !t.held.is_empty();
    t.distracted_by = None;
    t.nudge = None;
    let waiting = match t.held.len() {
        0 => String::new(),
        1 => " 1 update waited for you.".into(),
        n => format!(" {n} updates waited for you."),
    };
    let kind = if long { "long break" } else { "break" };
    after.alert = Some(("Focus session over".into(), format!("Time for a {} {kind}.{waiting}", minutes(length)), "focus-end"));
    after.music = Some("pause");
}

/// Move on when the running phase's time is up.
fn advance(app: &AppHandle) {
    let s = settings(app);
    let now = now_ms();
    let mut after = After::default();
    {
        let mut t = TIMER.lock().unwrap();
        if t.since.is_none() || t.length_ms == 0 || t.elapsed(now) < t.length_ms {
            return;
        }
        match t.phase {
            Phase::Focus => {
                // End at the planned moment (the check may run a little late), so
                // exactly the planned length is recorded and the break starts then.
                let end = t.since.unwrap() + t.length_ms.saturating_sub(t.banked_ms);
                focus_to_break(&mut t, &s.focus, end.min(now), &mut after);
            }
            Phase::Break => {
                t.next_round(s.focus.long_every);
                after.alert = Some(("Break over".into(), format!("Ready for focus {} of {}.", t.round, s.focus.long_every), "break-end"));
            }
            Phase::Ready => return,
        }
    }
    after.run(app);
    emit(app);
}

// ---------- actions (island buttons, hotkey, tray) ----------

/// start | pause | resume | toggle | skip | reset | intention | dismiss | clearHistory
pub fn action(app: &AppHandle, action: &str, text: Option<String>) -> Result<FocusView, String> {
    let s = settings(app);
    if !s.modules.focus && action != "clearHistory" {
        return Err("Focus timer is turned off in Settings → Modules.".into());
    }
    let now = now_ms();
    let mut after = After::default();
    {
        let mut t = TIMER.lock().unwrap();
        let running = t.since.is_some();
        match action {
            "start" | "toggle" | "pause" | "resume" => {
                let wanted = match action {
                    "toggle" => if t.phase == Phase::Ready { "start" } else if running { "pause" } else { "resume" },
                    "start" if t.phase != Phase::Ready && !running => "resume",
                    a => a,
                };
                match wanted {
                    "start" => {
                        if t.phase == Phase::Break {
                            // Start another round straight from the break.
                            t.next_round(s.focus.long_every);
                        }
                        if t.phase == Phase::Ready {
                            if let Some(i) = text {
                                t.intention = clean_intention(&i);
                            }
                            let length = if s.focus.open_ended { 0 } else { s.focus.focus_min as u64 * MINUTE };
                            t.begin(Phase::Focus, length, now);
                            t.held.clear();
                            t.digest_open = false;
                            t.distractions = 0;
                            t.distracted_by = None;
                            t.nudge = None;
                            after.music = Some("play");
                        }
                    }
                    "pause" => {
                        if let Some(since) = t.since.take() {
                            t.banked_ms += now.saturating_sub(since);
                        }
                    }
                    _ => {
                        if t.phase != Phase::Ready && t.since.is_none() {
                            t.since = Some(now);
                        }
                    }
                }
            }
            "skip" => match t.phase {
                Phase::Focus => focus_to_break(&mut t, &s.focus, now, &mut after),
                Phase::Break => t.next_round(s.focus.long_every),
                Phase::Ready => {}
            },
            "reset" => {
                if t.phase == Phase::Focus {
                    after.record = Some((t.elapsed(now), t.distractions));
                }
                let held = std::mem::take(&mut t.held);
                *t = Timer::new();
                // Nothing held back is lost: it shows now.
                t.digest_open = !held.is_empty();
                t.held = held;
            }
            "intention" => t.intention = clean_intention(&text.unwrap_or_default()),
            "dismiss" => {
                t.digest_open = false;
                t.held.clear();
            }
            "clearHistory" => {
                drop(t);
                clear_history();
                emit(app);
                return Ok(view(app));
            }
            other => return Err(format!("unknown focus action {other}")),
        }
    }
    after.run(app);
    poke();
    emit(app);
    Ok(view(app))
}

fn clean_intention(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect()
}

/// Ctrl+Alt+F and the tray: start, pause or resume.
pub fn toggle(app: &AppHandle) {
    if let Err(e) = action(app, "toggle", None) {
        eprintln!("[atoll] focus: {e}");
    }
}

// ---------- alerts during focus ----------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Decision {
    /// Sound, notification and island as usual.
    Deliver,
    /// Island only: no sound, no notification.
    Quiet,
    /// Kept for the digest at the break.
    Hold,
    /// Dropped (the island still shows the session's status).
    Drop,
}

/// The rule for one alert kind during a focus session (pure, for tests).
pub fn rule_for(f: &crate::settings::Focus, kind: Kind) -> Decision {
    let pick = |rule: &str| match rule {
        "show" => Decision::Deliver,
        "silent" => Decision::Drop,
        _ => Decision::Hold,
    };
    if !f.dnd {
        return Decision::Deliver;
    }
    match kind {
        Kind::Timer => Decision::Deliver,
        // Claude can't go on without you: it always shows, quietly by default.
        Kind::Attention => if f.permissions == "show" { Decision::Deliver } else { Decision::Quiet },
        Kind::Error => pick(&f.errors),
        Kind::Done | Kind::ChatReply | Kind::Usage => pick(&f.finished),
    }
}

/// Called by `alerts::fire` for every real alert.
pub fn intercept(app: &AppHandle, alert: &Alert) -> Decision {
    let s = settings(app);
    let in_focus = TIMER.lock().unwrap().phase == Phase::Focus;
    if !s.modules.focus || !in_focus {
        return Decision::Deliver;
    }
    let decision = rule_for(&s.focus, alert.kind);
    if decision == Decision::Hold {
        {
            let mut t = TIMER.lock().unwrap();
            // One entry per session and kind: the newest says the most.
            t.held.retain(|h| !(h.kind == alert.kind && h.session_id == alert.session_id));
            t.held.push(Held {
                kind: alert.kind,
                session_id: alert.session_id.clone(),
                title: alert.title.clone(),
                body: alert.body.clone(),
                at: now_ms(),
            });
            let extra = t.held.len().saturating_sub(MAX_HELD);
            t.held.drain(..extra);
        }
        emit(app);
    }
    decision
}

// ---------- distraction nudge ----------

/// The list entry matching the app in front (its program name or window title).
pub fn match_distraction(apps: &[String], exe: &str, title: &str) -> Option<String> {
    let (exe, title) = (exe.to_lowercase(), title.to_lowercase());
    apps.iter()
        .find(|a| {
            let a = a.trim().to_lowercase();
            !a.is_empty() && (exe.contains(&a) || title.contains(&a))
        })
        .cloned()
}

/// Program name (without .exe) and title of the window in front, unless it's Atoll.
fn front_window() -> Option<(String, String)> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        let title = String::from_utf16_lossy(&title[..n]);
        let mut exe = String::new();
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !process.is_null() {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) != 0 {
                let path = String::from_utf16_lossy(&buf[..len as usize]);
                exe = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            }
            CloseHandle(process);
        }
        Some((exe, title))
    }
}

/// Count and nudge when a listed app comes to the front. Returns true if it nudged.
fn check_nudge(app: &AppHandle, apps: &[String]) {
    let hit = front_window().and_then(|(exe, title)| match_distraction(apps, &exe, &title));
    let nudged = {
        let mut t = TIMER.lock().unwrap();
        if t.phase != Phase::Focus || t.since.is_none() {
            return;
        }
        match hit {
            Some(app_name) if t.distracted_by.as_deref() != Some(app_name.as_str()) => {
                t.distracted_by = Some(app_name.clone());
                t.distractions += 1;
                t.nudge = Some(Nudge { app: app_name, at: now_ms() });
                true
            }
            Some(_) => false,
            None => {
                t.distracted_by = None;
                false
            }
        }
    };
    if nudged {
        emit(app);
    }
}

// ---------- timer thread ----------

/// One pass: end a finished phase, check the front window; returns how long to sleep.
fn tick(app: &AppHandle) -> Duration {
    let s = settings(app);
    if !s.modules.focus {
        let was_active = {
            let mut t = TIMER.lock().unwrap();
            let active = t.phase != Phase::Ready || t.digest_open;
            if active {
                *t = Timer::new();
            }
            active
        };
        if was_active {
            emit(app);
        }
        return Duration::from_secs(3600);
    }
    advance(app);
    let nudge_on = s.focus.nudge && !s.focus.nudge_apps.is_empty();
    let (running_focus, left) = {
        let t = TIMER.lock().unwrap();
        let now = now_ms();
        let left = (t.since.is_some() && t.length_ms > 0).then(|| t.length_ms.saturating_sub(t.elapsed(now)));
        (t.phase == Phase::Focus && t.since.is_some(), left)
    };
    if nudge_on && running_focus {
        check_nudge(app, &s.focus.nudge_apps);
    }
    let mut wait = Duration::from_secs(3600);
    if let Some(ms) = left {
        wait = wait.min(Duration::from_millis(ms + 20));
    }
    if nudge_on && running_focus {
        wait = wait.min(NUDGE_POLL);
    }
    wait
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("focus-timer".into())
        .spawn(move || loop {
            let wait = tick(&app);
            let mut woke = WAKE.lock().unwrap();
            if !*woke {
                woke = WAKE_CV.wait_timeout(woke, wait).unwrap().0;
            }
            *woke = false;
        })
        .expect("could not start the focus timer thread");
}

/// Settings changed: re-check the timer and the tray.
pub fn settings_changed(app: &AppHandle) {
    poke();
    emit(app);
}

// ---------- view, events, tray ----------

pub fn view(app: &AppHandle) -> FocusView {
    let s = settings(app);
    let now = now_ms();
    let t = TIMER.lock().unwrap();
    let length_ms = match t.phase {
        Phase::Ready => if s.focus.open_ended { 0 } else { s.focus.focus_min as u64 * MINUTE },
        _ => t.length_ms,
    };
    FocusView {
        phase: t.phase,
        running: t.since.is_some(),
        length_ms,
        elapsed_ms: t.elapsed(now),
        at: now,
        round: t.round.min(s.focus.long_every),
        rounds: s.focus.long_every,
        long_break: t.long_break,
        intention: t.intention.clone(),
        held: t.held.clone(),
        digest_open: t.digest_open,
        distractions: t.distractions,
        nudge: t.nudge.clone(),
        stats: stats(now),
    }
}

fn emit(app: &AppHandle) {
    let v = view(app);
    update_tray(app, &v);
    let _ = app.emit("focus", v);
}

/// The tray's focus item (created in lib.rs), kept in step with the timer.
pub fn set_tray_item(app: &AppHandle, item: MenuItem<Wry>) {
    let _ = TRAY_ITEM.set(item);
    update_tray(app, &view(app));
}

pub fn tray_text(v: &FocusView) -> &'static str {
    match (v.phase, v.running) {
        (Phase::Ready, _) => "Start focus",
        (Phase::Focus, true) => "Pause focus",
        (Phase::Break, true) => "Pause break",
        _ => "Resume focus",
    }
}

fn update_tray(app: &AppHandle, v: &FocusView) {
    if let Some(item) = TRAY_ITEM.get() {
        let on = app.state::<AppState>().settings.lock().unwrap().modules.focus;
        let _ = item.set_text(if on { tray_text(v) } else { "Focus timer (off in Settings)" });
        let _ = item.set_enabled(on);
    }
}

// ---------- history (this PC only) ----------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct Day {
    focus_ms: u64,
    sessions: u32,
    distractions: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct History {
    /// Local date ("2026-11-07") -> totals.
    days: BTreeMap<String, Day>,
}

static HISTORY: Mutex<Option<History>> = Mutex::new(None);

fn history_path() -> std::path::PathBuf {
    crate::paths::local_dir().join("focus").join("history.json")
}

fn with_history<R>(f: impl FnOnce(&mut History) -> R) -> R {
    let mut h = HISTORY.lock().unwrap();
    let h = h.get_or_insert_with(|| load_json(&history_path()));
    f(h)
}

/// Add a finished (or ended) focus block to today's totals.
fn record(focus_ms: u64, distractions: u32) {
    if focus_ms < MINUTE {
        return;
    }
    let today = today();
    with_history(|h| {
        let day = h.days.entry(date_key(today)).or_default();
        day.focus_ms += focus_ms;
        day.distractions += distractions;
        if focus_ms >= SESSION_MIN_MS {
            day.sessions += 1;
        }
        let oldest = date_key(today - KEEP_DAYS);
        h.days.retain(|k, _| *k >= oldest);
        if let Err(e) = save_json(&history_path(), h) {
            eprintln!("[atoll] could not save focus history: {e}");
        }
    });
}

pub fn clear_history() {
    with_history(|h| h.days.clear());
    let _ = std::fs::remove_file(history_path());
}

fn stats(now: u64) -> Stats {
    let today = day_number(now, local_offset_ms());
    with_history(|h| summarize(&h.days, today))
}

/// Today, streak and the last 7 days from the stored days (pure, for tests).
fn summarize(days: &BTreeMap<String, Day>, today: i64) -> Stats {
    let get = |d: i64| days.get(&date_key(d)).copied().unwrap_or_default();
    let t = get(today);
    // Today only breaks the streak once it is over.
    let mut d = if t.sessions > 0 { today } else { today - 1 };
    let mut streak = 0;
    while get(d).sessions > 0 {
        streak += 1;
        d -= 1;
    }
    let week = (today - 6..=today)
        .map(|d| DayBar { date: date_key(d), label: weekday(d).into(), focus_ms: get(d).focus_ms })
        .collect();
    Stats { today_ms: t.focus_ms, today_sessions: t.sessions, streak, week }
}

// ---------- local dates (no date crate needed) ----------

/// Local time minus UTC, from Windows' clock (follows daylight saving).
fn local_offset_ms() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let local = days_from_civil(st.wYear as i64, st.wMonth as u32, st.wDay as u32) * DAY_MS
        + st.wHour as i64 * 3_600_000
        + st.wMinute as i64 * 60_000
        + st.wSecond as i64 * 1000
        + st.wMilliseconds as i64;
    let offset = local - now_ms() as i64;
    // Real offsets are whole minutes; this drops the few ms between the two reads.
    (offset as f64 / 60_000.0).round() as i64 * 60_000
}

fn today() -> i64 {
    day_number(now_ms(), local_offset_ms())
}

fn day_number(ms: u64, offset: i64) -> i64 {
    (ms as i64 + offset).div_euclid(DAY_MS)
}

fn date_key(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn weekday(day: i64) -> &'static str {
    // 1970-01-01 was a Thursday.
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize]
}

/// Days since 1970-01-01 for a calendar date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let (m, d) = (m as i64, d as i64);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Focus;

    #[test]
    fn calendar_maths_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for (y, m, d) in [(2026, 11, 7), (2024, 2, 29), (2000, 3, 1), (1999, 12, 31), (2026, 1, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(weekday(days_from_civil(2026, 11, 7)), "Sat");
        assert_eq!(date_key(days_from_civil(2026, 3, 9)), "2026-03-09");
        // Local midnight decides the day: 23:30 UTC is already tomorrow at UTC+8.
        let late = (days_from_civil(2026, 11, 6) * DAY_MS + 23 * 3_600_000 + 30 * 60_000) as u64;
        assert_eq!(day_number(late, 0), days_from_civil(2026, 11, 6));
        assert_eq!(day_number(late, 8 * 3_600_000), days_from_civil(2026, 11, 7));
    }

    #[test]
    fn streak_waits_for_today_to_end() {
        let today = days_from_civil(2026, 11, 7);
        let mut days = BTreeMap::new();
        let day = |sessions| Day { focus_ms: sessions as u64 * 25 * MINUTE, sessions, distractions: 0 };
        days.insert(date_key(today - 1), day(2));
        days.insert(date_key(today - 2), day(1));
        days.insert(date_key(today - 4), day(3));
        let s = summarize(&days, today);
        assert_eq!((s.streak, s.today_sessions, s.today_ms), (2, 0, 0));
        assert_eq!(s.week.len(), 7);
        assert_eq!(s.week[6].label, "Sat");
        assert_eq!(s.week[5].focus_ms, 50 * MINUTE);
        days.insert(date_key(today), day(1));
        assert_eq!(summarize(&days, today).streak, 3);
    }

    #[test]
    fn alerts_follow_the_focus_rules() {
        let mut f = Focus::default();
        assert_eq!(rule_for(&f, Kind::Done), Decision::Hold);
        assert_eq!(rule_for(&f, Kind::ChatReply), Decision::Hold);
        assert_eq!(rule_for(&f, Kind::Error), Decision::Deliver);
        assert_eq!(rule_for(&f, Kind::Attention), Decision::Quiet);
        assert_eq!(rule_for(&f, Kind::Timer), Decision::Deliver);
        f.finished = "silent".into();
        f.errors = "hold".into();
        f.permissions = "show".into();
        assert_eq!(rule_for(&f, Kind::Usage), Decision::Drop);
        assert_eq!(rule_for(&f, Kind::Error), Decision::Hold);
        assert_eq!(rule_for(&f, Kind::Attention), Decision::Deliver);
        f.dnd = false;
        assert_eq!(rule_for(&f, Kind::Done), Decision::Deliver);
    }

    #[test]
    fn distractions_match_program_or_title() {
        let apps = vec!["YouTube".to_string(), "Discord".to_string()];
        assert_eq!(match_distraction(&apps, "chrome", "Lo-fi beats - YouTube - Google Chrome").as_deref(), Some("YouTube"));
        assert_eq!(match_distraction(&apps, "Discord", "#general").as_deref(), Some("Discord"));
        assert_eq!(match_distraction(&apps, "Code", "focus.rs - twocodeclone"), None);
        assert_eq!(match_distraction(&[" ".to_string()], "Code", "anything"), None);
    }

    #[test]
    fn rounds_cycle_through_the_long_break() {
        let f = Focus::default();
        let mut t = Timer::new();
        let mut after = After::default();
        for round in 1..=4 {
            assert_eq!(t.round, round);
            t.begin(Phase::Focus, f.focus_min as u64 * MINUTE, 0);
            focus_to_break(&mut t, &f, 25 * MINUTE, &mut after);
            assert_eq!(t.long_break, round == 4);
            assert_eq!(t.length_ms, if round == 4 { 15 } else { 5 } * MINUTE);
            t.next_round(f.long_every);
        }
        assert_eq!(t.round, 1);
        assert_eq!(t.phase, Phase::Ready);
    }
}
