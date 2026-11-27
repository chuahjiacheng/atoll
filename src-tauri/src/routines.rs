//! Routines: switch presets automatically, at a time of day or when screens
//! are plugged in or unplugged. Stored with the presets in presets.json.
//!
//! Routines react to *changes*, they don't hold you to a preset:
//! - a time routine fires once when its time comes (or, if the PC was asleep
//!   or Atoll was closed then, once on waking with the latest one missed);
//! - a screen routine fires when its condition becomes true (2 → 1 screens),
//!   after the count has been steady for a few seconds (Windows adds screens
//!   one at a time).
//! So switching by hand afterwards sticks until the next change.
//! Nothing switches to the preset already in use, and every automatic switch
//! can be undone.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::presets::{self, PresetFile};

pub const MAX_ROUTINES: usize = 30;
const TICK: Duration = Duration::from_secs(5);
/// Ticks a new screen count must hold before screen routines react.
const STEADY_TICKS: u8 = 2;
/// Time routines catch up at most this far back.
const CATCH_UP_MS: u64 = 7 * 24 * 3_600_000;
/// Undo stays offered this long after a switch.
pub const UNDO_MS: u64 = 12 * 3_600_000;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum When {
    /// `at` = "HH:MM" local time; `days` 0 = Sunday … 6 = Saturday.
    Time { at: String, days: Vec<u8> },
    /// Number of screens connected: exactly `count`, or `count` or more.
    Displays {
        count: u32,
        #[serde(rename = "orMore")]
        or_more: bool,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    #[serde(default)]
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub preset_id: String,
    pub when: When,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RoutineOptions {
    pub paused: bool,
    /// Windows notification when a routine switches.
    pub notify: bool,
}

impl Default for RoutineOptions {
    fn default() -> Self {
        Self { paused: false, notify: true }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RoutineState {
    /// Time routines up to here have been handled (UTC ms, 0 = never ran).
    pub last_check: u64,
    /// Screens connected when last looked (0 = unknown).
    pub last_displays: u32,
    pub last_switch: Option<LastSwitch>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LastSwitch {
    pub preset_id: String,
    pub routine_id: String,
    pub at: u64,
    pub reason: String,
    /// The preset in use before (restored by Undo).
    pub from: Option<String>,
    /// The full settings before the switch.
    pub undo: Value,
}

fn yes() -> bool {
    true
}

// ---------- pure logic (tested below) ----------

const MIN_MS: i64 = 60_000;
const DAY_MIN: i64 = 1440;

/// "HH:MM" → minutes after midnight.
pub fn parse_at(at: &str) -> Option<i64> {
    let (h, m) = at.split_once(':')?;
    let (h, m): (i64, i64) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// 0 = Sunday for a local day number (days since 1970-01-01, a Thursday).
fn weekday(day: i64) -> u8 {
    (day + 4).rem_euclid(7) as u8
}

pub fn displays_match(when: &When, n: u32) -> bool {
    match when {
        When::Displays { count, or_more } => n > 0 && if *or_more { n >= *count } else { n == *count },
        _ => false,
    }
}

/// The latest time-routine occurrence in (`from`, `to`] (UTC ms), local time
/// given by `offset` (local − UTC, ms). Returns the routine and when it was due.
pub fn due_time(routines: &[Routine], from: u64, to: u64, offset: i64) -> Option<(&Routine, u64)> {
    if to <= from {
        return None;
    }
    let from = from.max(to.saturating_sub(CATCH_UP_MS));
    let local_min = |utc: u64| (utc as i64 + offset).div_euclid(MIN_MS);
    let (lo, hi) = (local_min(from), local_min(to));
    let mut best: Option<(&Routine, i64)> = None;
    for r in routines.iter().filter(|r| r.enabled) {
        let When::Time { at, days } = &r.when else { continue };
        let Some(at) = parse_at(at) else { continue };
        for day in (hi.div_euclid(DAY_MIN) - 7)..=hi.div_euclid(DAY_MIN) {
            let m = day * DAY_MIN + at;
            if m > lo && m <= hi && days.contains(&weekday(day)) && best.is_none_or(|(_, b)| m > b) {
                best = Some((r, m));
            }
        }
    }
    best.map(|(r, m)| (r, (m * MIN_MS - offset).max(0) as u64))
}

/// The screen routine whose condition just became true (first in the list wins).
pub fn due_displays(routines: &[Routine], before: u32, now: u32) -> Option<&Routine> {
    if before == 0 || now == 0 || before == now {
        return None;
    }
    routines.iter().filter(|r| r.enabled).find(|r| displays_match(&r.when, now) && !displays_match(&r.when, before))
}

pub fn describe(when: &When) -> String {
    match when {
        When::Time { at, days } => {
            let mut d = days.clone();
            d.sort_unstable();
            d.dedup();
            let on = match d.as_slice() {
                [0, 1, 2, 3, 4, 5, 6] => "every day".to_string(),
                [1, 2, 3, 4, 5] => "on weekdays".to_string(),
                [0, 6] => "at weekends".to_string(),
                _ => {
                    const N: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                    format!("on {}", d.iter().map(|x| N[*x as usize % 7]).collect::<Vec<_>>().join(", "))
                }
            };
            format!("At {at} {on}")
        }
        When::Displays { count: 1, or_more: false } => "When only one screen is connected".into(),
        When::Displays { count, or_more } => format!("When {count}{} screens are connected", if *or_more { " or more" } else { "" }),
    }
}

/// Checks a routine from the dashboard; returns it tidied.
pub fn validate(mut r: Routine, f: &PresetFile) -> Result<Routine, String> {
    if !f.presets.iter().any(|p| p.id == r.preset_id) {
        return Err("Choose a preset for this routine.".into());
    }
    match &mut r.when {
        When::Time { at, days } => {
            let m = parse_at(at).ok_or("Choose a time.")?;
            *at = format!("{:02}:{:02}", m / 60, m % 60);
            days.retain(|d| *d < 7);
            days.sort_unstable();
            days.dedup();
            if days.is_empty() {
                return Err("Pick at least one day.".into());
            }
        }
        When::Displays { count, .. } => {
            if !(1..=6).contains(count) {
                return Err("Choose between 1 and 6 screens.".into());
            }
        }
    }
    Ok(r)
}

/// Screen routines that could both fire on the same change, by id.
pub fn overlaps(routines: &[Routine]) -> Vec<String> {
    let screens: Vec<&Routine> = routines.iter().filter(|r| r.enabled && matches!(r.when, When::Displays { .. })).collect();
    let mut out = vec![];
    for (i, a) in screens.iter().enumerate() {
        for b in &screens[i + 1..] {
            if a.preset_id != b.preset_id && (1..=8).any(|n| displays_match(&a.when, n) && displays_match(&b.when, n)) {
                out.push(a.id.clone());
                out.push(b.id.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Drop routines whose preset is gone, and a stale undo.
pub fn clean(f: &mut PresetFile) {
    let ids: Vec<String> = f.presets.iter().map(|p| p.id.clone()).collect();
    f.routines.retain(|r| ids.contains(&r.preset_id) && !r.id.is_empty());
    if f.routine_state.last_switch.as_ref().is_some_and(|s| !ids.contains(&s.preset_id)) {
        f.routine_state.last_switch = None;
    }
}

pub struct Decision {
    pub routine_id: String,
    pub preset_id: String,
    pub reason: String,
}

/// What one tick should do. Screen changes win over a time routine in the same tick.
pub fn decide(f: &PresetFile, now: u64, offset: i64, displays_now: u32) -> Option<Decision> {
    if f.routine_options.paused || f.routine_state.last_check == 0 {
        return None;
    }
    let pick = due_displays(&f.routines, f.routine_state.last_displays, displays_now)
        .or_else(|| due_time(&f.routines, f.routine_state.last_check, now, offset).map(|(r, _)| r))?;
    // Already on it: nothing to switch.
    if f.active.as_deref() == Some(pick.preset_id.as_str()) {
        return None;
    }
    Some(Decision { routine_id: pick.id.clone(), preset_id: pick.preset_id.clone(), reason: describe(&pick.when) })
}

// ---------- the clock ----------

/// Local time minus UTC (ms), from Windows (follows daylight saving).
fn local_offset_ms() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    // Days from civil (Howard Hinnant).
    let (y, m, d) = (st.wYear as i64, st.wMonth as i64, st.wDay as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let local = days * 86_400_000 + st.wHour as i64 * 3_600_000 + st.wMinute as i64 * 60_000 + st.wSecond as i64 * 1000;
    let offset = local - presets::now_ms() as i64;
    (offset as f64 / 60_000.0).round() as i64 * 60_000
}

fn screen_count(app: &AppHandle) -> u32 {
    app.available_monitors().map(|m| m.len() as u32).unwrap_or(0)
}

/// Last count seen and how many ticks it has held.
static SEEN: Mutex<(u32, u8)> = Mutex::new((0, 0));

pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        tick(&app);
        std::thread::sleep(TICK);
    });
}

fn tick(app: &AppHandle) {
    let n = screen_count(app);
    let steady = {
        let mut seen = SEEN.lock().unwrap();
        if seen.0 == n {
            seen.1 = seen.1.saturating_add(1);
        } else {
            *seen = (n, 1);
        }
        (seen.1 >= STEADY_TICKS).then_some(n)
    };
    let now = presets::now_ms();
    let offset = local_offset_ms();
    let current = presets::current(app);

    let switched = {
        let _guard = presets::FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let p = presets::path();
        let mut f = presets::load_from(&p);
        let before = f.clone();
        f.routine_state.last_check = effective_last_check(f.routine_state.last_check);
        // Without a steady count, screen routines wait; time routines don't.
        let displays_now = steady.unwrap_or(f.routine_state.last_displays);
        let decision = decide(&f, now, offset, displays_now);
        let mut next = None;
        if let Some(d) = &decision {
            if let Some(preset) = f.presets.iter().find(|p| p.id == d.preset_id) {
                match presets::apply_to(&current, &preset.settings) {
                    Ok(s) => {
                        f.routine_state.last_switch = Some(LastSwitch {
                            preset_id: d.preset_id.clone(),
                            routine_id: d.routine_id.clone(),
                            at: now,
                            reason: d.reason.clone(),
                            from: f.active.clone(),
                            undo: serde_json::to_value(&current).unwrap_or(Value::Null),
                        });
                        f.active = Some(d.preset_id.clone());
                        next = Some((s, preset.name.clone(), d.reason.clone()));
                    }
                    Err(e) => eprintln!("[atoll] routine could not apply preset: {e}"),
                }
            }
        }
        f.routine_state.last_check = now;
        if steady.is_some() {
            f.routine_state.last_displays = displays_now;
        }
        // Write when something changed, else every few minutes (keeps catch-up
        // honest after a restart). No presets yet: nothing to write.
        let first = before.routine_state.last_check == 0;
        let stale = now.saturating_sub(before.routine_state.last_check) > 5 * 60_000;
        let changed = next.is_some() || before.routine_state.last_displays != f.routine_state.last_displays;
        if !f.presets.is_empty() && (first || stale || changed) {
            if let Err(e) = presets::save_to(&p, &f) {
                eprintln!("[atoll] {e}");
            }
        }
        remember_check(now);
        next.map(|n| (n, f.routine_options.notify))
    };

    if let Some(((settings, name, reason), notify)) = switched {
        crate::commit_settings(app, settings);
        let _ = app.emit("presets-changed", ());
        if notify {
            crate::alerts::toast(&format!("Switched to \u{201c}{name}\u{201d}"), &format!("{reason}. Undo in Settings \u{2192} Presets."));
        }
    }
}

/// Between file writes, the tick's progress is kept here so a rule never fires twice.
static LAST_CHECK: Mutex<u64> = Mutex::new(0);

fn remember_check(now: u64) {
    *LAST_CHECK.lock().unwrap() = now;
}

/// The file's last_check, moved forward by checks not yet written.
pub(crate) fn effective_last_check(stored: u64) -> u64 {
    if stored == 0 {
        0
    } else {
        stored.max(*LAST_CHECK.lock().unwrap())
    }
}

// ---------- commands ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    #[serde(flatten)]
    pub routine: Routine,
    pub label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchView {
    pub preset_id: String,
    pub at: u64,
    pub reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineList {
    pub options: RoutineOptions,
    pub routines: Vec<RoutineView>,
    /// Ids of screen routines that could fire on the same change.
    pub overlaps: Vec<String>,
    pub last_switch: Option<SwitchView>,
    pub displays: u32,
}

fn view(f: &PresetFile, app: &AppHandle) -> RoutineList {
    let now = presets::now_ms();
    RoutineList {
        options: f.routine_options.clone(),
        routines: f.routines.iter().map(|r| RoutineView { label: describe(&r.when), routine: r.clone() }).collect(),
        overlaps: overlaps(&f.routines),
        last_switch: f
            .routine_state
            .last_switch
            .as_ref()
            .filter(|s| now.saturating_sub(s.at) < UNDO_MS)
            .map(|s| SwitchView { preset_id: s.preset_id.clone(), at: s.at, reason: s.reason.clone() }),
        displays: screen_count(app),
    }
}

#[tauri::command]
pub fn list_routines(app: AppHandle) -> RoutineList {
    let f = {
        let _guard = presets::FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        presets::load_from(&presets::path())
    };
    view(&f, &app)
}

/// Add (empty id) or change a routine.
#[tauri::command]
pub fn save_routine(app: AppHandle, routine: Routine) -> Result<RoutineList, String> {
    let f = presets::with_file(|f| {
        let mut r = validate(routine, f)?;
        // A new or changed routine starts from now: no catching up on its past.
        f.routine_state.last_check = effective_last_check(f.routine_state.last_check).max(presets::now_ms());
        remember_check(f.routine_state.last_check);
        match f.routines.iter_mut().find(|x| !r.id.is_empty() && x.id == r.id) {
            Some(x) => *x = r,
            None => {
                if f.routines.len() >= MAX_ROUTINES {
                    return Err(format!("You can have up to {MAX_ROUTINES} routines."));
                }
                r.id = presets::new_id();
                f.routines.push(r);
            }
        }
        Ok(f.clone())
    })?;
    Ok(view(&f, &app))
}

#[tauri::command]
pub fn delete_routine(app: AppHandle, id: String) -> Result<RoutineList, String> {
    let f = presets::with_file(|f| {
        f.routines.retain(|r| r.id != id);
        Ok(f.clone())
    })?;
    Ok(view(&f, &app))
}

#[tauri::command]
pub fn set_routine_options(app: AppHandle, paused: Option<bool>, notify: Option<bool>) -> Result<RoutineList, String> {
    let f = presets::with_file(|f| {
        if let Some(p) = paused {
            f.routine_options.paused = p;
        }
        if let Some(n) = notify {
            f.routine_options.notify = n;
        }
        Ok(f.clone())
    })?;
    Ok(view(&f, &app))
}

/// Put back the settings from before the last automatic switch.
#[tauri::command]
pub fn undo_routine_switch(app: AppHandle) -> Result<RoutineList, String> {
    let current = presets::current(&app);
    let (f, next) = presets::with_file(|f| {
        let last = f.routine_state.last_switch.take().ok_or("There is nothing to undo.")?;
        let next = presets::apply_to(&current, &last.undo)?;
        f.active = last.from.filter(|id| f.presets.iter().any(|p| &p.id == id));
        Ok((f.clone(), next))
    })?;
    crate::commit_settings(&app, next);
    Ok(view(&f, &app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::Preset;
    use serde_json::json;

    const H: u64 = 3_600_000;
    /// 2026-11-23 00:00 UTC, a Monday.
    const MON: u64 = 1_795_392_000_000;

    fn time(id: &str, preset: &str, at: &str, days: &[u8]) -> Routine {
        Routine { id: id.into(), enabled: true, preset_id: preset.into(), when: When::Time { at: at.into(), days: days.to_vec() } }
    }
    fn screens(id: &str, preset: &str, count: u32, or_more: bool) -> Routine {
        Routine { id: id.into(), enabled: true, preset_id: preset.into(), when: When::Displays { count, or_more } }
    }
    fn file(routines: Vec<Routine>) -> PresetFile {
        let p = |id: &str| Preset { id: id.into(), name: id.into(), saved_at: 1, settings: json!({}) };
        PresetFile {
            presets: vec![p("work"), p("home"), p("travel")],
            routines,
            routine_state: RoutineState { last_check: MON, last_displays: 2, last_switch: None },
            ..Default::default()
        }
    }

    #[test]
    fn monday_is_monday() {
        assert_eq!(weekday((MON / (24 * H)) as i64), 1);
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_at("09:30"), Some(570));
        assert_eq!(parse_at("9:05"), Some(545));
        assert_eq!(parse_at("24:00"), None);
        assert_eq!(parse_at("12:60"), None);
        assert_eq!(parse_at("noon"), None);
    }

    #[test]
    fn time_routine_fires_once_in_its_window() {
        let r = vec![time("a", "work", "09:00", &[1])];
        assert!(due_time(&r, MON + 8 * H, MON + 9 * H - 1, 0).is_none());
        let (hit, at) = due_time(&r, MON + 8 * H, MON + 9 * H, 0).unwrap();
        assert_eq!((hit.id.as_str(), at), ("a", MON + 9 * H));
        // The next tick's window starts after it: no second fire.
        assert!(due_time(&r, MON + 9 * H, MON + 9 * H + 5000, 0).is_none());
        // Wrong day.
        assert!(due_time(&vec![time("a", "work", "09:00", &[2])], MON + 8 * H, MON + 10 * H, 0).is_none());
    }

    #[test]
    fn local_time_zone_is_used() {
        // UTC+8: 09:00 local = 01:00 UTC.
        let r = vec![time("a", "work", "09:00", &[1])];
        let (_, at) = due_time(&r, MON, MON + 2 * H, 8 * H as i64).unwrap();
        assert_eq!(at, MON + H);
    }

    #[test]
    fn waking_up_applies_only_the_latest_missed_routine() {
        let r = vec![time("a", "work", "09:00", &[1]), time("b", "home", "18:00", &[1]), time("c", "travel", "23:00", &[1])];
        // Asleep from 08:00 to 20:00: 09:00 and 18:00 were missed, 18:00 wins.
        let (hit, _) = due_time(&r, MON + 8 * H, MON + 20 * H, 0).unwrap();
        assert_eq!(hit.id, "b");
    }

    #[test]
    fn disabled_routines_never_fire() {
        let mut r = time("a", "work", "09:00", &[1]);
        r.enabled = false;
        assert!(due_time(&[r], MON, MON + 10 * H, 0).is_none());
    }

    #[test]
    fn screen_routines_fire_on_the_change_only() {
        let r = vec![screens("one", "travel", 1, false), screens("multi", "work", 2, true)];
        assert_eq!(due_displays(&r, 2, 1).unwrap().id, "one");
        assert_eq!(due_displays(&r, 1, 2).unwrap().id, "multi");
        // 2 → 3 is still "2 or more": nothing new.
        assert!(due_displays(&r, 2, 3).is_none());
        assert!(due_displays(&r, 2, 2).is_none());
        // Unknown or no screens (asleep, lid shut): wait.
        assert!(due_displays(&r, 0, 1).is_none());
        assert!(due_displays(&r, 2, 0).is_none());
    }

    #[test]
    fn decide_prefers_screens_and_skips_the_preset_in_use() {
        let mut f = file(vec![time("t", "home", "09:00", &[1]), screens("one", "travel", 1, false)]);
        f.routine_state.last_check = MON + 8 * H;
        // Both due in the same tick: the screen change wins.
        assert_eq!(decide(&f, MON + 9 * H, 0, 1).unwrap().preset_id, "travel");
        // Only the time.
        assert_eq!(decide(&f, MON + 9 * H, 0, 2).unwrap().preset_id, "home");
        // Already on it.
        f.active = Some("home".into());
        assert!(decide(&f, MON + 9 * H, 0, 2).is_none());
        // Paused.
        f.active = None;
        f.routine_options.paused = true;
        assert!(decide(&f, MON + 9 * H, 0, 1).is_none());
    }

    #[test]
    fn first_run_never_catches_up() {
        let mut f = file(vec![time("t", "home", "09:00", &[1])]);
        f.routine_state.last_check = 0;
        assert!(decide(&f, MON + 10 * H, 0, 2).is_none());
    }

    #[test]
    fn catch_up_is_limited_to_a_week() {
        let r = vec![time("a", "work", "09:00", &[1])];
        // Last ran a month ago: still finds this Monday's 09:00.
        let (_, at) = due_time(&r, MON - 30 * 24 * H, MON + 10 * H, 0).unwrap();
        assert_eq!(at, MON + 9 * H);
    }

    #[test]
    fn validation_tidies_and_rejects() {
        let f = file(vec![]);
        let r = validate(time("", "work", "7:5", &[5, 1, 1, 9]), &f).unwrap();
        assert_eq!(r.when, When::Time { at: "07:05".into(), days: vec![1, 5] });
        assert!(validate(time("", "nope", "07:00", &[1]), &f).is_err());
        assert!(validate(time("", "work", "07:00", &[]), &f).is_err());
        assert!(validate(time("", "work", "25:00", &[1]), &f).is_err());
        assert!(validate(screens("", "work", 0, false), &f).is_err());
    }

    #[test]
    fn labels_read_well() {
        assert_eq!(describe(&When::Time { at: "09:00".into(), days: vec![1, 2, 3, 4, 5] }), "At 09:00 on weekdays");
        assert_eq!(describe(&When::Time { at: "22:30".into(), days: (0..7).collect() }), "At 22:30 every day");
        assert_eq!(describe(&When::Time { at: "10:00".into(), days: vec![2, 4] }), "At 10:00 on Tue, Thu");
        assert_eq!(describe(&When::Displays { count: 1, or_more: false }), "When only one screen is connected");
        assert_eq!(describe(&When::Displays { count: 2, or_more: true }), "When 2 or more screens are connected");
    }

    #[test]
    fn overlapping_screen_routines_are_flagged() {
        let r = vec![screens("a", "work", 2, true), screens("b", "home", 3, false), screens("c", "travel", 1, false)];
        assert_eq!(overlaps(&r), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn deleting_a_preset_drops_its_routines() {
        let mut f = file(vec![time("t", "home", "09:00", &[1]), screens("s", "work", 1, false)]);
        f.presets.retain(|p| p.id != "home");
        clean(&mut f);
        assert_eq!(f.routines.len(), 1);
        assert_eq!(f.routines[0].id, "s");
    }

    #[test]
    fn routines_round_trip_through_json() {
        let f = file(vec![time("t", "home", "09:00", &[1, 2]), screens("s", "work", 2, true)]);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["routines"][1]["when"], json!({ "kind": "displays", "count": 2, "orMore": true }));
        assert_eq!(v["routines"][0]["when"]["kind"], "time");
        let back: PresetFile = serde_json::from_value(v).unwrap();
        assert_eq!(back, f);
        // An older presets.json without routines still loads.
        let old: PresetFile = serde_json::from_value(json!({ "active": null, "presets": [] })).unwrap();
        assert!(old.routines.is_empty() && old.routine_options.notify);
    }
}
