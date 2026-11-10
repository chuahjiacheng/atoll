//! Text in copied images, using Windows' built-in OCR (Windows.Media.Ocr).
//! Owned by the `search-clipboard` branch.
//!
//! Runs on its own short-lived thread, never on the clipboard listener, so a
//! slow recognition never delays the history. Uses the recognisers for the
//! languages in your Windows profile (on this PC: en-GB, en-US, zh-Hans-CN).

use windows::core::Result as WinResult;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

/// Longest text kept from one image.
const MAX_CHARS: usize = 20_000;

/// Recognise text in an encoded image (PNG/JPEG/...). `None` if there is no
/// text, OCR is unavailable, or the image can't be decoded.
pub fn recognise(encoded: &[u8]) -> Option<String> {
    let img = image::load_from_memory(encoded).ok()?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(10_000);
    let img = if img.width() > max || img.height() > max { img.resize(max, max, image::imageops::FilterType::Triangle) } else { img };
    // Tiny images (icons) never hold readable text.
    if img.width() < 24 || img.height() < 12 {
        return None;
    }
    let gray = img.to_luma8();
    match run(gray.width(), gray.height(), gray.as_raw()) {
        Ok(t) => {
            let t = t.trim();
            (!t.is_empty()).then(|| t.chars().take(MAX_CHARS).collect())
        }
        Err(e) => {
            eprintln!("[atoll] ocr: {e}");
            None
        }
    }
}

fn run(width: u32, height: u32, gray: &[u8]) -> WinResult<String> {
    ensure_winrt();
    let writer = DataWriter::new()?;
    writer.WriteBytes(gray)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, width as i32, height as i32)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    // Join lines with newlines (Text joins them with spaces).
    let mut out = String::new();
    for line in result.Lines()? {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line.Text()?.to_string_lossy());
    }
    Ok(out)
}

/// WinRT needs the calling thread in an apartment; harmless if it already is.
fn ensure_winrt() {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

/// Languages Windows can recognise here, e.g. ["en-GB", "en-US"].
pub fn languages() -> Vec<String> {
    ensure_winrt();
    OcrEngine::AvailableRecognizerLanguages()
        .map(|langs| langs.into_iter().filter_map(|l| l.LanguageTag().ok()).map(|t| t.to_string_lossy()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probe: draws nothing on screen. Renders text with GDI into a bitmap,
    /// then checks Windows OCR reads it back from an unpackaged process.
    #[test]
    #[ignore]
    fn probe_ocr_reads_rendered_text() {
        let langs = languages();
        println!("OCR languages: {langs:?}");
        assert!(!langs.is_empty(), "no OCR languages installed");
        let png = render_text_png("Atoll clipboard 2026");
        let t0 = std::time::Instant::now();
        let text = recognise(&png).expect("no text recognised");
        println!("recognised {:?} in {:?}", text, t0.elapsed());
        assert!(text.to_lowercase().contains("clipboard"), "{text}");
    }

    /// Black text on white, drawn with GDI into a memory DC.
    fn render_text_png(text: &str) -> Vec<u8> {
        use windows_sys::Win32::Graphics::Gdi::*;
        let (w, h) = (640i32, 120i32);
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            SelectObject(dc, bmp);
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            px.fill(255);
            let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
            let font = CreateFontW(56, 0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 5, 0, face.as_ptr());
            SelectObject(dc, font);
            SetBkMode(dc, 1);
            SetTextColor(dc, 0);
            let wide: Vec<u16> = text.encode_utf16().collect();
            TextOutW(dc, 20, 30, wide.as_ptr(), wide.len() as i32);
            GdiFlush();
            let rgba: Vec<u8> = px.chunks(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            DeleteObject(font);
            DeleteObject(bmp);
            DeleteDC(dc);
            let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap();
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        }
    }
}
