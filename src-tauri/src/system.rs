//! Windows integration: log file, tray icon (visibility, status badge),
//! hiding the island while a fullscreen app is in front, opening folders.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::image::Image;
use tauri::{AppHandle, Manager};

use crate::windows::ISLAND;
use crate::AppState;

pub const TRAY_ID: &str = "crest";

/// The island is hidden because a fullscreen app is in front (we will show it again).
static HIDDEN_FOR_FULLSCREEN: AtomicBool = AtomicBool::new(false);
/// Last badge state put on the tray icon, so we only redraw it when it changes.
static TRAY_STATE: Mutex<String> = Mutex::new(String::new());

pub fn log_dir() -> std::path::PathBuf {
    crate::paths::local_dir().join("logs")
}

/// In a build without a console (the release exe), send stdout/stderr to
/// `%LOCALAPPDATA%\Crest\logs\atoll.log` so About → Open log folder has
/// something to show. Rust's std reads the standard handles on every write,
/// so redirecting them here covers every println!/eprintln!.
pub fn redirect_logs() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{GetConsoleWindow, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    if !unsafe { GetConsoleWindow() }.is_null() {
        return; // dev build in a terminal: keep the console
    }
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join("atoll.log");
    if std::fs::metadata(&file).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&file, dir.join("atoll.old.log"));
    }
    let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) else { return };
    let handle = f.into_raw_handle();
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle as _);
        SetStdHandle(STD_ERROR_HANDLE, handle as _);
    }
    println!("---- Atoll {} started ----", env!("CARGO_PKG_VERSION"));
}

/// Open a folder in Explorer ("data" or "logs").
pub fn open_folder(app: &AppHandle, which: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = match which {
        "logs" => log_dir(),
        _ => crate::paths::local_dir(),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

// ---------- tray ----------

/// Status shown on the tray while the island is hidden: (word, badge colour).
fn agent_status(app: &AppHandle) -> Option<(String, [u8; 3])> {
    let state = app.state::<AppState>();
    if !state.permissions.lock().unwrap().views().is_empty() {
        return Some(("Needs you".into(), [0x8f, 0xa3, 0xff]));
    }
    let list = state.sessions.lock().unwrap().snapshot();
    use crate::sessions::Status;
    let word = |s: &crate::sessions::Session| {
        let detail = s.activity.as_ref().map(|a| format!("{} {}", a.label, a.detail)).unwrap_or_default();
        format!("{} {}", s.project, detail).trim().to_string()
    };
    if let Some(s) = list.iter().find(|s| s.status == Status::Error) {
        return Some((format!("Error · {}", word(s)), [0xff, 0x7a, 0x7a]));
    }
    if let Some(s) = list.iter().find(|s| s.status == Status::Attention) {
        return Some((format!("Needs you · {}", word(s)), [0x8f, 0xa3, 0xff]));
    }
    if let Some(s) = list.iter().find(|s| s.status == Status::Working) {
        return Some((format!("Busy · {}", word(s)), [0xf5, 0xb9, 0x47]));
    }
    None
}

/// Draw a status dot in the top-right corner of the app icon.
fn badged(base: &Image<'_>, rgb: [u8; 3]) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut px = base.rgba().to_vec();
    let r = w as f32 * 0.24;
    let (cx, cy) = (w as f32 - r - 0.5, r + 0.5);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let i = ((y * w + x) * 4) as usize;
            if d <= r {
                px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            } else if d <= r + 1.5 {
                // dark ring so the dot reads on any taskbar
                px[i..i + 4].copy_from_slice(&[16, 16, 21, 255]);
            }
        }
    }
    Image::new_owned(px, w, h)
}

/// Keep the tray in sync: visibility, tooltip and the status badge (only
/// while the island is hidden for a fullscreen app, as the design says).
pub fn update_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let show = app.state::<AppState>().settings.lock().unwrap().general.show_tray;
    let hidden = HIDDEN_FOR_FULLSCREEN.load(Ordering::Relaxed);
    let status = if hidden { agent_status(app) } else { None };
    let key = format!("{show}|{:?}", status);
    {
        let mut last = TRAY_STATE.lock().unwrap();
        if *last == key {
            return;
        }
        *last = key;
    }
    let _ = tray.set_visible(show);
    let tip = match &status {
        Some((word, _)) => format!("Atoll (hidden during a fullscreen app) · {word}"),
        None if hidden => "Atoll (hidden during a fullscreen app)".to_string(),
        None => "Atoll".to_string(),
    };
    let _ = tray.set_tooltip(Some(tip.chars().take(120).collect::<String>()));
    if let Some(base) = app.default_window_icon() {
        let icon = match status {
            Some((_, rgb)) => badged(base, rgb),
            None => base.clone().to_owned(),
        };
        let _ = tray.set_icon(Some(icon));
    }
}

// ---------- fullscreen ----------

/// The user showed or hid the island themselves: forget that we hid it.
pub fn forget_fullscreen_hide() {
    HIDDEN_FOR_FULLSCREEN.store(false, Ordering::Relaxed);
}

/// What is in front on the island's screen.
#[derive(Clone, Copy, PartialEq)]
enum Front {
    /// Nothing fullscreen (or the desktop, the taskbar, Atoll itself).
    Normal,
    /// A fullscreen app: a game, a video, a slideshow. The island hides.
    App,
    /// A fullscreen system overlay that lasts a moment: the Snipping Tool
    /// overlay (Win+Shift+S, Print Screen), Click to Do, Game Bar. These are
    /// tool windows, never taskbar apps. The island stays as it is under them.
    Overlay,
}

/// What window (other than the desktop or Atoll) is in front on the monitor
/// at `point`, and does it cover that whole monitor?
fn front(point: (i32, i32)) -> Front {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, GWL_EXSTYLE,
        WS_EX_TOOLWINDOW,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return Front::Normal;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            return Front::Normal;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return Front::Normal;
        }
        let monitor = MonitorFromPoint(POINT { x: point.0, y: point.1 }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Front::Normal;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return Front::Normal;
        }
        // Covers the island's monitor (the snip overlay spans every monitor at once).
        let m = info.rcMonitor;
        if !(r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom) {
            return Front::Normal;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            Front::Overlay
        } else {
            Front::App
        }
    }
}

/// Is the cursor at `cursor` really over the island? Not while the island is
/// hidden, and not through a fullscreen overlay: dragging a snip across the
/// island used to open it underneath the Snipping Tool.
pub fn can_hover(island: isize, cursor: (i32, i32)) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    island != 0 && unsafe { IsWindowVisible(island as _) } != 0 && front(cursor) != Front::Overlay
}

/// Bring the island back without taking the keyboard from the app you are in.
/// `show()` activates the window (tao uses SW_SHOW), and an activated island
/// handed focus back to a text box inside it, which opened the panel.
fn show_quietly(win: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
    let before = unsafe { GetForegroundWindow() } as isize;
    let island = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    let _ = win.show();
    crate::backdrop::poke();
    // Queued behind the show, so it runs once the window is up.
    let _ = win.run_on_main_thread(move || unsafe {
        if before != 0 && island != 0 && before != island && GetForegroundWindow() as isize == island {
            SetForegroundWindow(before as _);
        }
    });
}

/// Every 750 ms: hide the island when a fullscreen app comes to the front on
/// its screen, and bring it back afterwards (only if we were the ones hiding it).
/// A passing overlay (a screenshot) changes nothing.
pub fn start_fullscreen_watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(750));
        let enabled = app.state::<AppState>().settings.lock().unwrap().general.hide_fullscreen;
        let Some(win) = app.get_webview_window(ISLAND) else { continue };
        let hidden_by_us = HIDDEN_FOR_FULLSCREEN.load(Ordering::Relaxed);
        if !enabled {
            if hidden_by_us {
                show_quietly(&win);
                HIDDEN_FOR_FULLSCREEN.store(false, Ordering::Relaxed);
                update_tray(&app);
            }
            continue;
        }
        let Ok(pos) = win.outer_position() else { continue };
        let Ok(size) = win.outer_size() else { continue };
        let point = (pos.x + size.width as i32 / 2, pos.y + 20);
        match front(point) {
            Front::App if !hidden_by_us && win.is_visible().unwrap_or(false) => {
                let _ = win.hide();
                crate::backdrop::poke();
                HIDDEN_FOR_FULLSCREEN.store(true, Ordering::Relaxed);
                update_tray(&app);
            }
            Front::Normal if hidden_by_us => {
                show_quietly(&win);
                HIDDEN_FOR_FULLSCREEN.store(false, Ordering::Relaxed);
                update_tray(&app);
            }
            _ => {}
        }
    });
}
