//! Native drag and drop for the island window.
//!
//! **In.** wry's own drop target only takes files dragged from Explorer.
//! [`install`] swaps it, on the island's WebView2 child windows, for one that
//! also takes text, images and the "virtual files" that browsers, Outlook and
//! chat apps hand out (FileGroupDescriptorW + FileContents, e.g. an image
//! dragged out of Chrome). The page gets `drop-in` events
//! (`enter`/`over`/`leave`/`drop`, client position in physical px) and, on
//! drop, picks the destination and calls [`commit`].
//!
//! **Out.** [`drag_text`] drags text out as plain text (CF_UNICODETEXT), so it
//! lands in any text box like a selection dragged from a browser. Files and
//! images still go out through `media::drag_files`.
//!
//! Formats: https://learn.microsoft.com/windows/win32/shell/clipboard

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use windows::core::{implement, Ref, BOOL, HRESULT};
use windows::Win32::Foundation::{DATA_S_SAMEFORMATETC, DV_E_FORMATETC, E_NOTIMPL, HWND, LPARAM, OLE_E_ADVISENOTSUPPORTED, POINT, POINTL, S_OK};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    IDropSource, IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_DIB, CF_HDROP, CF_UNICODETEXT,
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, SHCreateStdEnumFmtEtc, SHDoDragDrop, HDROP};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

use crate::media::new_id;
use crate::winclip::{Content, Image};
use crate::windows::ISLAND;

/// Biggest virtual file accepted from a drop (bytes).
const MAX_VIRTUAL: usize = 512 * 1024 * 1024;

/// True while Atoll itself is dragging something out, so the island doesn't
/// offer to take its own drag back.
static OWN: AtomicBool = AtomicBool::new(false);
/// What was dropped last, until the page says where it goes.
static PENDING: Mutex<Option<Content>> = Mutex::new(None);

pub fn set_own_drag(on: bool) {
    OWN.store(on, Ordering::Relaxed);
}

thread_local! {
    // Keeps our targets alive (OLE holds its own reference too).
    static TARGETS: RefCell<Vec<IDropTarget>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Serialize)]
struct DropEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    x: f64,
    y: f64,
    /// "files" | "text" | "image" (enter/drop only).
    what: Option<&'static str>,
}

fn format(name: &str) -> u16 {
    let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(w.as_ptr())) as u16 }
}

fn etc(cf: u16, lindex: i32, tymed: u32) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex, tymed }
}

fn has(obj: &IDataObject, cf: u16, tymed: u32) -> bool {
    unsafe { obj.QueryGetData(&etc(cf, -1, tymed)) }.is_ok()
}

/// The bytes of one format (HGLOBAL or IStream).
fn bytes(obj: &IDataObject, cf: u16, lindex: i32) -> Option<Vec<u8>> {
    unsafe {
        let mut m = obj.GetData(&etc(cf, lindex, (TYMED_HGLOBAL.0 | TYMED_ISTREAM.0) as u32)).ok()?;
        let out = if m.tymed == TYMED_HGLOBAL.0 as u32 {
            let h = m.u.hGlobal;
            let size = GlobalSize(h);
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                None
            } else {
                let v = std::slice::from_raw_parts(p, size).to_vec();
                let _ = GlobalUnlock(h);
                Some(v)
            }
        } else if m.tymed == TYMED_ISTREAM.0 as u32 {
            (*m.u.pstm).as_ref().map(|s| {
                let mut out = Vec::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let mut read = 0u32;
                    let hr = s.Read(buf.as_mut_ptr() as _, buf.len() as u32, Some(&mut read));
                    if hr.is_err() || read == 0 || out.len() > MAX_VIRTUAL {
                        break;
                    }
                    out.extend_from_slice(&buf[..read as usize]);
                }
                out
            })
        } else {
            None
        };
        ReleaseStgMedium(&mut m);
        out
    }
}

fn hdrop_files(obj: &IDataObject) -> Option<Vec<String>> {
    unsafe {
        let mut m = obj.GetData(&etc(CF_HDROP.0, -1, TYMED_HGLOBAL.0 as u32)).ok()?;
        let hdrop = HDROP(m.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        let mut files = Vec::new();
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            files.push(String::from_utf16_lossy(&buf[..len]));
        }
        ReleaseStgMedium(&mut m);
        (!files.is_empty()).then_some(files)
    }
}

/// One entry of a FILEGROUPDESCRIPTORW (name, is folder, size if known).
pub(crate) fn parse_descriptors(b: &[u8]) -> Vec<(String, bool, Option<u64>)> {
    const SIZE: usize = 592; // sizeof(FILEDESCRIPTORW)
    let rd = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if b.len() < 4 {
        return vec![];
    }
    let n = rd(0) as usize;
    (0..n)
        .filter_map(|i| {
            let o = 4 + i * SIZE;
            if o + SIZE > b.len() {
                return None;
            }
            let flags = rd(o);
            let attrs = rd(o + 36);
            let size = (flags & 0x40 != 0).then(|| ((rd(o + 64) as u64) << 32) | rd(o + 68) as u64);
            let name: Vec<u16> = b[o + 72..o + SIZE].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            Some((String::from_utf16_lossy(&name), attrs & 0x10 != 0, size))
        })
        .collect()
}

/// A name from a virtual file, made safe to use as a path under our folder.
pub(crate) fn safe_relative(name: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for part in name.split(['\\', '/']) {
        let clean: String = part.chars().map(|c| if "<>:\"|?*".contains(c) || (c as u32) < 32 { '_' } else { c }).collect();
        let clean = clean.trim().trim_end_matches('.').to_string();
        if clean.is_empty() || clean == "." || clean == ".." {
            continue;
        }
        p.push(clean);
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// Folder for things dropped on the island that had no file of their own.
pub fn dropped_dir() -> PathBuf {
    crate::paths::local_dir().join("dropped").join(new_id())
}

/// Write the virtual files out under `%LOCALAPPDATA%\Crest\dropped\<id>\`.
fn virtual_files(obj: &IDataObject, descriptors: &[(String, bool, Option<u64>)]) -> Option<Vec<String>> {
    let dir = dropped_dir();
    let contents = format("FileContents");
    let mut top = Vec::new();
    for (i, (name, is_dir, size)) in descriptors.iter().enumerate() {
        let Some(rel) = safe_relative(name) else { continue };
        let path = dir.join(&rel);
        if *is_dir {
            let _ = std::fs::create_dir_all(&path);
        } else {
            let Some(mut data) = bytes(obj, contents, i as i32) else { continue };
            if let Some(s) = size {
                data.truncate(*s as usize);
            }
            if std::fs::create_dir_all(path.parent().unwrap()).is_err() || std::fs::write(&path, data).is_err() {
                continue;
            }
        }
        // Only the top-level items go to the island (folders bring their contents).
        let first = dir.join(rel.components().next().unwrap());
        let s = first.to_string_lossy().to_string();
        if !top.contains(&s) {
            top.push(s);
        }
    }
    (!top.is_empty()).then_some(top)
}

fn text(obj: &IDataObject) -> Option<String> {
    let b = bytes(obj, CF_UNICODETEXT.0, -1)?;
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let t = String::from_utf16_lossy(&w);
    (!t.trim().is_empty()).then_some(t)
}

/// What kind of thing is being dragged, if the island can take it.
/// Files are checked by reading them: some data objects (the shell's own)
/// answer "yes" to QueryGetData for file formats they don't actually have.
fn what(obj: &IDataObject) -> Option<&'static str> {
    let hg = TYMED_HGLOBAL.0 as u32;
    let any = (TYMED_HGLOBAL.0 | TYMED_ISTREAM.0) as u32;
    if has(obj, CF_HDROP.0, hg) && hdrop_files(obj).is_some() {
        return Some("files");
    }
    let descriptors = bytes(obj, format("FileGroupDescriptorW"), -1).map(|b| parse_descriptors(&b)).unwrap_or_default();
    if !descriptors.is_empty() {
        // A dragged link offers a .url "file" too; call that text.
        let links = descriptors.iter().all(|(n, _, _)| n.to_lowercase().ends_with(".url"));
        return Some(if links && has(obj, CF_UNICODETEXT.0, hg) { "text" } else { "files" });
    }
    if (has(obj, format("PNG"), any) && bytes(obj, format("PNG"), -1).is_some_and(|b| !b.is_empty()))
        || (has(obj, CF_DIB.0, hg) && bytes(obj, CF_DIB.0, -1).is_some_and(|b| b.len() > 40))
    {
        return Some("image");
    }
    if has(obj, CF_UNICODETEXT.0, hg) && text(obj).is_some() {
        return Some("text");
    }
    None
}

/// Read what was dropped.
fn read(obj: &IDataObject) -> Option<Content> {
    match what(obj)? {
        "files" => {
            if let Some(f) = hdrop_files(obj) {
                return Some(Content::Files(f));
            }
            let d = parse_descriptors(&bytes(obj, format("FileGroupDescriptorW"), -1)?);
            virtual_files(obj, &d).map(Content::Files)
        }
        "image" => {
            if let Some(png) = bytes(obj, format("PNG"), -1) {
                return Some(Content::Image(Image::Png(png)));
            }
            let (width, height, pixels) = crate::winclip::dib_to_rgba(&bytes(obj, CF_DIB.0, -1)?)?;
            Some(Content::Image(Image::Rgba { width, height, pixels }))
        }
        _ => text(obj).map(Content::Text),
    }
}

#[implement(IDropTarget)]
struct Target {
    app: AppHandle,
    hwnd: HWND,
    effect: Cell<DROPEFFECT>,
}

impl Target {
    fn emit(&self, kind: &'static str, pt: Option<&POINTL>, what: Option<&'static str>) {
        let mut p = POINT { x: pt.map_or(0, |p| p.x), y: pt.map_or(0, |p| p.y) };
        if pt.is_some() {
            let _ = unsafe { ScreenToClient(self.hwnd, &mut p) };
        }
        let _ = self.app.emit_to(ISLAND, "drop-in", DropEvent { kind, x: p.x as f64, y: p.y as f64, what });
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let kind = if OWN.load(Ordering::Relaxed) { None } else { data.as_ref().and_then(what) };
        let e = if kind.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        self.effect.set(e);
        unsafe { *effect = e };
        if kind.is_some() {
            self.emit("enter", Some(pt), kind);
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        unsafe { *effect = self.effect.get() };
        if self.effect.get() != DROPEFFECT_NONE {
            self.emit("over", Some(pt), None);
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.effect.get() != DROPEFFECT_NONE {
            self.emit("leave", None, None);
        }
        self.effect.set(DROPEFFECT_NONE);
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let accepted = self.effect.replace(DROPEFFECT_NONE) != DROPEFFECT_NONE;
        let content = if accepted { data.as_ref().and_then(read) } else { None };
        unsafe { *effect = if content.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        match content {
            Some(c) => {
                let what = match &c {
                    Content::Files(_) => "files",
                    Content::Image(_) => "image",
                    Content::Text(_) => "text",
                };
                *PENDING.lock().unwrap() = Some(c);
                self.emit("drop", Some(pt), Some(what));
            }
            None if accepted => self.emit("leave", None, None),
            None => {}
        }
        Ok(())
    }
}

/// Put our drop target on the island's WebView2 windows (replacing wry's).
pub fn install(app: &AppHandle) {
    let Some(win) = app.get_webview_window(ISLAND) else { return };
    let Ok(hwnd) = win.hwnd() else { return };
    let raw = hwnd.0 as isize;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let hwnd = HWND(raw as _);
        let mut children: Vec<HWND> = Vec::new();
        unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
            unsafe { (*(l.0 as *mut Vec<HWND>)).push(h) };
            true.into()
        }
        let _ = unsafe { EnumChildWindows(Some(hwnd), Some(collect), LPARAM(&mut children as *mut _ as isize)) };
        let mut n = 0;
        for child in children {
            let target: IDropTarget = Target { app: handle.clone(), hwnd: child, effect: Cell::new(DROPEFFECT_NONE) }.into();
            unsafe {
                let _ = RevokeDragDrop(child);
                if RegisterDragDrop(child, &target).is_ok() {
                    n += 1;
                    TARGETS.with(|t| t.borrow_mut().push(target));
                }
            }
        }
        println!("[atoll] drop target on {n} island window(s)");
    });
}

/// Hand the last drop to `target`: "whiteboard", "fileTray" or "clipboard".
pub fn commit(app: &AppHandle, target: &str, at: Option<(f64, f64)>) -> Result<(), String> {
    let content = PENDING.lock().unwrap().take().ok_or("Nothing was dropped.")?;
    match target {
        "whiteboard" => crate::board::add_content(app, &content, at),
        "fileTray" => crate::tray::add(app, as_files(&content)?),
        "clipboard" => crate::clipboard::add(app, &content)?,
        _ => return Err(format!("Unknown drop target {target}")),
    }
    Ok(())
}

/// Files for some content: text and images are written out as a file first.
pub fn as_files(content: &Content) -> Result<Vec<String>, String> {
    match content {
        Content::Files(f) => Ok(f.clone()),
        Content::Text(t) => {
            let dir = dropped_dir();
            Ok(vec![crate::media::text_file_in(&dir, t)?.to_string_lossy().into()])
        }
        Content::Image(img) => {
            let dir = dropped_dir();
            let saved = crate::media::save_image(img, &dir, "image")?;
            let _ = std::fs::remove_file(&saved.thumb);
            Ok(vec![saved.path.to_string_lossy().into()])
        }
    }
}

/// Drag `text` out of the island as plain text. Runs on the main thread;
/// the page hears `drag-out-ended` when it's over.
pub fn drag_text(app: &AppHandle, text: String) -> Result<(), String> {
    if text.is_empty() {
        return Err("Nothing to drag.".into());
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        set_own_drag(true);
        let result = unsafe { start_text(&text) };
        set_own_drag(false);
        if let Err(e) = &result {
            eprintln!("[atoll] text drag failed: {e}");
        }
        let _ = handle.emit_to(ISLAND, "drag-out-ended", matches!(result, Ok(true)));
    })
    .map_err(|e| e.to_string())
}

unsafe fn start_text(text: &str) -> windows::core::Result<bool> {
    unsafe {
        let obj: IDataObject = TextData { wide: text.encode_utf16().chain([0]).collect() }.into();
        let effect = SHDoDragDrop(None, &obj, None::<&IDropSource>, DROPEFFECT_COPY)?;
        Ok(effect != DROPEFFECT_NONE)
    }
}

/// A data object holding only plain text (CF_UNICODETEXT). The shell's own
/// data object can't be used: it claims file formats it doesn't have, and
/// apps that prefer files would take the drop and get nothing.
#[implement(IDataObject)]
struct TextData {
    wide: Vec<u16>,
}

fn is_text(f: *const FORMATETC) -> bool {
    let f = unsafe { &*f };
    f.cfFormat == CF_UNICODETEXT.0 && f.tymed & TYMED_HGLOBAL.0 as u32 != 0 && f.dwAspect == DVASPECT_CONTENT.0
}

#[allow(non_snake_case)]
impl IDataObject_Impl for TextData_Impl {
    fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        if !is_text(f) {
            return Err(DV_E_FORMATETC.into());
        }
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, self.wide.len() * 2)?;
            std::ptr::copy_nonoverlapping(self.wide.as_ptr(), GlobalLock(h) as *mut u16, self.wide.len());
            let _ = GlobalUnlock(h);
            Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: ManuallyDrop::new(None) })
        }
    }
    fn GetDataHere(&self, _f: *const FORMATETC, _m: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn QueryGetData(&self, f: *const FORMATETC) -> HRESULT {
        if is_text(f) { S_OK } else { DV_E_FORMATETC }
    }
    fn GetCanonicalFormatEtc(&self, _i: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        unsafe { (*out).ptd = std::ptr::null_mut() };
        DATA_S_SAMEFORMATETC
    }
    fn SetData(&self, _f: *const FORMATETC, _m: *const STGMEDIUM, _r: BOOL) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&[etc(CF_UNICODETEXT.0, -1, TYMED_HGLOBAL.0 as u32)]) }
    }
    fn DAdvise(&self, _f: *const FORMATETC, _a: u32, _s: Ref<'_, IAdviseSink>) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _c: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

/// A file (decrypted if needed) that can be dragged out: encrypted clipboard
/// images are written to `%LOCALAPPDATA%\Crest\tmp` first.
pub fn plain_copy(path: &Path, name: &str) -> Result<PathBuf, String> {
    if !crate::crypt::is_encrypted(path) {
        return Ok(path.to_path_buf());
    }
    let bytes = crate::crypt::read(path)?;
    let dir = crate::paths::local_dir().join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(name);
    std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_parsing() {
        let mut b = vec![0u8; 4 + 592];
        b[0] = 1;
        b[4] = 0x40; // FD_FILESIZE
        b[4 + 68] = 5; // size low
        for (i, c) in "cat.png".encode_utf16().enumerate() {
            b[4 + 72 + i * 2..4 + 74 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        assert_eq!(parse_descriptors(&b), vec![("cat.png".to_string(), false, Some(5))]);
    }

    #[test]
    fn unsafe_names_are_cleaned() {
        assert_eq!(safe_relative("..\\..\\evil.txt"), Some(PathBuf::from("evil.txt")));
        assert_eq!(safe_relative("a/b:c?.txt"), Some(PathBuf::from("a").join("b_c_.txt")));
        assert_eq!(safe_relative(".."), None);
    }

    /// A real shell data object carrying text is read back as text.
    #[test]
    fn reads_text_data_object() {
        let obj: IDataObject = TextData { wide: "hello island".encode_utf16().chain([0]).collect() }.into();
        assert_eq!(what(&obj), Some("text"));
        assert!(matches!(read(&obj), Some(Content::Text(t)) if t == "hello island"));
        // Our drag-out object offers text and nothing else.
        assert!(!has(&obj, CF_HDROP.0, TYMED_HGLOBAL.0 as u32));
    }

    /// The shell's empty data object claims CF_HDROP / FileGroupDescriptorW
    /// without having them; that must not count as files.
    #[test]
    fn phantom_file_formats_are_ignored() {
        unsafe {
            let _ = windows::Win32::System::Ole::OleInitialize(None);
            let obj: IDataObject = windows::Win32::UI::Shell::SHCreateDataObject(None, None, None::<&IDataObject>).unwrap();
            assert!(has(&obj, CF_HDROP.0, TYMED_HGLOBAL.0 as u32));
            assert_eq!(what(&obj), None);
        }
    }
}
