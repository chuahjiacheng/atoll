//! The two windows: the island overlay and the settings dashboard.

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::settings::Appearance;

pub const ISLAND: &str = "island";
pub const DASHBOARD: &str = "dashboard";

/// Room around the island inside its transparent window, so the drop shadow
/// (0 16px 40px) and status glow (28px) are never clipped.
const SHADOW_PAD: u32 = 56;
/// Room kept above the island inside the window (CSS px) so its glow isn't cut.
const TOP_ROOM: f64 = 32.0;
/// The widest island state keeps this far from the screen edges (logical px).
const EDGE: f64 = 16.0;
/// Peeks are 440 px wide; the window must fit them even if the panel is narrower.
const PEEK_WIDTH: u32 = 440;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// Windows display scaling (1.25 = 125 %).
    pub scale: f64,
}

pub fn list_monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let primary = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    app.available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let name = m.name().cloned().unwrap_or_else(|| "Unknown".into());
            MonitorInfo {
                primary: primary.as_deref() == Some(name.as_str()),
                width: m.size().width,
                height: m.size().height,
                scale: m.scale_factor(),
                name,
            }
        })
        .collect()
}

/// Size the island window for the current appearance and centre it at the
/// top of the chosen monitor.
pub fn place_island(app: &AppHandle, a: &Appearance) {
    let Some(win) = app.get_webview_window(ISLAND) else { return };
    let monitor = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .find(|m| a.monitor != "primary" && m.name().map(String::as_str) == Some(a.monitor.as_str()))
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| win.current_monitor().ok().flatten());
    let Some(m) = monitor else { return };

    // The whole page zooms by the island scale; the window is sized to match.
    let zoom = a.scale as f64 / 100.0;
    let _ = win.set_zoom(zoom);
    let widest = a.expanded_width.max(a.collapsed_width).max(PEEK_WIDTH) as f64;
    let width = (widest + SHADOW_PAD as f64 * 2.0) * zoom;
    let height = (TOP_ROOM + a.expanded_height as f64 + SHADOW_PAD as f64) * zoom;
    let _ = win.set_size(LogicalSize::new(width, height));

    // Vertical: part of the gap is inside the window (room for the glow), the rest above it.
    // The page puts the island min(gap, TOP_ROOM * zoom) below the window top (theme.ts).
    let gap = a.position_y as f64;
    let above = (gap - TOP_ROOM * zoom).max(0.0);

    // Horizontal: centre the island at position_x, but keep its widest state on screen.
    let scale = m.scale_factor();
    let (mx, mw) = (m.position().x as f64, m.size().width as f64);
    let (win_w, pad, island_w, edge) = (width * scale, SHADOW_PAD as f64 * zoom * scale, widest * zoom * scale, EDGE * scale);
    let centre = mx + mw * a.position_x as f64 / 100.0;
    let min_x = mx + edge - pad;
    let max_x = (mx + mw - edge - pad - island_w).max(min_x);
    let x = (centre - win_w / 2.0).clamp(min_x, max_x);
    let y = m.position().y as f64 + above * scale;
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub fn toggle_island(app: &AppHandle) {
    crate::system::forget_fullscreen_hide();
    if let Some(win) = app.get_webview_window(ISLAND) {
        let visible = win.is_visible().unwrap_or(true);
        let _ = if visible { win.hide() } else { win.show() };
    }
    // The blur under the island follows at once, not on its next check.
    crate::backdrop::poke();
}

pub fn set_island_visible(app: &AppHandle, visible: bool) {
    crate::system::forget_fullscreen_hide();
    if let Some(win) = app.get_webview_window(ISLAND) {
        let _ = if visible { win.show() } else { win.hide() };
    }
    crate::backdrop::poke();
}

/// Pin / unpin the expanded island (the island page owns that state).
pub fn toggle_island_expanded(app: &AppHandle) {
    set_island_visible(app, true);
    let _ = app.emit_to(ISLAND, "toggle-pin", ());
}

/// Module hotkey: open the island on one tab (pressing it again closes it).
/// Clipboard, note and Ask also take keyboard focus so you can type right away.
pub fn open_tab(app: &AppHandle, tab: &str) {
    crate::paste::remember_foreground();
    set_island_visible(app, true);
    let _ = app.emit_to(ISLAND, "open-tab", tab);
    if matches!(tab, "note" | "clipboard" | "ask") {
        if let Some(win) = app.get_webview_window(ISLAND) {
            let _ = win.set_focus();
        }
    }
}

/// The page a "Connect …" button asked Settings to open on. A window that is
/// still being built takes it when it loads; one already open gets
/// `settings:goto` (and takes it then, so it is not used twice).
static PENDING_PAGE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Open Settings on one page (not the page it was last left on).
pub fn open_dashboard_at(app: &AppHandle, page: &str) {
    *PENDING_PAGE.lock().unwrap() = Some(page.to_string());
    open_dashboard(app);
    let _ = app.emit_to(DASHBOARD, "settings:goto", page);
}

/// The page Settings was asked to open on, once.
pub fn take_dashboard_page() -> Option<String> {
    PENDING_PAGE.lock().unwrap().take()
}

/// Open the settings dashboard, or bring it to the front if it is open.
///
/// Built on a helper thread: creating a window from code that runs on the
/// main event loop (tray / hotkey / single-instance callbacks, sync commands)
/// can deadlock on Windows.
pub fn open_dashboard(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(DASHBOARD) {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let built: tauri::Result<WebviewWindow> =
            WebviewWindowBuilder::new(&app, DASHBOARD, WebviewUrl::App("dashboard.html".into()))
                .title("Atoll Settings")
                .inner_size(1040.0, 720.0)
                .min_inner_size(820.0, 560.0)
                // The design draws its own 32 px title bar (min / max / close).
                .decorations(false)
                .shadow(true)
                .center()
                .build();
        if let Err(e) = built {
            eprintln!("[crest] could not open settings: {e}");
        }
    });
}
