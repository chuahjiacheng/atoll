//! Synced lyrics from LRCLIB (https://lrclib.net/docs), a free public lyrics
//! database. Spotify's Web API has no lyrics.
//!
//! Privacy: LRCLIB receives the track name, artist, album and duration, nothing
//! else. The lyrics are only kept in memory (last few tracks), never on disk.
//! [Unverified] LRCLIB documents no licence for its lyrics data: private use only.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::spotify::Track;

/// LRCLIB asks clients to identify themselves.
pub const USER_AGENT: &str = concat!("Atoll/", env!("CARGO_PKG_VERSION"), " (private personal desktop tool)");
const CACHE: usize = 20;

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    /// Start time in ms.
    pub t: u64,
    pub text: String,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    pub track_id: String,
    /// Time-synced lines (empty if only plain lyrics exist).
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    pub instrumental: bool,
    /// "found", "not_found" or "error".
    pub state: String,
}

#[derive(Default)]
pub struct LyricsCache(Mutex<VecDeque<Lyrics>>);

pub fn remember(app: &AppHandle, l: Lyrics) {
    let cache = app.state::<LyricsCache>();
    let mut q = cache.0.lock().unwrap();
    q.retain(|x| x.track_id != l.track_id);
    q.push_back(l);
    while q.len() > CACHE {
        q.pop_front();
    }
}

pub fn cached(app: &AppHandle, track_id: &str) -> Option<Lyrics> {
    app.state::<LyricsCache>().0.lock().unwrap().iter().find(|l| l.track_id == track_id).cloned()
}

/// Parse LRC text: "[mm:ss.xx] line", several timestamps per line allowed.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            rest = &stripped[end + 1..];
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.trim().parse::<u64>(), s.trim().parse::<f64>()) {
                    stamps.push(m * 60_000 + (s * 1000.0).round() as u64);
                }
            }
        }
        for t in stamps {
            lines.push(LyricLine { t, text: rest.trim().to_string() });
        }
    }
    lines.sort_by_key(|l| l.t);
    lines
}

fn from_record(track_id: &str, v: &Value) -> Lyrics {
    let synced = v.get("syncedLyrics").and_then(Value::as_str).map(parse_lrc).unwrap_or_default();
    let plain = v.get("plainLyrics").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(String::from);
    let instrumental = v.get("instrumental").and_then(Value::as_bool).unwrap_or(false);
    let found = !synced.is_empty() || plain.is_some() || instrumental;
    Lyrics {
        track_id: track_id.to_string(),
        synced,
        plain,
        instrumental,
        state: if found { "found" } else { "not_found" }.into(),
    }
}

fn not_found(track_id: &str, state: &str) -> Lyrics {
    Lyrics { track_id: track_id.into(), synced: vec![], plain: None, instrumental: false, state: state.into() }
}

/// Exact match first (needs duration within ±2 s), then a search fallback.
pub async fn fetch(http: &reqwest::Client, t: &Track) -> Lyrics {
    let secs = (t.duration_ms / 1000).to_string();
    let first_artist = t.artists.split(", ").next().unwrap_or(&t.artists).to_string();
    let get = http
        .get("https://lrclib.net/api/get")
        .query(&[
            ("track_name", t.name.as_str()),
            ("artist_name", first_artist.as_str()),
            ("album_name", t.album.as_str()),
            ("duration", secs.as_str()),
        ])
        .send()
        .await;
    match get {
        Ok(r) if r.status().is_success() => {
            if let Ok(v) = r.json::<Value>().await {
                let l = from_record(&t.id, &v);
                if l.state == "found" {
                    return l;
                }
            }
        }
        Ok(r) if r.status().as_u16() == 404 => {}
        Ok(r) => return not_found(&t.id, if r.status().as_u16() == 429 { "error" } else { "not_found" }),
        Err(_) => return not_found(&t.id, "error"),
    }
    // Fallback: search, prefer a result with synced lyrics.
    let search = http
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", t.name.as_str()), ("artist_name", first_artist.as_str())])
        .send()
        .await;
    match search {
        Ok(r) if r.status().is_success() => {
            let list: Vec<Value> = r.json().await.unwrap_or_default();
            let best = list
                .iter()
                .find(|v| v.get("syncedLyrics").and_then(Value::as_str).is_some())
                .or_else(|| list.first());
            best.map(|v| from_record(&t.id, v)).unwrap_or_else(|| not_found(&t.id, "not_found"))
        }
        Ok(_) => not_found(&t.id, "not_found"),
        Err(_) => not_found(&t.id, "error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc() {
        let l = parse_lrc("[ar: X]\n[00:12.50] Hello\n[00:05.00][01:00.00] Chorus\n[00:20.00]\nbad line");
        assert_eq!(l.len(), 4);
        assert_eq!(l[0], LyricLine { t: 5000, text: "Chorus".into() });
        assert_eq!(l[1].t, 12_500);
        assert_eq!(l[2].text, "");
        assert_eq!(l[3].t, 60_000);
    }
}
