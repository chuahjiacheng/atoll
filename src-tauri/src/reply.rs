//! Answer Claude from the island without leaving the app you're in.
//!
//! The normal route never shows Claude: Windows UI Automation (reply_uia.ps1)
//! presses the Claude app's own reply button for that choice, the same as a
//! click in its "next step" box; for an answer you typed it fills the message
//! box and presses Send. Claude must be awake for that (a minimised Claude
//! stops updating its page), and it pulls itself to the front when pressed, so
//! `send_quietly` wakes it behind your app, has it moved off-screen for the
//! press, hands the keyboard straight back to you and puts Claude back.
//!
//! Only if that fails does the old route run: bring Claude to the front, type
//! the reply as Unicode key events and press Enter, then switch you back.
//! Windows only lets the foreground app hand the foreground to another window,
//! so we first tap F24 (not Alt, which Electron reads as a menu-bar press and
//! eats the first letters).

use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_F24, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, GW_OWNER, HWND_BOTTOM, SWP_NOACTIVATE, SW_MINIMIZE,
    GetWindowPlacement, SetWindowPlacement, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED, SW_SHOWMAXIMIZED, GetLayeredWindowAttributes,
    SetLayeredWindowAttributes, LWA_ALPHA, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE,
};

/// The app you were last in that is neither Crest nor Claude.
static YOUR_APP: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Keep [`YOUR_APP`] up to date (checked five times a second), so a reply sent
/// from the island can hand the keyboard back to where you were. Call once.
pub fn track_your_app() {
    std::thread::spawn(|| loop {
        let fg = unsafe { GetForegroundWindow() };
        if !fg.is_null() {
            // process_name is None for Crest's own windows.
            if let Some(name) = crate::chatwatch::process_name(fg) {
                if name != "claude.exe" {
                    YOUR_APP.store(fg as isize, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    });
}

/// Where to hand the keyboard back to after a reply: the app you were last in.
fn your_app() -> HWND {
    let tracked = YOUR_APP.load(std::sync::atomic::Ordering::Relaxed) as HWND;
    if !tracked.is_null() && unsafe { IsWindow(tracked) } != 0 {
        return tracked;
    }
    crate::paste::previous()
}

/// Turn the minimise/restore animation off (or back on) for `hwnd`.
unsafe fn set_transitions_off(hwnd: HWND, off: bool) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
    let v = windows::core::BOOL(off as i32);
    let _ = DwmSetWindowAttribute(
        windows::Win32::Foundation::HWND(hwnd),
        DWMWA_TRANSITIONS_FORCEDISABLED,
        (&v as *const windows::core::BOOL).cast(),
        std::mem::size_of::<windows::core::BOOL>() as u32,
    );
}

/// Claude's window, fully transparent until dropped (then its opacity is put
/// back as it was).
struct Invisible {
    hwnd: HWND,
    alpha: u8,
}

impl Invisible {
    fn new(hwnd: HWND) -> Invisible {
        let (mut key, mut alpha, mut how) = (0u32, 255u8, 0);
        unsafe {
            if GetLayeredWindowAttributes(hwnd, &mut key, &mut alpha, &mut how) == 0 || how & LWA_ALPHA == 0 {
                alpha = 255;
            }
            SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        }
        Invisible { hwnd, alpha }
    }
}

impl Drop for Invisible {
    fn drop(&mut self) {
        unsafe { SetLayeredWindowAttributes(self.hwnd, 0, self.alpha, LWA_ALPHA) };
    }
}

/// Run the background send `f` without Claude ever staying in front.
///
/// - A minimised Claude stops updating its page, so its reply buttons never
///   appear: un-minimise it *behind* the app you're in (no focus, no
///   animation), and minimise it again afterwards. An open-but-covered Claude
///   keeps updating and is left where it is.
/// - Pressing Claude's button makes Claude pull its own window to the front
///   (Chromium forces it; Windows' foreground lock doesn't stop it). A watcher
///   checks every 5 ms while the press is handled and, the moment Claude is in
///   front, gives focus straight back to `you` and puts Claude back.
///
/// If `f` fails the watcher stops at once, so the typing fallback can bring
/// Claude forward.
fn send_quietly(hwnd: HWND, you: HWND, f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;
    let was_min = unsafe { IsIconic(hwnd) } != 0;
    let you = if you == hwnd || (!you.is_null() && unsafe { IsWindow(you) } == 0) { std::ptr::null_mut() } else { you };
    let behind = if you.is_null() { HWND_BOTTOM } else { you };
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    // Make Claude fully transparent for the whole hand-off: un-minimising
    // draws it on top of your app for a frame, and the press makes it jump to
    // the front, and neither should be seen. Claude's window is already a
    // layered window, so this only changes its opacity; the guard puts the
    // opacity back when this function ends, however it ends.
    let _invisible = Invisible::new(hwnd);
    unsafe { set_transitions_off(hwnd, true) };
    if was_min {
        unsafe {
            SetWindowPos(hwnd, behind, 0, 0, 0, 0, flags);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(hwnd, behind, 0, 0, 0, 0, flags);
        }
    }
    // Where Claude sits now: the script moves it off-screen just before the
    // press (so its jump to the front shows nothing), and we put it back.
    let mut home: RECT = unsafe { std::mem::zeroed() };
    let mut home_place: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    home_place.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    unsafe {
        GetWindowRect(hwnd, &mut home);
        GetWindowPlacement(hwnd, &mut home_place);
    }
    let put_back = move || unsafe {
        if IsIconic(hwnd) != 0 {
            // Minimised while away: fix the spot it will restore to.
            let mut wp = home_place;
            if home_place.showCmd as i32 == SW_SHOWMAXIMIZED as i32 {
                wp.flags |= WPF_RESTORETOMAXIMIZED;
            }
            wp.showCmd = SW_SHOWMINNOACTIVE as _;
            SetWindowPlacement(hwnd, &wp);
        } else {
            SetWindowPos(hwnd, behind, home.left, home.top, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        }
    };

    // 0 = sending, 1 = sent (keep watching a little longer), 2 = stop now.
    let phase = Arc::new(AtomicU8::new(0));
    let watcher = {
        let phase = phase.clone();
        let (claude, you, behind) = (hwnd as isize, you as isize, behind as isize);
        std::thread::spawn(move || unsafe {
            let (claude, you, behind) = (claude as HWND, you as HWND, behind as HWND);
            let mut sent_at: Option<std::time::Instant> = None;
            loop {
                match phase.load(Ordering::Relaxed) {
                    2 => break,
                    1 => {
                        let t = *sent_at.get_or_insert_with(std::time::Instant::now);
                        if t.elapsed() > Duration::from_millis(2500) {
                            break;
                        }
                    }
                    _ => {}
                }
                if GetForegroundWindow() == claude {
                    if !you.is_null() {
                        focus(you);
                        // It may be off-screen: just push it behind you; it
                        // is moved back (and minimised) once the send is done.
                        SetWindowPos(claude, behind, 0, 0, 0, 0, flags);
                    } else {
                        // Nowhere to send you: minimise it the normal way so
                        // Windows moves the keyboard on.
                        ShowWindow(claude, SW_MINIMIZE);
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };

    let out = f();
    if out.is_err() {
        phase.store(2, Ordering::Relaxed);
        let _ = watcher.join();
        put_back();
        unsafe { set_transitions_off(hwnd, false) };
        return out;
    }
    // Give Claude a moment to act on the press (and jump, and be sent back)
    // before it goes home and pauses again.
    std::thread::sleep(Duration::from_millis(700));
    phase.store(1, Ordering::Relaxed);
    let _ = watcher.join();
    unsafe {
        if GetForegroundWindow() == hwnd && !you.is_null() {
            focus(you);
        }
    }
    put_back();
    if was_min && unsafe { IsIconic(hwnd) } == 0 && unsafe { GetForegroundWindow() } != hwnd {
        unsafe { ShowWindow(hwnd, SW_SHOWMINNOACTIVE) };
    }
    unsafe { set_transitions_off(hwnd, false) };
    out
}

/// The Claude app's main window: visible, unowned, titled, the largest one.
fn claude_window() -> Option<HWND> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> i32 {
        let best = &mut *(data as *mut (isize, i64));
        if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() || GetWindowTextLengthW(hwnd) == 0 {
            return 1;
        }
        if crate::chatwatch::process_name(hwnd).as_deref() != Some("claude.exe") {
            return 1;
        }
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let area = i64::from(r.right - r.left) * i64::from(r.bottom - r.top);
        // A minimised window reports a tiny rect; still a candidate.
        let area = if IsIconic(hwnd) != 0 { 1 } else { area };
        if area > best.1 {
            *best = (hwnd as isize, area);
        }
        1
    }
    let mut best: (isize, i64) = (0, 0);
    unsafe { EnumWindows(Some(each), &mut best as *mut _ as LPARAM) };
    (best.0 != 0).then_some(best.0 as HWND)
}

fn vk(code: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: code, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn unicode(unit: u16, up: bool) -> INPUT {
    let flags = KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}


/// Bring `hwnd` to the front. Any input event unlocks SetForegroundWindow; F24
/// is a key nothing listens to (tapping Alt would light up Electron's menu bar
/// and swallow the first letters we type).
unsafe fn focus(hwnd: HWND) {
    send(&[vk(VK_F24, false), vk(VK_F24, true)]);
    let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
    let me = GetCurrentThreadId();
    let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
    SetForegroundWindow(hwnd);
    if attached {
        AttachThreadInput(me, fg_thread, 0);
    }
}

/// The Claude app's data folders: the regular one and the Store (MSIX) build's
/// private copy of AppData.
fn claude_app_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(a) = env("APPDATA") {
        roots.push(a.join("Claude"));
    }
    if let Some(l) = env("LOCALAPPDATA") {
        if let Ok(rd) = std::fs::read_dir(l.join("Packages")) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("claude") {
                    roots.push(e.path().join("LocalCache").join("Roaming").join("Claude"));
                }
            }
        }
    }
    roots
}

/// The sidebar title of a Code-tab session, from its record in the Claude app
/// (`claude-code-sessions\<account>\<org>\local_<id>.json`), when the record
/// belongs to Claude Code session `cli_id`.
fn title_in_record(json: &str, cli_id: &str) -> Option<String> {
    // Cheap check first: most records are for other sessions.
    if !json.contains(cli_id) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let is = |k: &str| v.get(k).and_then(serde_json::Value::as_str) == Some(cli_id);
    if !is("cliSessionId") && !is("lastAssistantCliSessionId") {
        return None;
    }
    let title = v.get("title").and_then(serde_json::Value::as_str)?.trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// The title the Claude app shows for Claude Code session `cli_id`, if it is a
/// Code-tab session. The newest matching record wins (the Store build can hold
/// a stale copy next to the live one).
pub fn desktop_session_title(cli_id: &str) -> Option<String> {
    if cli_id.is_empty() {
        return None;
    }
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for root in claude_app_dirs() {
        let Ok(accounts) = std::fs::read_dir(root.join("claude-code-sessions")) else { continue };
        for account in accounts.flatten() {
            let Ok(orgs) = std::fs::read_dir(account.path()) else { continue };
            for org in orgs.flatten() {
                let Ok(files) = std::fs::read_dir(org.path()) else { continue };
                for f in files.flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    if !name.starts_with("local_") || !name.ends_with(".json") {
                        continue;
                    }
                    let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
                    let Some(title) = title_in_record(&text, cli_id) else { continue };
                    let at = f.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                    if best.as_ref().is_none_or(|(old, _)| at > *old) {
                        best = Some((at, title));
                    }
                }
            }
        }
    }
    best.map(|(_, t)| t)
}

/// Send `text` to the Claude app without focusing it: UI Automation fills its
/// message box and presses Send, so you stay in whatever app you are using.
/// `session`: the title of the Code-tab session the reply is for; the script
/// opens that session first (the app may be showing another one) and puts the
/// one you were looking at back afterwards.
fn send_in_background(hwnd: HWND, text: &str, is_choice: bool, session: Option<&str>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", include_str!("reply_uia.ps1")])
        .env("CREST_HWND", (hwnd as isize).to_string())
        .env("CREST_TEXT", text)
        .env("CREST_CHOICE", if is_choice { "1" } else { "0" })
        .env("CREST_SESSION", session.unwrap_or(""))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    // Read while it runs: a long failure message could otherwise fill the pipe
    // and stall the script until the timeout.
    let reader = child.stdout.take().map(|mut so| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut bytes = Vec::new();
            let _ = so.read_to_end(&mut bytes);
            bytes
        })
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(45) => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                return Err("timed out".into());
            }
        }
    }
    let bytes = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    let out = String::from_utf8_lossy(&bytes);
    let out = out.trim();
    if out == "ok" {
        Ok(())
    } else {
        Err(out.to_string())
    }
}

/// Send `text` to the Claude app as your reply. Errors when the app isn't open
/// or the fallback can't bring it to the front (then nothing is typed anywhere
/// else). `is_choice`: `text` is one of the question's reply choices, so Claude
/// shows a button for it (possibly a few seconds late) that we can press.
/// `session`: the Claude app's title for the session that asked. With several
/// sessions open, the app shows only one of them; the reply must go to the
/// session that asked, not the one on screen.
pub fn send_to_claude(text: &str, is_choice: bool, session: Option<&str>) -> Result<(), String> {
    let hwnd = claude_window().ok_or("The Claude app isn't open. Open it and pick again.")?;
    // The app you were in before you reached for the island (clicking the
    // island makes Crest the foreground app).
    let you = your_app();
    let quiet = send_quietly(hwnd, you, || send_in_background(hwnd, text, is_choice, session));
    if let Err(e) = &quiet {
        // The asking session couldn't be opened: the app still shows another
        // session, so typing now would answer the wrong one.
        if e.starts_with("fail:session") {
            let name = session.unwrap_or("that session");
            return Err(format!("Couldn't open \u{201c}{name}\u{201d} in the Claude app. Open it there and pick again."));
        }
    }
    if quiet.is_ok() {
        // Clicking the pill left the keyboard on the island: give it back.
        unsafe {
            let front = GetForegroundWindow();
            if !you.is_null() && IsWindow(you) != 0 && IsIconic(you) == 0 && crate::chatwatch::process_name(front).is_none() {
                focus(you);
            }
        }
        return Ok(());
    }
    // Fallback: bring Claude forward, type, press Enter, switch you back.
    let raw = hwnd as isize;
    let prev_raw = you as isize;
    let text = text.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        let hwnd = raw as HWND;
        let prev = prev_raw as HWND;
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        focus(hwnd);
        // Let the app take focus (and put the cursor back in its message box).
        let mut ok = false;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(25));
            if GetForegroundWindow() == hwnd {
                ok = true;
                break;
            }
        }
        if !ok {
            let _ = tx.send(Err("Couldn't bring the Claude app to the front. Click it, then pick again.".to_string()));
            return;
        }
        // Foreground is not the same as "the message box has keyboard focus":
        // Electron needs a moment to focus its page. Typing early drops the
        // first characters.
        std::thread::sleep(Duration::from_millis(400));
        // One character at a time with a short gap, so a busy renderer can't
        // drop part of a burst.
        for unit in text.encode_utf16() {
            send(&[unicode(unit, false), unicode(unit, true)]);
            std::thread::sleep(Duration::from_millis(6));
        }
        std::thread::sleep(Duration::from_millis(120));
        send(&[vk(VK_RETURN, false), vk(VK_RETURN, true)]);
        let _ = tx.send(Ok(()));
        // Hand the screen back to whatever you were using. Verify it took and
        // retry, since Windows sometimes refuses the first switch.
        std::thread::sleep(Duration::from_millis(250));
        if !prev.is_null() && prev != hwnd && IsWindow(prev) != 0 {
            for _ in 0..8 {
                if IsIconic(prev) != 0 {
                    break;
                }
                focus(prev);
                std::thread::sleep(Duration::from_millis(60));
                if GetForegroundWindow() == prev {
                    break;
                }
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_comes_from_the_matching_record() {
        let rec = r#"{"sessionId":"local_1","cliSessionId":"aaa-111","title":"Fix the pill","isArchived":false}"#;
        assert_eq!(title_in_record(rec, "aaa-111").as_deref(), Some("Fix the pill"));
    }

    #[test]
    fn other_sessions_records_are_ignored() {
        let rec = r#"{"cliSessionId":"bbb-222","title":"Other","lastAssistantCliSessionId":"bbb-222"}"#;
        assert_eq!(title_in_record(rec, "aaa-111"), None);
        // The id appearing somewhere else in the record is not a match.
        let rec = r#"{"cliSessionId":"bbb-222","title":"Other","note":"aaa-111"}"#;
        assert_eq!(title_in_record(rec, "aaa-111"), None);
    }

    #[test]
    fn a_resumed_session_matches_on_its_latest_cli_id() {
        let rec = r#"{"cliSessionId":"old-0","lastAssistantCliSessionId":"aaa-111","title":"Resumed"}"#;
        assert_eq!(title_in_record(rec, "aaa-111").as_deref(), Some("Resumed"));
    }

    #[test]
    fn blank_or_broken_records_give_no_title() {
        assert_eq!(title_in_record(r#"{"cliSessionId":"aaa-111","title":"  "}"#, "aaa-111"), None);
        assert_eq!(title_in_record(r#"{"cliSessionId":"aaa-111""#, "aaa-111"), None);
        assert_eq!(desktop_session_title(""), None);
    }

    /// Reads the real Claude app data: this test session's own title.
    #[test]
    #[ignore]
    fn finds_a_real_session_title() {
        let id = std::env::var("CREST_TEST_CLI_ID").expect("set CREST_TEST_CLI_ID");
        println!("{:?}", desktop_session_title(&id));
        assert!(desktop_session_title(&id).is_some());
    }
}
