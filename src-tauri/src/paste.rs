//! "Click to paste": remember which app you were in before you reached for
//! the island, and paste into it.
//!
//! When the cursor enters the island we note the foreground window (unless
//! it is one of Crest's own). Pasting puts that window back in front and
//! sends Ctrl+V with SendInput, exactly like pressing the keys yourself.
//! Windows only lets the foreground app change the foreground window; after
//! you click the island Crest is the foreground app, so this is allowed.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow};

static PREVIOUS: AtomicIsize = AtomicIsize::new(0);

fn is_own(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid == std::process::id()
}

/// Remember the current foreground window as the paste target.
pub fn remember_foreground() {
    let hwnd = unsafe { GetForegroundWindow() };
    if !hwnd.is_null() && !is_own(hwnd) {
        PREVIOUS.store(hwnd as isize, Ordering::Relaxed);
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// Bring the remembered window back and press Ctrl+V in it.
pub fn paste_into_previous() -> Result<(), String> {
    let hwnd = PREVIOUS.load(Ordering::Relaxed) as HWND;
    if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
        return Err("Copied. Atoll doesn't know where to paste: click into the app first, then use the island.".into());
    }
    let raw = hwnd as isize;
    std::thread::spawn(move || unsafe {
        SetForegroundWindow(raw as HWND);
        // Give the app a moment to take focus before the keys arrive.
        std::thread::sleep(Duration::from_millis(120));
        let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    });
    Ok(())
}
