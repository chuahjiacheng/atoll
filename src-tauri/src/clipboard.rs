//! Clipboard history.
//!
//! A hidden message-only window registers with `AddClipboardFormatListener`
//! and gets WM_CLIPBOARDUPDATE on every copy. Content marked private by the
//! copying app (password managers) is never read into history. Text, file
//! lists and images are kept; images are saved as PNG in
//! `%LOCALAPPDATA%\Crest\clipboard\`. Unpinned items are wiped on start
//! unless "keep after restart" is on.
//!
//! The same listener feeds "add every copy to the whiteboard".

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::media::{self, hash_of, new_id, now_ms};
use crate::winclip::{self, Content};
use crate::AppState;

/// Longest text kept (characters). Longer copies are skipped.
const MAX_TEXT: usize = 1_000_000;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    /// "text" | "image" | "files"
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    /// Full PNG (images).
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// App that copied it, e.g. "chrome.exe" (when Windows tells us).
    #[serde(default)]
    pub source: Option<String>,
    pub at: u64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub hash: u64,
    /// "link" | "colour" | "json" | "code" (clipintel.rs).
    #[serde(default)]
    pub types: Vec<String>,
    /// Transforms offered for this item ("jsonPretty", "jsonMinify", "stripTracking").
    #[serde(default)]
    pub transforms: Vec<String>,
    /// What secret the text holds ("GitHub token"); `text` is then masked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// The real text of a secret item, DPAPI-encrypted, base64. Never sent to the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
    /// When a secret item is forgotten (ms). `None` = kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Text found in an image (secrets masked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    items: Vec<Clip>,
}

#[derive(Default)]
pub struct ClipboardState {
    items: Mutex<Vec<Clip>>,
}

fn dir() -> PathBuf {
    crate::paths::local_dir().join("clipboard")
}

fn file() -> PathBuf {
    dir().join("history.json")
}

/// History when "Encrypt history" is on (DPAPI).
fn sealed_file() -> PathBuf {
    dir().join("history.json.enc")
}

/// "Skip password manager entries" (read by the listener thread).
static SKIP_PRIVATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Every item as stored, sealed secrets included (Rust side only).
fn all(app: &AppHandle) -> Vec<Clip> {
    app.state::<ClipboardState>().items.lock().unwrap().clone()
}

/// Items for the page and other modules: secrets stay masked.
pub fn list(app: &AppHandle) -> Vec<Clip> {
    all(app)
        .into_iter()
        .map(|mut c| {
            c.sealed = None;
            c
        })
        .collect()
}

fn seal(text: &str) -> Result<String, String> {
    use base64::Engine;
    crate::crypt::protect(text.as_bytes()).map(|b| base64::engine::general_purpose::STANDARD.encode(b))
}

/// The real text of an item (unseals secrets).
fn real_text(c: &Clip) -> Result<String, String> {
    use base64::Engine;
    match &c.sealed {
        Some(s) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| e.to_string())?;
            String::from_utf8(crate::crypt::unprotect(&bytes)?).map_err(|e| e.to_string())
        }
        None => Ok(c.text.clone().unwrap_or_default()),
    }
}

fn emit(app: &AppHandle) {
    let _ = app.emit("clipboard", list(app));
}

/// Save to disk. Without "keep after restart", only pinned items are written.
fn save(app: &AppHandle) {
    let c = app.state::<AppState>().settings.lock().unwrap().clipboard.clone();
    let items: Vec<Clip> = all(app).into_iter().filter(|x| c.persist || x.pinned).collect();
    let result = if c.encrypt {
        serde_json::to_vec(&Stored { items })
            .map_err(|e| e.to_string())
            .and_then(|json| crate::crypt::protect(&json))
            .and_then(|sealed| std::fs::create_dir_all(dir()).and_then(|_| std::fs::write(sealed_file(), sealed)).map_err(|e| e.to_string()))
            .map(|_| {
                let _ = std::fs::remove_file(file());
            })
    } else {
        media::save_json(&file(), &Stored { items }).map(|_| {
            let _ = std::fs::remove_file(sealed_file());
        })
    };
    if let Err(e) = result {
        eprintln!("[atoll] could not save clipboard history: {e}");
    }
}

fn remove_files(c: &Clip) {
    for p in [&c.image, &c.thumb].into_iter().flatten() {
        let _ = std::fs::remove_file(p);
    }
}

/// Load history at start-up, and delete image files nothing points at.
fn load(app: &AppHandle) {
    let c = app.state::<AppState>().settings.lock().unwrap().clipboard.clone();
    SKIP_PRIVATE.store(c.skip_private, std::sync::atomic::Ordering::Relaxed);
    let persist = c.persist;
    let stored: Stored = if sealed_file().exists() {
        crate::crypt::read(&sealed_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    } else {
        media::load_json(&file())
    };
    let now = now_ms();
    let items: Vec<Clip> = stored
        .items
        .into_iter()
        .filter(|c| persist || c.pinned)
        .filter(|c| c.pinned || c.expires_at.is_none_or(|t| t > now))
        .collect();
    for c in &items {
        if let (Some(at), false) = (c.expires_at, c.pinned) {
            schedule_expiry(app, c.id.clone(), at);
        }
    }
    let keep: std::collections::HashSet<String> =
        items.iter().flat_map(|c| [c.image.clone(), c.thumb.clone()]).flatten().collect();
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for e in entries.flatten() {
            let p = e.path();
            let image = p.extension().is_some_and(|x| x == "png" || x == crate::crypt::EXT) && p != sealed_file();
            if image && !keep.contains(&p.to_string_lossy().to_string()) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    *app.state::<ClipboardState>().items.lock().unwrap() = items;
}

/// Put a new copy at the top (or move an identical older one up).
/// Returns the item now at the top (an older identical copy keeps its id).
fn record(app: &AppHandle, mut clip: Clip) -> Clip {
    let max = app.state::<AppState>().settings.lock().unwrap().clipboard.max_items as usize;
    let top = {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        if let Some(i) = items.iter().position(|c| c.hash == clip.hash && c.kind == clip.kind) {
            let old = items.remove(i);
            remove_files(&clip); // keep the old files, drop the new duplicates
            // A fresh copy of a secret restarts its expiry.
            let expires_at = if old.pinned { None } else { clip.expires_at.or(old.expires_at) };
            clip = Clip { at: clip.at, source: clip.source.or(old.source.clone()), expires_at, ..old };
        }
        let top = clip.clone();
        items.insert(0, clip);
        // Trim the oldest unpinned items beyond the limit.
        while items.len() > max {
            let Some(i) = items.iter().rposition(|c| !c.pinned) else { break };
            remove_files(&items.remove(i));
        }
        top
    };
    save(app);
    emit(app);
    if let Some(at) = top.expires_at {
        schedule_expiry(app, top.id.clone(), at);
    }
    top
}

// ---------- secrets expire ----------

/// Forget a secret item at `at` (ms), unless it was pinned or copied again since.
fn schedule_expiry(app: &AppHandle, id: String, at: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(at.saturating_sub(now_ms()))).await;
        expire(&app, &id);
    });
}

fn expire(app: &AppHandle, id: &str) {
    let gone = {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        let due = |c: &Clip| c.id == id && !c.pinned && c.expires_at.is_some_and(|t| t <= now_ms() + 100);
        items.iter().position(due).map(|i| items.remove(i))
    };
    let Some(clip) = gone else { return };
    remove_files(&clip);
    // Also take it off the Windows clipboard if it is still there.
    if let Some(Content::Text(t)) = winclip::read() {
        if hash_of(&[t.as_bytes()]) == clip.hash {
            let _ = winclip::clear();
        }
    }
    save(app);
    emit(app);
}

// ---------- text in images ----------

/// Read the text in an image item on a short-lived thread, then store it.
fn recognise_later(app: &AppHandle, clip: &Clip) {
    let (Some(path), None) = (clip.image.clone(), clip.ocr.as_ref()) else { return };
    let (app, id, detect) = (app.clone(), clip.id.clone(), app.state::<AppState>().settings.lock().unwrap().clipboard.detect_secrets);
    std::thread::spawn(move || {
        let Ok(bytes) = crate::crypt::read(std::path::Path::new(&path)) else { return };
        let Some(text) = crate::ocr::recognise(&bytes) else { return };
        let text = if detect { crate::clipintel::find_secret(&text).map(|s| s.masked).unwrap_or(text) } else { text };
        let found = {
            let state = app.state::<ClipboardState>();
            let mut items = state.items.lock().unwrap();
            items.iter_mut().find(|c| c.id == id).map(|c| c.ocr = Some(text)).is_some()
        };
        if found {
            save(&app);
            emit(&app);
        }
    });
}

/// Run a transform on an item and put the result on the clipboard (the
/// stored item is unchanged). `ocrText` copies the text found in an image.
pub fn transform(app: &AppHandle, id: &str, op: &str, paste: bool) -> Result<(), String> {
    let clip = all(app).into_iter().find(|c| c.id == id).ok_or("That item is gone.")?;
    let out = if op == "ocrText" {
        clip.ocr.clone().ok_or("No text was found in this image.")?
    } else {
        crate::clipintel::transform(&real_text(&clip)?, op)?
    };
    winclip::write(&Content::Text(out))?;
    if paste { crate::paste::paste_into_previous() } else { Ok(()) }
}

/// Build a history entry from clipboard content (saves images to disk,
/// encrypted when "Encrypt history" is on).
fn to_clip(content: &Content, source: Option<String>, s: &crate::settings::ClipboardSettings) -> Option<Clip> {
    let encrypt = s.encrypt;
    let id = new_id();
    let base = Clip {
        id: id.clone(),
        kind: String::new(),
        text: None,
        files: vec![],
        image: None,
        thumb: None,
        width: 0,
        height: 0,
        source,
        at: now_ms(),
        pinned: false,
        hash: 0,
        types: vec![],
        transforms: vec![],
        secret: None,
        sealed: None,
        expires_at: None,
        ocr: None,
    };
    match content {
        Content::Text(t) if t.chars().count() <= MAX_TEXT => {
            let a = crate::clipintel::analyse(t, s.detect_secrets);
            let hash = hash_of(&[t.as_bytes()]);
            let base = Clip { kind: "text".into(), hash, types: a.types, transforms: a.transforms, ..base };
            match a.secret {
                // Never keep a secret we could not encrypt.
                Some(secret) => Some(Clip {
                    text: Some(secret.masked),
                    sealed: Some(seal(t).map_err(|e| eprintln!("[atoll] secret not kept: {e}")).ok()?),
                    secret: Some(secret.label),
                    expires_at: (s.secret_expiry_min > 0).then(|| now_ms() + s.secret_expiry_min as u64 * 60_000),
                    ..base
                }),
                None => Some(Clip { text: Some(t.clone()), ..base }),
            }
        }
        Content::Text(_) => None,
        Content::Files(files) => {
            Some(Clip { kind: "files".into(), hash: hash_of(&[files.join("\n").as_bytes()]), files: files.clone(), ..base })
        }
        Content::Image(img) => {
            let saved = media::save_image(img, &dir(), &id).map_err(|e| eprintln!("[atoll] clipboard image not saved: {e}")).ok()?;
            let bytes = std::fs::read(&saved.path).unwrap_or_default();
            let (image, thumb) = if encrypt {
                (crate::crypt::encrypt_file(&saved.path).ok()?, crate::crypt::encrypt_file(&saved.thumb).ok()?)
            } else {
                (saved.path, saved.thumb)
            };
            Some(Clip {
                kind: "image".into(),
                hash: hash_of(&[&bytes]),
                image: Some(image.to_string_lossy().into()),
                thumb: Some(thumb.to_string_lossy().into()),
                width: saved.width,
                height: saved.height,
                ..base
            })
        }
    }
}

/// Called on the worker thread for every outside copy.
fn on_copy(app: &AppHandle, content: Content, source: Option<String>) {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let keep_kind = match &content {
        Content::Text(_) => true,
        Content::Files(_) => s.clipboard.files,
        Content::Image(_) => s.clipboard.images,
    };
    let mut secret = false;
    if s.modules.clipboard && keep_kind && !s.clipboard.paused {
        if let Some(clip) = to_clip(&content, source, &s.clipboard) {
            secret = clip.secret.is_some();
            let top = record(app, clip);
            if top.kind == "image" && s.clipboard.ocr {
                recognise_later(app, &top);
            }
        }
    } else if let (Content::Text(t), true) = (&content, s.clipboard.detect_secrets) {
        secret = crate::clipintel::find_secret(t).is_some();
    }
    // Secrets never go onto the whiteboard, where they would sit unmasked.
    if s.modules.whiteboard && s.whiteboard.auto_add_copies && !secret {
        crate::board::add_content(app, &content, None);
    }
}

/// Something dropped on the clipboard tab: put it on the clipboard and at the
/// top of the history (even while recording is paused: you asked for it).
pub fn add(app: &AppHandle, content: &Content) -> Result<(), String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clipboard.clone();
    let clip = to_clip(content, Some("Dropped on Atoll".into()), &s).ok_or("That is too big to keep in the clipboard history.")?;
    winclip::write(content)?;
    let top = record(app, clip);
    if top.kind == "image" && s.ocr {
        recognise_later(app, &top);
    }
    Ok(())
}

/// A history item as clipboard content (secrets unsealed).
pub fn content_of(app: &AppHandle, id: &str) -> Result<Content, String> {
    let clip = all(app).into_iter().find(|c| c.id == id).ok_or("That item is gone.")?;
    Ok(match clip.kind.as_str() {
        "text" => Content::Text(real_text(&clip)?),
        "files" => Content::Files(clip.files),
        _ => media::image_content(std::path::Path::new(clip.image.as_deref().unwrap_or("")))?,
    })
}

/// Drag a history item out of the island: text as text, files as files,
/// images as a PNG file.
pub fn drag(app: &AppHandle, id: &str) -> Result<(), String> {
    let clip = all(app).into_iter().find(|c| c.id == id).ok_or("That item is gone.")?;
    match clip.kind.as_str() {
        "text" => crate::dnd::drag_text(app, real_text(&clip)?),
        "files" => media::drag_files(app, clip.files.iter().map(PathBuf::from).collect(), None),
        _ => {
            let image = std::path::Path::new(clip.image.as_deref().unwrap_or(""));
            let file = crate::dnd::plain_copy(image, &format!("Image {}x{}.png", clip.width, clip.height))?;
            let preview = clip.thumb.as_deref().and_then(|t| crate::dnd::plain_copy(std::path::Path::new(t), "drag-preview.png").ok());
            media::drag_files(app, vec![file], preview)
        }
    }
}

/// Copy a history item back to the clipboard (moves it to the top).
pub fn use_item(app: &AppHandle, id: &str, paste: bool) -> Result<(), String> {
    let clip = all(app).into_iter().find(|c| c.id == id).ok_or("That item is gone.")?;
    let content = match clip.kind.as_str() {
        "text" => Content::Text(real_text(&clip)?),
        "files" => Content::Files(clip.files.clone()),
        _ => media::image_content(std::path::Path::new(clip.image.as_deref().unwrap_or("")))?,
    };
    match (&content, clip.secret.is_some()) {
        (Content::Text(t), true) => winclip::write_secret(t)?,
        _ => winclip::write(&content)?,
    }
    {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        if let Some(i) = items.iter().position(|c| c.id == id) {
            let mut c = items.remove(i);
            c.at = now_ms();
            items.insert(0, c);
        }
    }
    save(app);
    emit(app);
    if paste { crate::paste::paste_into_previous() } else { Ok(()) }
}

/// Pinning a secret keeps it; unpinning starts its expiry again.
pub fn set_pinned(app: &AppHandle, id: &str, pinned: bool) {
    let minutes = app.state::<AppState>().settings.lock().unwrap().clipboard.secret_expiry_min as u64;
    let mut restart = None;
    if let Some(c) = app.state::<ClipboardState>().items.lock().unwrap().iter_mut().find(|c| c.id == id) {
        c.pinned = pinned;
        if c.secret.is_some() {
            c.expires_at = (!pinned && minutes > 0).then(|| now_ms() + minutes * 60_000);
            restart = c.expires_at;
        }
    }
    save(app);
    emit(app);
    if let Some(at) = restart {
        schedule_expiry(app, id.to_string(), at);
    }
}

pub fn delete(app: &AppHandle, id: &str) {
    {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        if let Some(i) = items.iter().position(|c| c.id == id) {
            remove_files(&items.remove(i));
        }
    }
    save(app);
    emit(app);
}

/// Remove everything except pinned items.
pub fn clear(app: &AppHandle) {
    {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        items.retain(|c| {
            if !c.pinned {
                remove_files(c);
            }
            c.pinned
        });
    }
    save(app);
    emit(app);
}

/// Remove everything, pinned items too (Privacy → Clear all data).
pub fn clear_everything(app: &AppHandle) {
    {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        items.iter().for_each(remove_files);
        items.clear();
    }
    let _ = std::fs::remove_file(file());
    let _ = std::fs::remove_file(sealed_file());
    emit(app);
}

/// A thumbnail as a data URL (encrypted thumbnails can't be served as files).
pub fn thumb_data(app: &AppHandle, id: &str) -> Result<String, String> {
    use base64::Engine;
    let clip = all(app).into_iter().find(|c| c.id == id).ok_or("That item is gone.")?;
    let path = clip.thumb.ok_or("No picture.")?;
    let bytes = crate::crypt::read(std::path::Path::new(&path))?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Clipboard settings changed: apply "skip private", (de)crypt stored
/// images when "Encrypt history" flips, and re-save history in the right form.
pub fn settings_changed(app: &AppHandle) {
    let c = app.state::<AppState>().settings.lock().unwrap().clipboard.clone();
    SKIP_PRIVATE.store(c.skip_private, std::sync::atomic::Ordering::Relaxed);
    {
        let state = app.state::<ClipboardState>();
        let mut items = state.items.lock().unwrap();
        for clip in items.iter_mut() {
            for slot in [&mut clip.image, &mut clip.thumb] {
                let Some(p) = slot.clone() else { continue };
                let path = std::path::Path::new(&p);
                let converted = match (c.encrypt, crate::crypt::is_encrypted(path)) {
                    (true, false) => crate::crypt::encrypt_file(path).ok(),
                    (false, true) => crate::crypt::decrypt_file(path).ok(),
                    _ => None,
                };
                if let Some(np) = converted {
                    *slot = Some(np.to_string_lossy().into());
                }
            }
        }
    }
    save(app);
    emit(app);
}

// ---------- the listener ----------

static SENDER: OnceLock<Mutex<mpsc::Sender<(Content, Option<String>)>>> = OnceLock::new();
static LAST_SOURCE: Mutex<Option<String>> = Mutex::new(None);

unsafe extern "system" fn wndproc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::System::DataExchange::GetClipboardOwner;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DefWindowProcW, KillTimer, SetTimer, WM_CLIPBOARDUPDATE, WM_TIMER};
    const READ_TIMER: usize = 1;
    if msg == WM_CLIPBOARDUPDATE {
        // Don't read at once: many apps are still finishing their copy (OLE
        // delayed rendering + flush) and would fail if we held the clipboard
        // now. Wait a moment; several quick updates collapse into one read.
        // Note who copied it now: the copying app may have exited by the time we read.
        *LAST_SOURCE.lock().unwrap() = crate::chatwatch::process_name(unsafe { GetClipboardOwner() });
        unsafe { SetTimer(hwnd, READ_TIMER, 250, None) };
        return 0;
    }
    if msg == WM_TIMER && wparam == READ_TIMER {
        unsafe { KillTimer(hwnd, READ_TIMER) };
        if !winclip::is_own_content() {
            if let Some(content) = winclip::read_with(SKIP_PRIVATE.load(std::sync::atomic::Ordering::Relaxed)) {
                let source = LAST_SOURCE.lock().unwrap().take();
                if let Some(tx) = SENDER.get() {
                    let _ = tx.lock().unwrap().send((content, source));
                }
            }
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Load history and start listening for copies.
pub fn start(app: AppHandle) {
    load(&app);
    let (tx, rx) = mpsc::channel::<(Content, Option<String>)>();
    let _ = SENDER.set(Mutex::new(tx));
    let worker = app.clone();
    std::thread::spawn(move || {
        for (content, source) in rx {
            on_copy(&worker, content, source);
        }
    });
    std::thread::spawn(|| unsafe {
        use windows_sys::Win32::System::DataExchange::AddClipboardFormatListener;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG, WNDCLASSW,
        };
        let class = wide("CrestClipboardWindow");
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null());
        if hwnd.is_null() {
            eprintln!("[crest] clipboard: could not create listener window");
            return;
        }
        winclip::set_owner(hwnd);
        if AddClipboardFormatListener(hwnd) == 0 {
            eprintln!("[crest] clipboard: AddClipboardFormatListener failed");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
