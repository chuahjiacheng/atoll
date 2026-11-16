//! The command palette window (owned by `search-clipboard`).
//!
//! A separate small window, drawn as a peek under the island, so the island
//! itself keeps its rule of never taking keyboard focus. The palette takes
//! focus only while open (you type into it) and hides the moment it loses
//! focus or you press Esc. Before it opens, the app you were in is
//! remembered so "Paste" goes back there.
//!
//! Width: declared as island state `peek:palette` (see
//! docs/requests/search-clipboard.md); 560 px content until the width
//! contract settles it.

use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const LABEL: &str = "palette";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+KeyK";

/// Content size plus room for the peek's shadow (CSS px).
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 440.0;
const PAD: f64 = 32.0;
/// Top of the palette below the top of the screen: under the tallest pill.
const TOP: f64 = 8.0 + 56.0 + 8.0;

pub fn toggle(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        Some(w) if w.is_visible().unwrap_or(false) => hide(app),
        _ => show(app),
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
}

fn place(app: &AppHandle, w: &tauri::WebviewWindow) {
    // Same monitor as the island.
    let monitor = app
        .get_webview_window(crate::windows::ISLAND)
        .and_then(|i| i.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let (mx, my) = (m.position().x as f64 / scale, m.position().y as f64 / scale);
    let mw = m.size().width as f64 / scale;
    let (ww, wh) = (WIDTH + PAD * 2.0, HEIGHT + PAD * 2.0);
    let _ = w.set_size(LogicalSize::new(ww, wh));
    let _ = w.set_position(LogicalPosition::new(mx + (mw - ww) / 2.0, my + TOP - PAD));
}

pub fn show(app: &AppHandle) {
    crate::paste::remember_foreground();
    if let Some(w) = app.get_webview_window(LABEL) {
        place(app, &w);
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit_to(LABEL, "palette-open", ());
        return;
    }
    // Built on a helper thread: creating a window from a hotkey callback on
    // the main loop can deadlock on Windows (see windows::open_dashboard).
    let app = app.clone();
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("palette.html".into()))
            .title("Atoll search")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(true)
            .inner_size(WIDTH + PAD * 2.0, HEIGHT + PAD * 2.0)
            .build();
        match built {
            Ok(w) => {
                let handle = app.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        hide(&handle);
                    }
                });
                place(&app, &w);
                // The page shows itself once it has drawn (palette_ready), so
                // the first open never flashes an empty window.
            }
            Err(e) => eprintln!("[atoll] could not open search: {e}"),
        }
    });
}

/// The page has drawn: show and focus it (first open only).
pub fn ready(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = app.emit_to(LABEL, "palette-open", ());
        }
    }
}
