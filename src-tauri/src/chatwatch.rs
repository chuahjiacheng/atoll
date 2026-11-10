//! EXPERIMENTAL: "a reply finished" alerts for the Claude desktop Chat tab.
//!
//! The Chat tab has no hooks or API. Its page content is also not exposed to
//! Windows UI Automation (Electron only does that while a screen reader is
//! running). What *is* observable without touching Claude is the standard
//! taskbar-flash signal: when an app calls FlashWindow(Ex) Windows broadcasts
//! HSHELL_FLASH to windows registered with `RegisterShellHookWindow`.
//!
//! [Unverified] Whether the Claude app flashes its taskbar button when a chat
//! reply finishes in the background. The dashboard shows "last signal seen"
//! so this can be confirmed with one real chat. Code tab sessions are already
//! covered by hooks, so signals right after a desktop Code tab event are ignored.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::alerts::{self, Alert, Kind};
use crate::sessions::now_ms;
use crate::AppState;

/// Ignore Claude-window signals this soon after a Code tab hook event.
const CODE_TAB_GRACE_MS: u64 = 10_000;

#[derive(Default)]
pub struct ChatWatchState {
    last_signal_ms: AtomicU64,
    last_desktop_activity_ms: AtomicU64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWatchStatus {
    pub running: bool,
    /// Last taskbar flash seen from the Claude app (ms since epoch, 0 = never).
    pub last_signal_at: u64,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static SHELLHOOK_MSG: AtomicU32 = AtomicU32::new(0);

pub fn note_desktop_activity(app: &AppHandle) {
    app.state::<ChatWatchState>().last_desktop_activity_ms.store(now_ms(), Ordering::Relaxed);
}

pub fn status(app: &AppHandle) -> ChatWatchStatus {
    ChatWatchStatus {
        running: SHELLHOOK_MSG.load(Ordering::Relaxed) != 0,
        last_signal_at: app.state::<ChatWatchState>().last_signal_ms.load(Ordering::Relaxed),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// File name of the process that owns `hwnd`, lower-cased (e.g. "claude.exe").
pub(crate) fn process_name(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase())
    }
}

fn is_claude_window(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    process_name(hwnd).as_deref() == Some("claude.exe")
}

fn on_flash(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let Some(app) = APP.get() else { return };
    if !is_claude_window(hwnd) {
        return;
    }
    let now = now_ms();
    let state = app.state::<ChatWatchState>();
    state.last_signal_ms.store(now, Ordering::Relaxed);
    let _ = app.emit("chatwatch-signal", now);
    println!("[crest] chat watcher: Claude window flashed");

    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    if !settings.modules.chat_watcher {
        return;
    }
    if settings.chat_watcher.only_when_unfocused && is_claude_window(unsafe { GetForegroundWindow() }) {
        return;
    }
    if now.saturating_sub(state.last_desktop_activity_ms.load(Ordering::Relaxed)) < CODE_TAB_GRACE_MS {
        return; // a Code tab session caused it; hooks already alerted
    }
    alerts::fire(
        app,
        Alert {
            kind: Kind::ChatReply,
            session_id: Some("claude-chat".into()),
            title: "Claude replied".into(),
            body: "A reply finished in the Claude app.".into(),
        },
        false,
    );
}

unsafe extern "system" fn wndproc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW;
    // winuser.h: HSHELL_FLASH = HSHELL_REDRAW (6) | HSHELL_HIGHBIT (0x8000)
    const HSHELL_FLASH: u32 = 0x8006;
    let hook = SHELLHOOK_MSG.load(Ordering::Relaxed);
    if hook != 0 && msg == hook {
        if wparam as u32 == HSHELL_FLASH {
            on_flash(lparam as windows_sys::Win32::Foundation::HWND);
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Start the watcher thread (one hidden window + message loop). Cheap; it
/// only reacts to taskbar-flash broadcasts.
pub fn start(app: AppHandle) {
    let _ = APP.set(app);
    std::thread::spawn(|| unsafe {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, RegisterShellHookWindow,
            RegisterWindowMessageW, TranslateMessage, MSG, WNDCLASSW,
        };
        let class = wide("CrestShellHookWindow");
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        // A normal (never shown) top-level window: message-only windows don't get shell hooks.
        let title = wide("Crest shell hook");
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
        if hwnd.is_null() {
            eprintln!("[crest] chat watcher: could not create window");
            return;
        }
        SHELLHOOK_MSG.store(RegisterWindowMessageW(wide("SHELLHOOK").as_ptr()), Ordering::Relaxed);
        if RegisterShellHookWindow(hwnd) == 0 {
            eprintln!("[crest] chat watcher: RegisterShellHookWindow failed");
            SHELLHOOK_MSG.store(0, Ordering::Relaxed);
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
