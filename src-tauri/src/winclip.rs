//! Small wrappers around the Win32 clipboard: read and write text, file
//! lists and images, and the "please don't record this" markers that
//! password managers set.
//!
//! Formats used (https://learn.microsoft.com/windows/win32/dataxchg/clipboard-formats):
//! - CF_UNICODETEXT for text
//! - CF_HDROP for copied files (Explorer)
//! - the registered "PNG" format (browsers, Snipping Tool) or CF_DIB for images
//! - "ExcludeClipboardContentFromMonitorProcessing", "Clipboard Viewer Ignore"
//!   and "CanIncludeInClipboardHistory" = 0 mark content that must not be kept
//!   (https://learn.microsoft.com/windows/win32/dataxchg/clipboard-formats#cloud-clipboard-and-clipboard-history-formats)

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::DragQueryFileW;

/// Hidden window that owns our clipboard writes (set by the clipboard listener).
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// Clipboard sequence number right after our own last write.
static OWN_SEQ: AtomicU32 = AtomicU32::new(0);

pub fn set_owner(hwnd: HWND) {
    OWNER.store(hwnd as isize, Ordering::Relaxed);
}

/// Was the current clipboard content written by Crest itself?
pub fn is_own_content() -> bool {
    let own = OWN_SEQ.load(Ordering::Relaxed);
    own != 0 && unsafe { GetClipboardSequenceNumber() } == own
}

pub enum Image {
    /// Encoded PNG bytes as found on the clipboard.
    Png(Vec<u8>),
    /// Decoded pixels (from CF_DIB), RGBA, top row first.
    Rgba { width: u32, height: u32, pixels: Vec<u8> },
}

pub enum Content {
    Text(String),
    Files(Vec<String>),
    Image(Image),
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn format(name: &str) -> u32 {
    unsafe { RegisterClipboardFormatW(wide(name).as_ptr()) }
}

/// Opens the clipboard (retrying briefly: another app may hold it) and
/// closes it again when dropped.
struct Open;

impl Open {
    fn new() -> Option<Open> {
        let owner = OWNER.load(Ordering::Relaxed) as HWND;
        for _ in 0..12 {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

/// Copy the bytes of one clipboard format (clipboard must be open).
unsafe fn read_bytes(fmt: u32) -> Option<Vec<u8>> {
    unsafe {
        let handle = GetClipboardData(fmt);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }
}

fn available(fmt: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(fmt) != 0 }
}

/// True when the copying app asked clipboard tools not to record this
/// (password managers do). Must be called with the clipboard open.
unsafe fn excluded() -> bool {
    if available(format("ExcludeClipboardContentFromMonitorProcessing")) || available(format("Clipboard Viewer Ignore")) {
        return true;
    }
    let can = format("CanIncludeInClipboardHistory");
    if available(can) {
        if let Some(b) = unsafe { read_bytes(can) } {
            if b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                return true;
            }
        }
    }
    false
}

/// What is on the clipboard right now: files, then text, then an image.
/// `None` when it is empty, unreadable, or marked as private.
pub fn read() -> Option<Content> {
    read_with(true)
}

/// Like `read`, but `skip_private: false` also reads copies marked private.
pub fn read_with(skip_private: bool) -> Option<Content> {
    let _open = Open::new()?;
    unsafe {
        if skip_private && excluded() {
            return None;
        }
        if available(CF_HDROP as u32) {
            let handle = GetClipboardData(CF_HDROP as u32);
            if !handle.is_null() {
                let count = DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0);
                let mut files = Vec::new();
                for i in 0..count {
                    let len = DragQueryFileW(handle, i, std::ptr::null_mut(), 0) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(handle, i, buf.as_mut_ptr(), buf.len() as u32);
                    files.push(String::from_utf16_lossy(&buf[..len]));
                }
                if !files.is_empty() {
                    return Some(Content::Files(files));
                }
            }
        }
        if available(CF_UNICODETEXT as u32) {
            if let Some(bytes) = read_bytes(CF_UNICODETEXT as u32) {
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
                let text = String::from_utf16_lossy(&units);
                if !text.trim().is_empty() {
                    return Some(Content::Text(text));
                }
            }
        }
        let png = format("PNG");
        if available(png) {
            if let Some(bytes) = read_bytes(png) {
                if bytes.starts_with(b"\x89PNG") {
                    return Some(Content::Image(Image::Png(bytes)));
                }
            }
        }
        if available(CF_DIB as u32) {
            if let Some((width, height, pixels)) = read_bytes(CF_DIB as u32).and_then(|b| dib_to_rgba(&b)) {
                return Some(Content::Image(Image::Rgba { width, height, pixels }));
            }
        }
    }
    None
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Convert a packed DIB (BITMAPINFOHEADER + pixels) with 24 or 32 bits per
/// pixel into RGBA rows, top row first.
pub fn dib_to_rgba(b: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if b.len() < 40 {
        return None;
    }
    let header = u32_at(b, 0) as usize;
    let width = u32_at(b, 4) as i32;
    let height = u32_at(b, 8) as i32;
    let bpp = u16_at(b, 14) as usize;
    let compression = u32_at(b, 16);
    let colors_used = u32_at(b, 32) as usize;
    // BI_RGB = 0, BI_BITFIELDS = 3 (assumed to be the standard BGR(A) masks)
    if width <= 0 || height == 0 || !(bpp == 24 || bpp == 32) || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w > 16_384 || h > 16_384 {
        return None;
    }
    let mut offset = header + colors_used * 4;
    if compression == 3 && header == 40 {
        offset += 12; // three colour masks follow a plain BITMAPINFOHEADER
    }
    let stride = (w * bpp).div_ceil(32) * 4;
    if b.len() < offset + stride * h {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let src_row = if height > 0 { h - 1 - y } else { y };
        let row = &b[offset + src_row * stride..];
        for x in 0..w {
            let (px, o) = (&row[x * bpp / 8..], (y * w + x) * 4);
            out[o] = px[2];
            out[o + 1] = px[1];
            out[o + 2] = px[0];
            out[o + 3] = if bpp == 32 { px[3] } else { 255 };
            any_alpha |= bpp == 32 && px[3] != 0;
        }
    }
    if bpp == 32 && !any_alpha {
        // Most apps leave the alpha byte at 0 for opaque images.
        out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Some((w as u32, h as u32, out))
}

/// Packed bottom-up 32-bit DIB from RGBA pixels (for CF_DIB).
pub fn rgba_to_dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = Vec::with_capacity(40 + w * h * 4);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes()); // positive = bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution + colour counts
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &rgba[(y * w + x) * 4..];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out
}

/// Put bytes on the open clipboard under `fmt`.
unsafe fn put(fmt: u32, data: &[u8]) -> bool {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
        if mem.is_null() {
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            GlobalFree(mem);
            return false;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        GlobalUnlock(mem);
        if SetClipboardData(fmt, mem).is_null() {
            GlobalFree(mem);
            return false;
        }
        true
    }
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// Replace the clipboard with `content`.
pub fn write(content: &Content) -> Result<(), String> {
    let open = Open::new().ok_or("The clipboard is busy. Try again.")?;
    let ok = unsafe {
        EmptyClipboard();
        match content {
            Content::Text(t) => put(CF_UNICODETEXT as u32, &utf16_bytes(t)),
            Content::Files(files) => {
                // DROPFILES { pFiles = 20, pt = (0,0), fNC = 0, fWide = 1 } + double-null list
                let mut data = Vec::new();
                data.extend_from_slice(&20u32.to_le_bytes());
                data.extend_from_slice(&[0u8; 12]);
                data.extend_from_slice(&1u32.to_le_bytes());
                for f in files {
                    data.extend(utf16_bytes(f));
                }
                data.extend_from_slice(&[0, 0]);
                let ok = put(CF_HDROP as u32, &data);
                put(format("Preferred DropEffect"), &1u32.to_le_bytes()); // copy, not move
                ok
            }
            Content::Image(Image::Rgba { width, height, pixels }) => put(CF_DIB as u32, &rgba_to_dib(*width, *height, pixels)),
            Content::Image(Image::Png(bytes)) => {
                let mut ok = put(format("PNG"), bytes);
                if let Ok(img) = image::load_from_memory(bytes) {
                    let rgba = img.to_rgba8();
                    ok |= put(CF_DIB as u32, &rgba_to_dib(rgba.width(), rgba.height(), rgba.as_raw()));
                }
                ok
            }
        }
    };
    drop(open);
    OWN_SEQ.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Relaxed);
    if ok { Ok(()) } else { Err("Could not put that on the clipboard.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_round_trip() {
        // 2x2: red, green / blue, half-transparent white (top row first)
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128];
        let dib = rgba_to_dib(2, 2, &rgba);
        let (w, h, back) = dib_to_rgba(&dib).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(back, rgba);
    }

    #[test]
    fn opaque_dib_without_alpha() {
        let mut dib = rgba_to_dib(1, 1, &[10, 20, 30, 0]);
        dib[40 + 3] = 0;
        let (_, _, back) = dib_to_rgba(&dib).unwrap();
        assert_eq!(back, vec![10, 20, 30, 255]);
    }
}
