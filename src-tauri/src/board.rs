//! Whiteboard: a freeform canvas of text, image and file cards inside the
//! expanded island. Saved to `%LOCALAPPDATA%\Crest\board\board.json` after
//! every change, so it survives restarts.
//!
//! - Images (pasted, copied or dropped image files) are copied into
//!   `board\assets` so the card keeps working if the original moves.
//! - Other files are kept as a link to the original path (Crest never copies
//!   or moves your files); the card shows "missing" if the file is gone.
//! - Clicking a card copies it (and optionally pastes it); dragging a card
//!   out of the island drops a real file into the other app.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::media::{self, new_id, now_ms};
use crate::winclip::{self, Content};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    /// "text" | "image" | "file"
    pub kind: String,
    /// Canvas position and card width (CSS px).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    #[serde(default)]
    pub text: Option<String>,
    /// Image: PNG in board\assets. File: the original path.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// File cards: does the file still exist? (refreshed on every load)
    #[serde(default)]
    pub missing: bool,
    pub at: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub cards: Vec<Card>,
}

#[derive(Default)]
pub struct BoardState {
    board: Mutex<Option<Board>>,
}

fn dir() -> PathBuf {
    crate::paths::local_dir().join("board")
}

fn assets() -> PathBuf {
    dir().join("assets")
}

fn file() -> PathBuf {
    dir().join("board.json")
}

fn with_board<R>(app: &AppHandle, f: impl FnOnce(&mut Board) -> R) -> R {
    let state = app.state::<BoardState>();
    let mut guard = state.board.lock().unwrap();
    let board = guard.get_or_insert_with(|| media::load_json(&file()));
    f(board)
}

pub fn get(app: &AppHandle) -> Board {
    with_board(app, |b| {
        for c in b.cards.iter_mut().filter(|c| c.kind == "file") {
            c.missing = !c.path.as_deref().is_some_and(|p| Path::new(p).exists());
        }
        b.clone()
    })
}

/// Save and tell the island.
fn changed(app: &AppHandle) {
    let board = get(app);
    if let Err(e) = media::save_json(&file(), &board) {
        eprintln!("[crest] could not save the whiteboard: {e}");
    }
    let _ = app.emit("board", board);
}

/// Where a new card goes when no drop position is given: a gentle cascade.
fn next_spot(b: &Board) -> (f64, f64) {
    let n = b.cards.len() as f64;
    (16.0 + (n % 8.0) * 22.0, 14.0 + (n % 8.0) * 18.0)
}

fn card(kind: &str, x: f64, y: f64, w: f64) -> Card {
    Card { id: new_id(), kind: kind.into(), x, y, w, text: None, path: None, thumb: None, name: None, missing: false, at: now_ms() }
}

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

/// Build cards for some content. Positions are offset so several items don't stack exactly.
fn cards_for(content: &Content, x: f64, y: f64) -> Vec<Card> {
    match content {
        Content::Text(t) => vec![Card { text: Some(t.clone()), ..card("text", x, y, 180.0) }],
        Content::Image(img) => {
            let id = new_id();
            match media::save_image(img, &assets(), &id) {
                Ok(saved) => vec![Card {
                    id,
                    path: Some(saved.path.to_string_lossy().into()),
                    thumb: Some(saved.thumb.to_string_lossy().into()),
                    ..card("image", x, y, 160.0)
                }],
                Err(e) => {
                    eprintln!("[crest] board image not saved: {e}");
                    vec![]
                }
            }
        }
        Content::Files(files) => files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| file_card(f, x + i as f64 * 16.0, y + i as f64 * 14.0))
            .collect(),
    }
}

/// An image file becomes an image card (copied in); anything else a file link.
fn file_card(path: &str, x: f64, y: f64) -> Option<Card> {
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    if p.is_file() && media::is_image_path(p) {
        let id = new_id();
        if let Ok(saved) = media::import_image_file(p, &assets(), &id) {
            return Some(Card {
                id,
                name: Some(file_name(path)),
                path: Some(saved.path.to_string_lossy().into()),
                thumb: Some(saved.thumb.to_string_lossy().into()),
                ..card("image", x, y, 160.0)
            });
        }
    }
    Some(Card { name: Some(file_name(path)), path: Some(path.to_string()), ..card("file", x, y, 170.0) })
}

/// Add content (clipboard copy, Paste button) at `at` or the next free spot.
pub fn add_content(app: &AppHandle, content: &Content, at: Option<(f64, f64)>) {
    let (x, y) = at.unwrap_or_else(|| with_board(app, |b| next_spot(b)));
    let new = cards_for(content, x, y);
    if new.is_empty() {
        return;
    }
    with_board(app, |b| b.cards.extend(new));
    changed(app);
}

pub fn add_files(app: &AppHandle, paths: Vec<String>, at: Option<(f64, f64)>) {
    add_content(app, &Content::Files(paths), at);
}

pub fn add_text(app: &AppHandle, text: String, x: f64, y: f64) -> Card {
    let c = Card { text: Some(text), ..card("text", x, y, 180.0) };
    with_board(app, |b| b.cards.push(c.clone()));
    changed(app);
    c
}

pub fn add_from_clipboard(app: &AppHandle, at: Option<(f64, f64)>) -> Result<(), String> {
    let content = winclip::read().ok_or("The clipboard is empty (or holds something private).")?;
    add_content(app, &content, at);
    Ok(())
}

#[derive(Deserialize)]
pub struct CardPatch {
    x: Option<f64>,
    y: Option<f64>,
    w: Option<f64>,
    text: Option<String>,
    /// Move the card to the top of the stack.
    raise: Option<bool>,
}

pub fn update(app: &AppHandle, id: &str, patch: CardPatch) {
    with_board(app, |b| {
        let Some(i) = b.cards.iter().position(|c| c.id == id) else { return };
        let c = &mut b.cards[i];
        if let Some(x) = patch.x {
            c.x = x.clamp(-4000.0, 4000.0);
        }
        if let Some(y) = patch.y {
            c.y = y.clamp(-4000.0, 4000.0);
        }
        if let Some(w) = patch.w {
            c.w = w.clamp(90.0, 520.0);
        }
        if let Some(t) = patch.text {
            c.text = Some(t);
        }
        if patch.raise == Some(true) {
            let c = b.cards.remove(i);
            b.cards.push(c);
        }
    });
    changed(app);
}

fn remove_assets(c: &Card) {
    if c.kind == "image" {
        for p in [&c.path, &c.thumb].into_iter().flatten() {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn remove(app: &AppHandle, id: &str) {
    with_board(app, |b| {
        if let Some(i) = b.cards.iter().position(|c| c.id == id) {
            remove_assets(&b.cards.remove(i));
        }
    });
    changed(app);
}

pub fn clear(app: &AppHandle) {
    with_board(app, |b| {
        b.cards.iter().for_each(remove_assets);
        b.cards.clear();
    });
    changed(app);
}

fn find(app: &AppHandle, id: &str) -> Result<Card, String> {
    get(app).cards.into_iter().find(|c| c.id == id).ok_or_else(|| "That card is gone.".to_string())
}

/// Copy a card to the clipboard, and paste it into the previous app if asked.
pub fn use_card(app: &AppHandle, id: &str, paste: bool) -> Result<(), String> {
    let c = find(app, id)?;
    let content = match c.kind.as_str() {
        "text" => Content::Text(c.text.unwrap_or_default()),
        "image" => media::image_content(Path::new(c.path.as_deref().unwrap_or("")))?,
        _ => {
            if c.missing {
                return Err("That file has been moved or deleted.".into());
            }
            Content::Files(vec![c.path.unwrap_or_default()])
        }
    };
    winclip::write(&content)?;
    if paste { crate::paste::paste_into_previous() } else { Ok(()) }
}

/// Drag a card out of the island: text cards as plain text (drops into any
/// text box), images and files as real files.
pub fn drag(app: &AppHandle, id: &str) -> Result<(), String> {
    let c = find(app, id)?;
    let (file, preview) = match c.kind.as_str() {
        "text" => return crate::dnd::drag_text(app, c.text.unwrap_or_default()),
        "image" => (PathBuf::from(c.path.unwrap_or_default()), c.thumb.map(PathBuf::from)),
        _ => (PathBuf::from(c.path.unwrap_or_default()), None),
    };
    media::drag_files(app, vec![file], preview)
}

/// Show a file card (or an image card's saved PNG) in Explorer.
pub fn reveal(app: &AppHandle, id: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let c = find(app, id)?;
    let path = c.path.ok_or("This card has no file.")?;
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

/// Keep a card in the file tray (text and images become files first).
pub fn to_tray(app: &AppHandle, id: &str) -> Result<(), String> {
    let c = find(app, id)?;
    let content = match c.kind.as_str() {
        "text" => Content::Text(c.text.unwrap_or_default()),
        _ if c.missing => return Err("That file has been moved or deleted.".into()),
        _ => Content::Files(vec![c.path.unwrap_or_default()]),
    };
    crate::tray::add(app, crate::dnd::as_files(&content)?);
    Ok(())
}

/// Copy a card into the clipboard history (and onto the clipboard).
pub fn to_clipboard(app: &AppHandle, id: &str) -> Result<(), String> {
    let c = find(app, id)?;
    let content = match c.kind.as_str() {
        "text" => Content::Text(c.text.unwrap_or_default()),
        "image" => media::image_content(Path::new(c.path.as_deref().unwrap_or("")))?,
        _ => Content::Files(vec![c.path.unwrap_or_default()]),
    };
    crate::clipboard::add(app, &content)
}

/// Copy of a card, placed a little below and to the right.
pub fn duplicate(app: &AppHandle, id: &str) -> Result<(), String> {
    let c = find(app, id)?;
    let mut copy = Card { id: new_id(), x: c.x + 20.0, y: c.y + 20.0, at: now_ms(), ..c.clone() };
    if c.kind == "image" {
        let saved = media::import_image_file(Path::new(c.path.as_deref().unwrap_or("")), &assets(), &copy.id)?;
        copy.path = Some(saved.path.to_string_lossy().into());
        copy.thumb = Some(saved.thumb.to_string_lossy().into());
    }
    with_board(app, |b| b.cards.push(copy));
    changed(app);
    Ok(())
}

/// Open a file card with its default app.
pub fn open(app: &AppHandle, id: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let c = find(app, id)?;
    let path = c.path.ok_or("Nothing to open.")?;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}
