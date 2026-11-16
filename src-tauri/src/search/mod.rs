//! Universal search and the command palette (owned by `search-clipboard`).
//!
//! - `index`:   the in-memory index (pure, tested, benchmarked).
//! - `sources`: each module's data as documents (read-only).
//! - `agent`:   agent history from the event store (stub until it lands).
//! - `palette`: the palette window (Ctrl+Alt+K by default).
//!
//! Indexing is event-driven: a worker thread sleeps on a channel until a
//! module announces a change ("clipboard", "board", "tray", "note",
//! "calendar-agenda", "recorder:changed"), waits 250 ms for the burst to
//! settle, then re-reads just that source. No timers run while idle.

pub mod agent;
pub mod index;
pub mod palette;
pub mod sources;

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Listener, Manager};

use index::{Doc, Index, Source};

#[derive(Default)]
pub struct SearchState {
    index: Mutex<Index>,
    /// Newest agent event already indexed (ms).
    agent_since: AtomicI64,
}

static DIRTY: OnceLock<Mutex<mpsc::Sender<Source>>> = OnceLock::new();

fn mark(source: Source) {
    if let Some(tx) = DIRTY.get() {
        let _ = tx.lock().unwrap().send(source);
    }
}

/// Re-read one source into the index.
fn refresh(app: &AppHandle, source: Source) {
    let state = app.state::<SearchState>();
    if !sources::enabled(app, source) {
        state.index.lock().unwrap().clear_source(source);
        return;
    }
    if source == Source::Agent {
        // Incremental: only events newer than the last one seen.
        let since = state.agent_since.load(Ordering::Relaxed);
        let docs = sources::agent(app, since);
        if let Some(max) = docs.iter().map(|d| d.at as i64).max() {
            state.agent_since.store(max, Ordering::Relaxed);
        }
        let mut ix = state.index.lock().unwrap();
        for d in docs {
            ix.upsert(d);
        }
        return;
    }
    let docs: Vec<Doc> = match source {
        Source::Clipboard => sources::clipboard(app),
        Source::Whiteboard => sources::whiteboard(app),
        Source::Note => sources::note(app),
        Source::FileTray => sources::file_tray(app),
        Source::Calendar => sources::calendar(app),
        Source::Agent => unreachable!(),
    };
    state.index.lock().unwrap().replace_source(source, docs);
}

/// Build the index and start listening for changes. Call once at start-up.
pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Source>();
    if DIRTY.set(Mutex::new(tx)).is_err() {
        return;
    }
    for (event, source) in [
        ("clipboard", Source::Clipboard),
        ("board", Source::Whiteboard),
        ("tray", Source::FileTray),
        ("note", Source::Note),
        ("calendar-agenda", Source::Calendar),
        (agent::CHANGED_EVENT, Source::Agent),
    ] {
        app.listen_any(event, move |_| mark(source));
    }
    let worker = app.clone();
    std::thread::Builder::new()
        .name("atoll-search".into())
        .spawn(move || {
            for s in Source::ALL {
                refresh(&worker, s);
            }
            // Block until something changes; then gather the burst.
            while let Ok(first) = rx.recv() {
                let mut due: HashSet<Source> = HashSet::from([first]);
                while let Ok(s) = rx.recv_timeout(Duration::from_millis(250)) {
                    due.insert(s);
                }
                for s in due {
                    refresh(&worker, s);
                }
            }
        })
        .expect("search worker");
}

/// Settings changed: a module may have been switched on or off.
pub fn settings_changed() {
    for s in Source::ALL {
        mark(s);
    }
}

// ---------- results and actions ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Result {
    #[serde(flatten)]
    pub hit: index::Hit,
    /// Available actions; the first is what Enter does.
    pub actions: Vec<&'static str>,
}

fn actions_for(d: &Doc) -> Vec<&'static str> {
    match (d.source, d.kind.as_str()) {
        (Source::Clipboard, "files") => vec!["paste", "copy", "reveal"],
        (Source::Clipboard, _) => vec!["paste", "copy"],
        (Source::Whiteboard, "text") => vec!["paste", "copy"],
        (Source::Whiteboard, _) => vec!["paste", "open", "reveal", "copy"],
        (Source::Note, _) => vec!["open", "paste", "copy"],
        (Source::FileTray, _) => vec!["open", "reveal", "paste", "copy"],
        (Source::Calendar, _) => vec!["open", "copy"],
        (Source::Agent, _) => vec!["open", "copy"],
    }
}

pub fn query(app: &AppHandle, text: &str, limit: usize, only: Option<Source>) -> Vec<Result> {
    let now = crate::media::now_ms();
    let hits = app.state::<SearchState>().index.lock().unwrap().search(text, limit.clamp(1, 200), only, now);
    hits.into_iter().map(|hit| Result { actions: actions_for(&hit.doc), hit }).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub documents: usize,
    pub bytes: usize,
}

pub fn stats(app: &AppHandle) -> Stats {
    let ix = app.state::<SearchState>();
    let ix = ix.index.lock().unwrap();
    Stats { documents: ix.len(), bytes: ix.bytes() }
}

fn doc_of(app: &AppHandle, key: &str) -> std::result::Result<Doc, String> {
    app.state::<SearchState>().index.lock().unwrap().get(key).ok_or_else(|| "That item is gone.".into())
}

/// Run an action on a result. The palette closes first so pasting reaches
/// the app you were in.
pub fn act(app: &AppHandle, key: &str, action: &str) -> std::result::Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let doc = doc_of(app, key)?;
    let id = key.split_once(':').map(|(_, id)| id).unwrap_or("");
    palette::hide(app);
    let paste = action == "paste";
    match (doc.source, action) {
        (Source::Clipboard, "paste" | "copy") => crate::clipboard::use_item(app, id, paste),
        (Source::Clipboard, "reveal") => match crate::clipboard::content_of(app, id)? {
            crate::winclip::Content::Files(f) if !f.is_empty() => app.opener().reveal_item_in_dir(&f[0]).map_err(|e| e.to_string()),
            _ => Err("That isn't a file.".into()),
        },
        (Source::Whiteboard, "paste" | "copy") => crate::board::use_card(app, id, paste),
        (Source::Whiteboard, "open") => crate::board::open(app, id),
        (Source::Whiteboard, "reveal") => crate::board::reveal(app, id),
        (Source::Note, "open") => {
            crate::windows::open_tab(app, "note");
            Ok(())
        }
        (Source::Note, "paste" | "copy") => {
            crate::winclip::write(&crate::winclip::Content::Text(doc.body.clone()))?;
            if paste { crate::paste::paste_into_previous() } else { Ok(()) }
        }
        (Source::FileTray, "open") => crate::tray::open(app, id),
        (Source::FileTray, "reveal") => crate::tray::reveal(app, id),
        (Source::FileTray, "copy") => crate::tray::copy(app, vec![id.to_string()]),
        (Source::FileTray, "paste") => crate::tray::paste(app, vec![id.to_string()]),
        (Source::Calendar, "open") => {
            let ev = crate::calendar::agenda(app).events.into_iter().find(|e| e.id == id).ok_or("That event is gone.")?;
            let url = ev.meet.or(ev.link).ok_or("This event has no link.")?;
            app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
        }
        (Source::Calendar | Source::Agent, "copy") => {
            let text = if doc.sub.is_empty() { doc.title.clone() } else { format!("{} ({})", doc.title, doc.sub.split(" · ").skip(1).collect::<Vec<_>>().join(" · ")) };
            crate::winclip::write(&crate::winclip::Content::Text(text))
        }
        (Source::Agent, "open") => {
            crate::windows::open_tab(app, "claude");
            Ok(())
        }
        _ => Err("That action isn't available here.".into()),
    }
}

// ---------- settings ----------

/// One-time defaults for search, kept out of the shared settings version
/// number so parallel branches don't collide: adds the palette hotkey
/// (Ctrl+Alt+K) unless that combination is already yours.
pub fn seed_settings(s: &mut crate::settings::Settings) {
    if s.search.seeded {
        return;
    }
    s.search.seeded = true;
    let taken = s.hotkeys.modules.values().chain([&s.hotkeys.toggle_island, &s.hotkeys.expand_island, &s.hotkeys.open_settings]).any(|v| v.eq_ignore_ascii_case(palette::DEFAULT_HOTKEY));
    if !taken {
        s.hotkeys.modules.entry("palette".into()).or_insert_with(|| palette::DEFAULT_HOTKEY.into());
    }
}
