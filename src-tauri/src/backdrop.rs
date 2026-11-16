//! Soft blur behind the island.
//!
//! The island is a transparent web view, and a page's `backdrop-filter` only
//! sees the page itself, never the desktop. So wherever the island is not
//! fully opaque, what lies behind it showed through sharp. Windows can blur
//! what lies behind a window, so a second, empty window sits just under the
//! island window and draws that blur in the island's exact shape.
//!
//! How the shape is made matters. A window *region* plus an accent blur came
//! out as a square box on Windows 11 (build 26200): Windows ignores the region
//! once the accent is on. So the blur is drawn with Windows' visual layer
//! instead: a host-backdrop brush (the blurred desktop behind the window, which
//! Windows itself provides) on a visual clipped to an anti-aliased rounded
//! rectangle. `DWMWA_USE_HOSTBACKDROPBRUSH` lets a classic window use it
//! (Windows 11). The helper window is the size of the island window and lets
//! every click through; only the rounded visual inside it moves as the island
//! morphs, so the window itself never resizes mid-animation.
//!
//! - The island page reports its shape (`set_backdrop`, CSS px in its window).
//! - It shows only while the island is visible, Settings → Appearance → Blur
//!   is above 0, the island is not fully opaque, and Windows' own
//!   transparency effects are on.
//! - It never takes focus, and clicks pass straight through it.
//! - `probe_rounded_blur` (an ignored test) shows it for a moment at the top
//!   left of the screen and saves a capture, to check it on a given PC.

use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows::core::Interface;
use windows::System::DispatcherQueueController;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT};
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::Win32::System::WinRT::Composition::ICompositorInterop;
use windows::UI::Composition::{
    CompositionColorBrush, CompositionRoundedRectangleGeometry, CompositionStretch, Compositor, ContainerVisual,
    SpriteVisual,
};
use windows_numerics::{Vector2, Vector3};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowRect, IsWindowVisible, PostMessageW,
    RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, LWA_ALPHA,
    MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_APP,
    WM_MOUSEACTIVATE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

use crate::liquid::{Liquid, Look};

use crate::windows::ISLAND;
use crate::AppState;

/// The island's shape in physical px, relative to the island window's top-left.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Shape {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
}

/// What the helper last showed, so unchanged frames cost nothing.
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    /// The island window's screen rectangle (the helper window matches it).
    window: (i32, i32, i32, i32),
    shape: Shape,
    tint: u32,
    /// Liquid glass (liquid.rs) instead of Windows' blur.
    liquid: bool,
}

/// Switch for Liquid glass. Off = "Liquid" falls back to Windows' blur. `probe_liquid_glass`
/// showed it working on Windows 11 build 26200 (2026-11-08).
const LIQUID_ENABLED: bool = true;
/// Hull tint over the liquid glass (the island draws its hull on top too).
const LIQUID_TINT: f32 = 0.08;

static APP: OnceLock<AppHandle> = OnceLock::new();
static BACKDROP: AtomicIsize = AtomicIsize::new(0);
static ISLAND_HWND: AtomicIsize = AtomicIsize::new(0);
static SHAPE: Mutex<Option<Shape>> = Mutex::new(None);
static PENDING: AtomicBool = AtomicBool::new(false);
static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);
static SYSTEM: Mutex<Option<(Instant, bool, bool)>> = Mutex::new(None);
/// DEV ONLY (tray item exists only in debug builds): let the island into
/// screenshots. Liquid glass freezes on its last clean frame meanwhile, since
/// it would refract itself once it is capturable. Remove before release.
static SCREENSHOTS: AtomicBool = AtomicBool::new(false);
/// What the island/helper display affinity currently is.
static EXCLUDED: AtomicBool = AtomicBool::new(false);

/// Flip dev screenshot mode (tray check item, debug builds).
pub fn toggle_screenshots() {
    SCREENSHOTS.fetch_xor(true, Ordering::Relaxed);
    poke();
}

thread_local! {
    /// The visual tree, owned by the helper window's thread.
    static GLASS: RefCell<Option<Glass>> = const { RefCell::new(None) };
}

/// Follow the island even when nothing reports (it moved, hid or changed theme).
const RESYNC_MS: u32 = 250;
/// Windows' transparency and light/dark switches are re-read this often.
const SYSTEM_TTL: Duration = Duration::from_secs(2);
/// Tint over the blur: the hull colour at this alpha (the island page draws
/// its own hull on top at the user's opacity).
const TINT_ALPHA: u32 = 0x30;
const DARK_HULL: u32 = 0x10_10_15;
const LIGHT_HULL: u32 = 0xFC_FC_FD;

/// The island page's shape (CSS px in the island window), sent on every change.
#[tauri::command]
pub fn set_backdrop(app: AppHandle, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let Some(win) = app.get_webview_window(ISLAND) else { return };
    let dpi = win.scale_factor().unwrap_or(1.0);
    let zoom = app.state::<AppState>().settings.lock().unwrap().appearance.scale as f64 / 100.0;
    *SHAPE.lock().unwrap() = Some(to_physical(x, y, w, h, r, dpi * zoom));
    poke();
}

/// CSS px → physical px, kept 1 px inside the island's edge, so the blur never
/// shows as a rim around the island's own soft edge.
fn to_physical(x: f64, y: f64, w: f64, h: f64, r: f64, scale: f64) -> Shape {
    let inset = 1.0;
    Shape {
        x: (x * scale + inset).round() as i32,
        y: (y * scale + inset).round() as i32,
        w: (w * scale - 2.0 * inset).round().max(0.0) as i32,
        h: (h * scale - 2.0 * inset).round().max(0.0) as i32,
        r: (r * scale - inset).round().max(0.0) as i32,
    }
}

/// Re-check now (the island was shown or hidden, or its shape changed).
pub fn poke() {
    let hwnd = BACKDROP.load(Ordering::Relaxed);
    if hwnd != 0 && !PENDING.swap(true, Ordering::AcqRel) {
        unsafe { PostMessageW(hwnd as HWND, WM_APP, 0, 0) };
    }
}

/// Switch for the whole feature. `probe_rounded_blur` showed a rounded,
/// anti-aliased blur on Windows 11 build 26200 (2026-11-07).
const ENABLED: bool = true;

/// Create the helper window on its own thread (it needs a message loop).
pub fn start(app: AppHandle) {
    if !ENABLED || APP.set(app).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("island-backdrop".into()).spawn(|| unsafe {
        let Some(hwnd) = create_window() else { return };
        match Glass::new(hwnd) {
            Ok(glass) => GLASS.with(|g| *g.borrow_mut() = Some(glass)),
            Err(e) => {
                // No host backdrop here (Windows 10, or Windows refused): no blur, nothing shown.
                eprintln!("[atoll] island blur unavailable: {e}");
                return;
            }
        }
        BACKDROP.store(hwnd as isize, Ordering::Relaxed);
        SetTimer(hwnd, 1, RESYNC_MS, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A hidden, click-through, never-activated window with no GDI surface (only
/// composition visuals draw in it).
unsafe fn create_window() -> Option<HWND> {
    let class = wide("AtollIslandBackdrop");
    let instance = GetModuleHandleW(null());
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: null_mut(),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    // A second registration (the probe test after another) fails harmlessly.
    RegisterClassW(&wc);
    let title = wide("Atoll island blur");
    let hwnd = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        class.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        eprintln!("[atoll] island blur: could not create its window");
        return None;
    }
    // Layered + transparent = clicks pass through; full alpha so the visuals show as drawn.
    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
    Some(hwnd)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_APP => {
            PENDING.store(false, Ordering::Release);
            frame(hwnd);
            0
        }
        WM_TIMER => {
            // The slow resync. While a move is running, its own frames do this.
            if !crate::shape::moving(crate::shape::now_ms()) {
                sync(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// One frame of the glass. While the island's shape is moving (shape.rs) this
/// repeats once per screen refresh, each time drawing the rectangle the shape
/// has at that moment; when the shape comes to rest it stops, and the helper
/// goes back to waking only on changes and its slow resync timer.
unsafe fn frame(hwnd: HWND) {
    let t = crate::shape::now_ms();
    let in_run = crate::shape::moving(t);
    let drew = sync(hwnd);
    if let Some(d) = &drew {
        crate::shape::glass_drew(t, in_run, if d.liquid { "liquid" } else { "frosted" }, d.rect);
    } else {
        crate::shape::glass_drew(t, false, "off", Default::default());
    }
    if in_run && drew.is_some() {
        // Liquid glass waits for the screen itself when it presents a frame.
        // Otherwise wait for the screen's next refresh, so this runs once per
        // refresh instead of spinning.
        if !drew.is_some_and(|d| d.presented) {
            wait_for_refresh();
        }
        poke();
    }
}

/// Sleep until the screen's next refresh.
///
/// Neither `DwmFlush` nor a display output's `WaitForVBlank` blocks on this
/// PC (both were measured returning in under 0.2 ms, so a loop built on them
/// spun thousands of times a second). So the wait is worked out instead:
/// Windows' compositor reports when its last refresh happened and how long a
/// refresh lasts, and this sleeps to the next one. If that is unavailable it
/// sleeps one refresh. Either way it always sleeps: the loop cannot spin.
unsafe fn wait_for_refresh() {
    use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let period_ms = refresh_ms();
    let mut wait_ms = period_ms;
    let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    if DwmGetCompositionTimingInfo(windows::Win32::Foundation::HWND::default(), &mut info).is_ok() && info.qpcRefreshPeriod > 0 {
        let (mut now, mut freq) = (0i64, 0i64);
        QueryPerformanceCounter(&mut now);
        QueryPerformanceFrequency(&mut freq);
        if freq > 0 {
            let period = info.qpcRefreshPeriod as i64;
            let since = (now - info.qpcVBlank as i64).rem_euclid(period);
            wait_ms = (period - since) as f64 * 1000.0 / freq as f64;
            // Too close to call: take the refresh after.
            if wait_ms < 1.0 {
                wait_ms += period_ms;
            }
        }
    }
    std::thread::sleep(Duration::from_secs_f64(wait_ms.clamp(1.0, 40.0) / 1000.0));
}

/// What `sync` put on screen.
#[derive(Clone, Copy)]
struct Drew {
    liquid: bool,
    /// The liquid renderer presented a frame (which paces to the screen).
    presented: bool,
    /// The shape drawn, CSS px in the island window.
    rect: crate::shape::Rect,
}

/// One screen refresh in ms, from Windows' compositor (cached with the other system values).
fn refresh_ms() -> f64 {
    static CACHE: Mutex<Option<(Instant, f64)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, ms)) = *cache {
        if at.elapsed() < SYSTEM_TTL {
            return ms;
        }
    }
    let mut info = windows::Win32::Graphics::Dwm::DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<windows::Win32::Graphics::Dwm::DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    let ms = unsafe { windows::Win32::Graphics::Dwm::DwmGetCompositionTimingInfo(windows::Win32::Foundation::HWND::default(), &mut info) }
        .ok()
        .filter(|_| info.rateRefresh.uiNumerator > 0)
        .map(|_| 1000.0 * info.rateRefresh.uiDenominator as f64 / info.rateRefresh.uiNumerator as f64)
        .filter(|ms| (2.0..=40.0).contains(ms))
        .unwrap_or(1000.0 / 60.0);
    *cache = Some((Instant::now(), ms));
    ms
}

/// How far ahead of "now" the glass aims, in ms. 0 by default: the page and
/// the glass are assumed to reach the screen about equally late. [Unverified]
/// Set ATOLL_GLASS_LEAD_MS to try other values on a given PC.
fn lead_ms() -> f64 {
    static LEAD: OnceLock<f64> = OnceLock::new();
    *LEAD.get_or_init(|| std::env::var("ATOLL_GLASS_LEAD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0))
}

/// The blur and its tint: two visuals sharing one rounded clip.
struct Glass {
    _queue: DispatcherQueueController,
    compositor: Compositor,
    root: ContainerVisual,
    /// Made the first time Liquid is used.
    liquid: Option<LiquidVisual>,
    _target: DesktopWindowTarget,
    blur: SpriteVisual,
    tint: SpriteVisual,
    tint_brush: CompositionColorBrush,
    geometry: CompositionRoundedRectangleGeometry,
}

impl Glass {
    unsafe fn new(hwnd: HWND) -> windows::core::Result<Glass> {
        let on = windows::core::BOOL(1);
        DwmSetWindowAttribute(
            windows::Win32::Foundation::HWND(hwnd),
            DWMWA_USE_HOSTBACKDROPBRUSH,
            (&on as *const windows::core::BOOL).cast(),
            std::mem::size_of::<windows::core::BOOL>() as u32,
        )?;
        let queue = CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })?;
        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        let target = interop.CreateDesktopWindowTarget(windows::Win32::Foundation::HWND(hwnd), false)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;

        let geometry = compositor.CreateRoundedRectangleGeometry()?;
        let blur = compositor.CreateSpriteVisual()?;
        blur.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
        blur.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
        let tint_brush = compositor.CreateColorBrushWithColor(Color { A: 0, R: 0, G: 0, B: 0 })?;
        let tint = compositor.CreateSpriteVisual()?;
        tint.SetBrush(&tint_brush)?;
        tint.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
        let children = root.Children()?;
        children.InsertAtTop(&blur)?;
        children.InsertAtTop(&tint)?;
        Ok(Glass { _queue: queue, compositor, root, liquid: None, _target: target, blur, tint, tint_brush, geometry })
    }

    /// Windows' blur or the liquid renderer: show one, hide the other.
    fn mode(&mut self, liquid: bool) -> windows::core::Result<()> {
        self.blur.SetIsVisible(!liquid)?;
        self.tint.SetIsVisible(!liquid)?;
        if liquid && self.liquid.is_none() {
            self.liquid = Some(LiquidVisual::new(&self.compositor, &self.root)?);
        }
        if let Some(l) = &self.liquid {
            l.visual.SetIsVisible(liquid)?;
            if !liquid {
                l.renderer.stop();
            }
        }
        Ok(())
    }

    /// Put the rounded blur at `s` (px inside the helper window), tinted `rgb`.
    fn place(&self, s: Shape, rgb: u32) -> windows::core::Result<()> {
        let offset = Vector3 { X: s.x as f32, Y: s.y as f32, Z: 0.0 };
        let size = Vector2 { X: s.w as f32, Y: s.h as f32 };
        for v in [&self.blur, &self.tint] {
            v.SetOffset(offset)?;
            v.SetSize(size)?;
        }
        self.geometry.SetSize(size)?;
        self.geometry.SetCornerRadius(Vector2 { X: s.r as f32, Y: s.r as f32 })?;
        self.tint_brush.SetColor(Color {
            A: TINT_ALPHA as u8,
            R: (rgb >> 16) as u8,
            G: (rgb >> 8) as u8,
            B: rgb as u8,
        })
    }
}

/// The liquid renderer's swap chain on a visual the size of the helper window,
/// clipped to the island's rounded shape.
struct LiquidVisual {
    renderer: Liquid,
    visual: SpriteVisual,
    geometry: CompositionRoundedRectangleGeometry,
}

impl LiquidVisual {
    fn new(compositor: &Compositor, root: &ContainerVisual) -> windows::core::Result<Self> {
        let renderer = Liquid::new(1, 1)?;
        let interop: ICompositorInterop = compositor.cast()?;
        let surface = unsafe { interop.CreateCompositionSurfaceForSwapChain(&renderer.swap_chain())? };
        let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
        brush.SetStretch(CompositionStretch::Fill)?;
        let visual = compositor.CreateSpriteVisual()?;
        visual.SetBrush(&brush)?;
        let geometry = compositor.CreateRoundedRectangleGeometry()?;
        visual.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
        visual.SetIsVisible(false)?;
        root.Children()?.InsertAtTop(&visual)?;
        Ok(LiquidVisual { renderer, visual, geometry })
    }

    /// Size it to the window, clip it to `s`, and hand the renderer the look.
    /// Returns whether the renderer presented a new frame.
    fn place(&self, window: (i32, i32, i32, i32), s: Shape, look: Look) -> windows::core::Result<bool> {
        self.visual.SetSize(Vector2 { X: window.2 as f32, Y: window.3 as f32 })?;
        self.geometry.SetOffset(Vector2 { X: s.x as f32, Y: s.y as f32 })?;
        self.geometry.SetSize(Vector2 { X: s.w as f32, Y: s.h as f32 })?;
        self.geometry.SetCornerRadius(Vector2 { X: s.r as f32, Y: s.r as f32 })?;
        Ok(self.renderer.set(look, window))
    }
}

/// The monitor a window is on, and its top-left in screen px.
unsafe fn monitor_of(hwnd: HWND) -> (isize, (i32, i32)) {
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut info);
    (monitor as isize, (info.rcMonitor.left, info.rcMonitor.top))
}

/// Keep the island and the helper out of screen captures (or let them back in).
/// The liquid renderer captures the screen and must not see its own glass.
unsafe fn exclude_from_capture(island: isize, helper: HWND, on: bool) {
    if EXCLUDED.swap(on, Ordering::Relaxed) == on {
        return;
    }
    let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    SetWindowDisplayAffinity(island as HWND, affinity);
    SetWindowDisplayAffinity(helper, affinity);
}

/// The island's DPI scale (blur is set in CSS px).
unsafe fn dpi_of(island: isize) -> f32 {
    windows::Win32::UI::HiDpi::GetDpiForWindow(windows::Win32::Foundation::HWND(island as _)).max(96) as f32 / 96.0
}

fn island_hwnd(app: &AppHandle) -> isize {
    let cached = ISLAND_HWND.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let found = app
        .get_webview_window(ISLAND)
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);
    ISLAND_HWND.store(found, Ordering::Relaxed);
    found
}

/// Windows' "Transparency effects" and "app mode: light", cached for a moment.
fn system_look() -> (bool, bool) {
    let mut cache = SYSTEM.lock().unwrap();
    if let Some((at, transparency, light)) = *cache {
        if at.elapsed() < SYSTEM_TTL {
            return (transparency, light);
        }
    }
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .ok();
    let read = |name: &str, default: u32| key.as_ref().and_then(|k| k.get_value::<u32, _>(name).ok()).unwrap_or(default);
    let (transparency, light) = (read("EnableTransparency", 1) != 0, read("AppsUseLightTheme", 0) != 0);
    *cache = Some((Instant::now(), transparency, light));
    (transparency, light)
}

/// Show, move, reshape or hide the blur to match the island.
unsafe fn sync(hwnd: HWND) -> Option<Drew> {
    let app = APP.get()?;
    let island = island_hwnd(app);
    let state = app.state::<AppState>();
    let (blur, opaque, theme, liquid, distortion, shine, zoom) = {
        let s = state.settings.lock().unwrap();
        let a = &s.appearance;
        (a.blur, a.opacity >= 100, a.theme.clone(), LIQUID_ENABLED && a.glass == "liquid", a.distortion, a.liquid, a.scale as f64 / 100.0)
    };
    // Liquid glass shows even at blur 0 (clear glass); Windows' blur needs > 0.
    let blur_on = liquid || blur > 0;
    let (transparency, system_light) = system_look();
    // The shape for this very frame, worked out from the island's current
    // move (shape.rs). Before the page has sent one, the last reported shape.
    let live = if island != 0 {
        crate::shape::glass_rect(crate::shape::now_ms() + lead_ms(), refresh_ms() / 2.0)
    } else {
        None
    };
    let shape = match live {
        Some(r) => Some(to_physical(r.x, r.y, r.w, r.h, r.r, dpi_of(island) as f64 * zoom)),
        None => *SHAPE.lock().unwrap(),
    };
    let visible = island != 0 && IsWindowVisible(island as HWND) != 0;
    let mut shown = SHOWN.lock().unwrap();
    let wanted = shape.filter(|s| blur_on && !opaque && transparency && visible && s.w > 0 && s.h > 0);
    let Some(s) = wanted else {
        if let Some(before) = shown.take() {
            ShowWindow(hwnd, SW_HIDE);
            if before.liquid {
                GLASS.with(|g| g.borrow_mut().as_mut().map(|glass| glass.mode(false)));
                exclude_from_capture(island, hwnd, false);
            }
        }
        return None;
    };

    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(island as HWND, &mut r) == 0 {
        return None;
    }
    let light = theme == "light" || (theme != "dark" && system_light);
    let next = Shown {
        window: (r.left, r.top, r.right - r.left, r.bottom - r.top),
        shape: s,
        tint: if light { LIGHT_HULL } else { DARK_HULL },
        liquid,
    };
    let before = *shown;
    if before.map(|b| b.liquid) != Some(liquid) {
        let switched = GLASS.with(|g| g.borrow_mut().as_mut().map(|glass| glass.mode(liquid)));
        if !matches!(switched, Some(Ok(()))) {
            eprintln!("[atoll] liquid glass unavailable: {switched:?}");
            return None;
        }
        if !liquid {
            exclude_from_capture(island, hwnd, false);
        }
    }
    let mut presented = false;
    if liquid {
        let look = Look {
            shape: [s.x as f32, s.y as f32, s.w as f32, s.h as f32, s.r as f32],
            blur: blur as f32 * dpi_of(island),
            distortion: distortion as f32 / 100.0,
            liquid: shine as f32 / 100.0,
            tint: next.tint,
            tint_alpha: LIQUID_TINT,
        };
        let (monitor, origin) = monitor_of(island as HWND);
        let placed = GLASS.with(|g| {
            g.borrow().as_ref().and_then(|glass| glass.liquid.as_ref()).map(|l| {
                // Screenshot mode needs one clean frame first; until then keep capturing.
                if SCREENSHOTS.load(Ordering::Relaxed) && l.renderer.has_frame() {
                    l.renderer.freeze(); // stop capturing before the glass becomes visible to capture
                    exclude_from_capture(island, hwnd, false);
                } else {
                    exclude_from_capture(island, hwnd, true); // hide before capture restarts
                    l.renderer.capture(monitor, origin)?;
                }
                l.place(next.window, s, look)
            })
        });
        match placed {
            Some(Ok(drawn)) => presented = drawn,
            Some(Err(e)) => eprintln!("[atoll] liquid glass: {e}"),
            None => {}
        }
    } else if before.map(|b| (b.shape, b.tint, b.liquid)) != Some((next.shape, next.tint, false)) {
        let placed = GLASS.with(|g| g.borrow().as_ref().map(|glass| glass.place(next.shape, next.tint)));
        if !matches!(placed, Some(Ok(()))) {
            return None;
        }
    }
    let flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW;
    if before.map(|b| b.window) == Some(next.window) {
        // Keep it directly under the island in the z-order (inserted after it).
        // Not on every frame of a move: nothing reorders windows that fast.
        if live.is_none() || before.map(|b| b.shape) == Some(next.shape) {
            SetWindowPos(hwnd, island as HWND, 0, 0, 0, 0, flags | SWP_NOMOVE | SWP_NOSIZE);
        }
    } else {
        let (x, y, w, h) = next.window;
        SetWindowPos(hwnd, island as HWND, x, y, w, h, flags);
    }
    *shown = Some(next);
    Some(Drew { liquid, presented, rect: live.unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_scales_and_stays_inside_the_edge() {
        // 384 × 40 pill, 20 px corners, at 125 % display scaling and 75 % island zoom.
        let s = to_physical(100.0, 32.0, 384.0, 40.0, 20.0, 1.25 * 0.75);
        assert_eq!(s, Shape { x: 95, y: 31, w: 358, h: 36, r: 18 });
        // Nothing to blur for an empty shape.
        assert_eq!(to_physical(0.0, 0.0, 1.0, 1.0, 0.0, 1.0).w, 0);
    }

    /// Shows the rounded blur for a moment at the top left of the main screen
    /// and saves a capture of it (path printed). Run by hand:
    /// `cargo test --lib probe_rounded_blur -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_rounded_blur() {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, HWND_TOPMOST, PM_REMOVE};
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let hwnd = create_window().expect("window");
            let glass = Glass::new(hwnd).expect("host backdrop");
            // A 360 × 120 pill with 60 px corners, inside a 440 × 200 window.
            let (wx, wy, ww, wh) = (40, 40, 440, 200);
            glass.place(Shape { x: 40, y: 40, w: 360, h: 120, r: 60 }, DARK_HULL).expect("place");
            SetWindowPos(hwnd, HWND_TOPMOST, wx, wy, ww, wh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let end = Instant::now() + Duration::from_millis(1500);
            let mut msg: MSG = std::mem::zeroed();
            while Instant::now() < end {
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let out = std::env::var("ATOLL_PROBE_OUT").unwrap_or_else(|_| {
                std::env::temp_dir().join("atoll-blur-probe.bmp").to_string_lossy().into_owned()
            });
            capture(wx, wy, ww, wh, &out);
            ShowWindow(hwnd, SW_HIDE);
            println!("capture: {out}");
        }
    }

    /// Shows Liquid glass at the top left of the main screen in a few looks
    /// and saves captures (before + each look). Run by hand:
    /// `cargo test --lib probe_liquid_glass -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_liquid_glass() {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, HWND_TOPMOST, PM_REMOVE};
        unsafe fn pump(ms: u64) {
            let end = Instant::now() + Duration::from_millis(ms);
            let mut msg: MSG = std::mem::zeroed();
            while Instant::now() < end {
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let dir = std::env::var("ATOLL_PROBE_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
            let hwnd = create_window().expect("window");
            let mut glass = Glass::new(hwnd).expect("compositor");
            glass.mode(true).expect("liquid renderer");
            let window = (40, 40, 520, 240);
            let shape = Shape { x: 40, y: 40, w: 440, h: 160, r: 80 };
            let look = |blur: f32, distortion: f32, liquid: f32| Look {
                shape: [40.0, 40.0, 440.0, 160.0, 80.0],
                blur,
                distortion,
                liquid,
                tint: DARK_HULL,
                tint_alpha: LIQUID_TINT,
            };
            let l = glass.liquid.as_ref().unwrap();
            capture(window.0, window.1, window.2, window.3, &dir.join("atoll-liquid-0-before.png").to_string_lossy());
            let (monitor, origin) = monitor_of(hwnd);
            l.place(window, shape, look(0.0, 1.0, 1.0)).expect("place");
            l.renderer.capture(monitor, origin).expect("screen capture");
            let end = Instant::now() + Duration::from_secs(3);
            while !l.renderer.has_frame() && Instant::now() < end {
                pump(20);
            }
            assert!(l.renderer.has_frame(), "no captured frame arrived");
            l.renderer.freeze();
            SetWindowPos(hwnd, HWND_TOPMOST, window.0, window.1, window.2, window.3, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            for (name, b, d, q) in [("1-clear", 0.0, 1.0, 1.0), ("2-default", 24.0, 0.6, 0.6), ("3-flat", 8.0, 0.0, 0.0)] {
                l.place(window, shape, look(b, d, q)).expect("place");
                pump(400);
                let out = dir.join(format!("atoll-liquid-{name}.png"));
                capture(window.0, window.1, window.2, window.3, &out.to_string_lossy());
                println!("capture: {}", out.display());
            }
            ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// Runs the glass through a series of island moves at the top left of the
    /// main screen, driven exactly as in the app (one frame per screen refresh,
    /// each drawing the shape shape.rs gives for that moment), and prints the
    /// frame times for Frosted and for Liquid. Run by hand:
    /// `cargo test --lib probe_glass_follows_shape -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_glass_follows_shape() {
        use crate::shape::{self, Motion, Segment, Shape as S};
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, HWND_TOPMOST, PM_REMOVE};
        unsafe fn pump() {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // The same retarget rule as spring.ts: keep position, cap speed towards the target.
        fn retarget(seg: &Segment, to: S, omega: f64, t: f64) -> Segment {
            let (pos, mut vel) = shape::evaluate(seg, t);
            for (gap, v) in [(to.w - pos.w, &mut vel.w), (to.h - pos.h, &mut vel.h), (to.r - pos.r, &mut vel.r)] {
                let limit = omega * gap.abs();
                if gap >= 0.0 && *v > limit { *v = limit } else if gap < 0.0 && *v < -limit { *v = -limit }
            }
            Segment { t0: t, from: pos, velocity: vel, to, omega }
        }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let hwnd = create_window().expect("window");
            let mut glass = Glass::new(hwnd).expect("compositor");
            let window = (40, 40, 832, 528);
            SetWindowPos(hwnd, HWND_TOPMOST, window.0, window.1, window.2, window.3, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let idle = S { w: 72.0, h: 40.0, r: 20.0 };
            let status = S { w: 336.0, h: 40.0, r: 20.0 };
            let peek = S { w: 440.0, h: 120.0, r: 28.0 };
            let panel = S { w: 720.0, h: 440.0, r: 32.0 };
            // (target, stiffness 1/ms, how long before the next target: some are cut short on purpose)
            let moves = [
                (status, 0.030, 450.0), (idle, 0.018, 600.0), (panel, 0.028, 500.0), (idle, 0.034, 450.0),
                (peek, 0.028, 450.0), (idle, 0.034, 400.0), (status, 0.030, 60.0), (panel, 0.028, 90.0),
                (idle, 0.034, 120.0), (status, 0.030, 500.0), (idle, 0.018, 700.0),
            ];
            let half = refresh_ms() / 2.0;
            println!("refresh: {:.2} ms", refresh_ms());
            for liquid in [false, true] {
                glass.mode(liquid).expect("mode");
                if liquid {
                    let l = glass.liquid.as_ref().unwrap();
                    let (monitor, origin) = monitor_of(hwnd);
                    l.place(window, Shape { x: 380, y: 8, w: 72, h: 40, r: 20 }, Look::default()).expect("place");
                    l.renderer.capture(monitor, origin).expect("screen capture");
                    let end = Instant::now() + Duration::from_secs(3);
                    while !l.renderer.has_frame() && Instant::now() < end {
                        pump();
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    assert!(l.renderer.has_frame(), "no captured frame arrived");
                    // Keep one frame and stop capturing: the probe's glass is not hidden from capture.
                    l.renderer.freeze();
                }
                let mut seg = Segment { t0: shape::now_ms(), from: idle, velocity: S::default(), to: idle, omega: 0.02 };
                let mut deltas: Vec<f64> = Vec::new();
                let mut bleed: f64 = 0.0;
                for (to, omega, hold) in moves {
                    let start = shape::now_ms();
                    seg = retarget(&seg, to, omega, start);
                    shape::set_for_test(Some(Motion { segment: seg, cx: 416.0, top: 8.0, hover_min: 128.0 }));
                    let mut last: Option<f64> = None;
                    loop {
                        let t = shape::now_ms();
                        if t - start > hold || !shape::moving(t) {
                            break;
                        }
                        let r = shape::glass_rect(t, half).unwrap();
                        let exact = shape::evaluate(&seg, t).0;
                        bleed = bleed.max(r.w - exact.w).max(r.h - exact.h);
                        let s = to_physical(r.x, r.y, r.w, r.h, r.r, 1.0);
                        let presented = if liquid {
                            let look = Look {
                                shape: [s.x as f32, s.y as f32, s.w as f32, s.h as f32, s.r as f32],
                                blur: 24.0,
                                distortion: 0.6,
                                liquid: 0.6,
                                tint: DARK_HULL,
                                tint_alpha: LIQUID_TINT,
                            };
                            glass.liquid.as_ref().unwrap().place(window, s, look).expect("place")
                        } else {
                            glass.place(s, DARK_HULL).expect("place");
                            false
                        };
                        if !presented {
                            wait_for_refresh();
                        }
                        pump();
                        let done = shape::now_ms();
                        if let Some(l) = last {
                            deltas.push(done - l);
                        }
                        last = Some(done);
                    }
                    // Wait out the rest of the hold at rest (no frames drawn: nothing loops).
                    while shape::now_ms() - start < hold {
                        pump();
                        std::thread::sleep(Duration::from_millis(4));
                    }
                }
                let st = shape::frame_stats(&deltas);
                println!(
                    "{}: {} frames, budget {} ms, p50 {} p95 {} max {} ms, dropped {}, glass past the shape's edge by at most {:.3} px",
                    if liquid { "liquid" } else { "frosted" },
                    st.count, st.budget_ms, st.p50, st.p95, st.max, st.dropped, bleed
                );
            }
            shape::set_for_test(None);
            ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// Save a screen rectangle as a 32-bit BMP (or PNG, by the extension).
    unsafe fn capture(x: i32, y: i32, w: i32, h: i32, path: &str) {
        use windows_sys::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
            SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
        };
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        if path.ends_with(".png") {
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            image::save_buffer(path, &pixels, w as u32, h as u32, image::ColorType::Rgba8).expect("write capture");
            return;
        }
        let mut file = Vec::with_capacity(54 + pixels.len());
        let size = (54 + pixels.len()) as u32;
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&[0, 0, 0, 0]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&w.to_le_bytes());
        file.extend_from_slice(&(-h).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0u8; 24]);
        file.extend_from_slice(&pixels);
        std::fs::write(path, file).expect("write capture");
    }
}
