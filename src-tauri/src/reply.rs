//! Answer Claude from the island: bring the Claude desktop app to the front,
//! type the reply into its message box and press Enter, the same as picking a
//! reply button in the app's "next step" box (which sends that text as your
//! message).
//!
//! Windows only lets the foreground app hand the foreground to another
//! window. The island never takes focus, so we first press and release Alt
//! (the usual way to get that permission), then call SetForegroundWindow.
//! The text is typed as Unicode key events, so any language works.

use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_MENU, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowTextLengthW, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
};

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
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: code, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
    }
}

fn unicode(unit: u16, up: bool) -> INPUT {
    let flags = KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

/// Type `text` into the Claude app and send it. Errors when the app isn't open
/// or won't come to the front (then nothing is typed anywhere else).
pub fn send_to_claude(text: &str) -> Result<(), String> {
    let hwnd = claude_window().ok_or("The Claude app isn't open. Open it and pick again.")?;
    let raw = hwnd as isize;
    let text = text.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        let hwnd = raw as HWND;
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        send(&[vk(VK_MENU, false), vk(VK_MENU, true)]);
        SetForegroundWindow(hwnd);
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
        std::thread::sleep(Duration::from_millis(150));
        let mut keys: Vec<INPUT> = Vec::new();
        for unit in text.encode_utf16() {
            keys.push(unicode(unit, false));
            keys.push(unicode(unit, true));
        }
        send(&keys);
        std::thread::sleep(Duration::from_millis(60));
        send(&[vk(VK_RETURN, false), vk(VK_RETURN, true)]);
        let _ = tx.send(Ok(()));
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(Ok(()))
}
