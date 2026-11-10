//! File tray: files you drop on the island to keep them handy, and the
//! quick note.
//!
//! The tray only remembers paths (`%LOCALAPPDATA%\Crest\tray.json`); your
//! files are never copied, moved or deleted. Removing an item only removes
//! it from the tray.
//!
//! The quick note is one plain-text file, `%LOCALAPPDATA%\Crest\note.txt`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::media::{self, new_id, now_ms};
use crate::winclip::{self, Content};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrayFile {
    pub id: String,
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub missing: bool,
    pub at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    files: Vec<TrayFile>,
}

#[derive(Default)]
pub struct TrayState {
    files: Mutex<Option<Vec<TrayFile>>>,
}

fn file() -> PathBuf {
    crate::paths::local_dir().join("tray.json")
}

fn note_file() -> PathBuf {
    crate::paths::local_dir().join("note.txt")
}

fn with_files<R>(app: &AppHandle, f: impl FnOnce(&mut Vec<TrayFile>) -> R) -> R {
    let state = app.state::<TrayState>();
    let mut guard = state.files.lock().unwrap();
    let files = guard.get_or_insert_with(|| media::load_json::<Stored>(&file()).files);
    f(files)
}

pub fn list(app: &AppHandle) -> Vec<TrayFile> {
    with_files(app, |files| {
        for f in files.iter_mut() {
            f.missing = !Path::new(&f.path).exists();
        }
        files.clone()
    })
}

fn changed(app: &AppHandle) {
    let files = list(app);
    if let Err(e) = media::save_json(&file(), &Stored { files: files.clone() }) {
        eprintln!("[crest] could not save the file tray: {e}");
    }
    let _ = app.emit("tray", files);
}

pub fn add(app: &AppHandle, paths: Vec<String>) {
    let new: Vec<TrayFile> = paths
        .into_iter()
        .filter_map(|p| {
            let path = Path::new(&p);
            let meta = std::fs::metadata(path).ok()?;
            Some(TrayFile {
                id: new_id(),
                name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone()),
                size: if meta.is_file() { meta.len() } else { 0 },
                is_dir: meta.is_dir(),
                thumb: media::file_thumbnail(path).map(|t| t.to_string_lossy().to_string()),
                missing: false,
                at: now_ms(),
                path: p,
            })
        })
        .collect();
    with_files(app, |files| {
        for f in new {
            // Dropping the same file again just moves it to the top.
            files.retain(|old| !old.path.eq_ignore_ascii_case(&f.path));
            files.insert(0, f);
        }
    });
    changed(app);
}

pub fn remove(app: &AppHandle, id: &str) {
    with_files(app, |files| files.retain(|f| f.id != id));
    changed(app);
}

pub fn clear(app: &AppHandle) {
    with_files(app, |files| files.clear());
    changed(app);
}

fn find(app: &AppHandle, id: &str) -> Result<TrayFile, String> {
    let f = list(app).into_iter().find(|f| f.id == id).ok_or("That file is no longer in the tray.")?;
    if f.missing {
        return Err("That file has been moved or deleted.".into());
    }
    Ok(f)
}

pub fn open(app: &AppHandle, id: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let f = find(app, id)?;
    app.opener().open_path(f.path, None::<&str>).map_err(|e| e.to_string())
}

/// Show the file selected in Explorer.
pub fn reveal(app: &AppHandle, id: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let f = find(app, id)?;
    app.opener().reveal_item_in_dir(f.path).map_err(|e| e.to_string())
}

/// Put the file(s) on the clipboard, like Ctrl+C in Explorer.
pub fn copy(app: &AppHandle, ids: Vec<String>) -> Result<(), String> {
    let paths: Vec<String> = list(app).into_iter().filter(|f| ids.contains(&f.id) && !f.missing).map(|f| f.path).collect();
    if paths.is_empty() {
        return Err("Nothing to copy.".into());
    }
    winclip::write(&Content::Files(paths))
}

/// Copy the file(s), then paste them into the app you were using.
pub fn paste(app: &AppHandle, ids: Vec<String>) -> Result<(), String> {
    copy(app, ids)?;
    crate::paste::paste_into_previous()
}

/// Put the file(s) on the whiteboard too.
pub fn to_board(app: &AppHandle, ids: Vec<String>) {
    let paths: Vec<String> = list(app).into_iter().filter(|f| ids.contains(&f.id) && !f.missing).map(|f| f.path).collect();
    crate::board::add_files(app, paths, None);
}

/// Add what's on the clipboard (copied files, or text / an image saved as a file).
pub fn add_clipboard(app: &AppHandle) -> Result<(), String> {
    let content = winclip::read().ok_or("The clipboard is empty (or holds something private).")?;
    add(app, crate::dnd::as_files(&content)?);
    Ok(())
}

pub fn drag(app: &AppHandle, ids: Vec<String>) -> Result<(), String> {
    let files = list(app);
    let picked: Vec<&TrayFile> = files.iter().filter(|f| ids.contains(&f.id) && !f.missing).collect();
    let preview = picked.first().and_then(|f| f.thumb.clone()).map(PathBuf::from);
    media::drag_files(app, picked.iter().map(|f| PathBuf::from(&f.path)).collect(), preview)
}

pub fn note_get() -> String {
    std::fs::read_to_string(note_file()).unwrap_or_default()
}

pub fn note_set(app: &AppHandle, text: &str) -> Result<(), String> {
    let path = note_file();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    let _ = app.emit("note", text);
    Ok(())
}
