//! Shared helpers for the clipboard, whiteboard and file tray: saving
//! images, thumbnails, ids, small JSON stores, and dragging files out of
//! the island into other apps.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::winclip;

/// Longest side of a thumbnail, in pixels.
const THUMB: u32 = 360;
/// Image files bigger than this are not opened for thumbnails.
const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;

pub fn now_ms() -> u64 {
    crate::sessions::now_ms()
}

/// Short random id (also used in file names).
pub fn new_id() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("OS random number generator unavailable");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hash_of(parts: &[&[u8]]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

/// Saved image: full PNG + thumbnail + size.
pub struct Saved {
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// Save a clipboard image as `<dir>/<id>.png` (+ `<id>.thumb.png`).
pub fn save_image(img: &winclip::Image, dir: &Path, id: &str) -> Result<Saved, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{id}.png"));
    let decoded = match img {
        winclip::Image::Png(bytes) => {
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            image::load_from_memory(bytes).map_err(|e| e.to_string())?
        }
        winclip::Image::Rgba { width, height, pixels } => {
            let buf = image::RgbaImage::from_raw(*width, *height, pixels.clone()).ok_or("bad image size")?;
            let img = image::DynamicImage::ImageRgba8(buf);
            img.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
            img
        }
    };
    let thumb = dir.join(format!("{id}.thumb.png"));
    shrink(&decoded).save_with_format(&thumb, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(Saved { path, thumb, width: decoded.width(), height: decoded.height() })
}

pub fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}

/// Copy an image file into `dir` as PNG (+ thumbnail). Used when an image
/// file is dropped on the whiteboard, so the board keeps working if the
/// original moves.
pub fn import_image_file(src: &Path, dir: &Path, id: &str) -> Result<Saved, String> {
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    if size > MAX_IMAGE_BYTES {
        return Err("image is too large".into());
    }
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    let png = if bytes.starts_with(b"\x89PNG") {
        bytes
    } else {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        out.into_inner()
    };
    save_image(&winclip::Image::Png(png), dir, id)
}

/// Cached thumbnail for any image file (file tray), in `Crest\thumbs`.
pub fn file_thumbnail(path: &Path) -> Option<PathBuf> {
    if !is_image_path(path) {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let key = hash_of(&[path.to_string_lossy().as_bytes(), &modified.to_le_bytes()]);
    let dir = crate::paths::local_dir().join("thumbs");
    let thumb = dir.join(format!("{key:016x}.png"));
    if thumb.exists() {
        return Some(thumb);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let img = image::open(path).ok()?;
    shrink(&img).save_with_format(&thumb, image::ImageFormat::Png).ok()?;
    Some(thumb)
}

/// Read a whole image file back as clipboard content.
pub fn image_content(png_path: &Path) -> Result<winclip::Content, String> {
    // Encrypted clipboard images (`.png.enc`) are decrypted in memory.
    let bytes = crate::crypt::read(png_path).map_err(|_| "The image file is missing.".to_string())?;
    Ok(winclip::Content::Image(winclip::Image::Png(bytes)))
}

/// Load a JSON file, or the default when it is missing or broken.
pub fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("[crest] {} unreadable ({e}); starting empty", path.display());
            let _ = std::fs::rename(path, path.with_extension("json.broken"));
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Atomic JSON write (temp file + rename).
pub fn save_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Write `text` to a temporary .txt file.
#[allow(dead_code)]
pub fn temp_text_file(text: &str) -> Result<PathBuf, String> {
    text_file_in(&crate::paths::local_dir().join("tmp"), text)
}

/// Write `text` to `<dir>/<first few words>.txt`.
pub fn text_file_in(dir: &Path, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem: String = text
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .take(40)
        .collect();
    let stem = if stem.trim().is_empty() { "note".to_string() } else { stem.trim().to_string() };
    let path = dir.join(format!("{stem}.txt"));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Fallback drag picture.
const DRAG_ICON: &[u8] = include_bytes!("../icons/32x32.png");

/// Start a native drag of `files` out of the island (other apps receive
/// ordinary files, like a drag from Explorer). Must run on the main thread,
/// so it is scheduled there; the island page is told when it ends.
pub fn drag_files(app: &AppHandle, files: Vec<PathBuf>, preview: Option<PathBuf>) -> Result<(), String> {
    let files: Vec<PathBuf> = files.into_iter().filter(|f| f.exists()).collect();
    if files.is_empty() {
        return Err("Nothing to drag: the file no longer exists.".into());
    }
    let win = app.get_webview_window(crate::windows::ISLAND).ok_or("island window missing")?;
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let image = match preview.as_ref().and_then(|p| std::fs::read(p).ok()) {
            Some(bytes) => drag::Image::Raw(bytes),
            None => drag::Image::Raw(DRAG_ICON.to_vec()),
        };
        let done = handle.clone();
        crate::dnd::set_own_drag(true);
        let result = drag::start_drag(
            &win,
            drag::DragItem::Files(files),
            image,
            move |result, _| {
                use tauri::Emitter;
                crate::dnd::set_own_drag(false);
                let _ = done.emit_to(crate::windows::ISLAND, "drag-out-ended", matches!(result, drag::DragResult::Dropped));
            },
            drag::Options::default(),
        );
        if let Err(e) = result {
            use tauri::Emitter;
            crate::dnd::set_own_drag(false);
            eprintln!("[crest] drag out failed: {e}");
            let _ = handle.emit_to(crate::windows::ISLAND, "drag-out-ended", false);
        }
    })
    .map_err(|e| e.to_string())
}

/// Thumbnail no larger than THUMB on its longest side (small images stay as they are).
fn shrink(img: &image::DynamicImage) -> image::DynamicImage {
    if img.width() > THUMB || img.height() > THUMB {
        img.thumbnail(THUMB, THUMB)
    } else {
        img.clone()
    }
}
