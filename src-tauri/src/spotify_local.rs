//! Spotify on this PC, read through Windows' media session: the same source
//! the volume flyout and the keyboard media keys use. No sign-in, no
//! developer app, no Web API rate limits; works with Free and Premium.
//!
//! Docs: https://learn.microsoft.com/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager
//!
//! Gives: title, artist, album, cover, play state, position, and
//! play / pause / next / previous / seek (seek only when Spotify allows it).
//! It cannot see or move playback on other devices (phone, speakers): that
//! needs the optional account link in spotify.rs.
//!
//! Reading is local, so it runs on its own thread once a second while
//! playing (every 2 s otherwise) and is woken early after a command.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use ::windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use ::windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};
use base64::Engine;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::sessions::now_ms;
use crate::spotify::{Device, PlayerView, Track};
use crate::AppState;

/// 100 ns ticks between 1601-01-01 (Windows DateTime) and 1970-01-01.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
const TICKS_PER_MS: i64 = 10_000;
/// Covers bigger than this are ignored (Spotify sends about 300 × 300).
const MAX_COVER_BYTES: u64 = 4 * 1024 * 1024;

static MANAGER: OnceLock<SessionManager> = OnceLock::new();
static WAKE: Mutex<bool> = Mutex::new(false);
static WAKE_CV: Condvar = Condvar::new();

fn manager() -> ::windows::core::Result<&'static SessionManager> {
    if let Some(m) = MANAGER.get() {
        return Ok(m);
    }
    let m = SessionManager::RequestAsync()?.join()?;
    Ok(MANAGER.get_or_init(|| m))
}

/// The installer build reports "Spotify.exe"; the Microsoft Store build
/// reports "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".
fn is_spotify(app_id: &str) -> bool {
    app_id.to_ascii_lowercase().contains("spotify")
}

/// The Spotify app's media session, or None when Spotify isn't open.
fn spotify_session() -> ::windows::core::Result<Option<Session>> {
    let sessions = manager()?.GetSessions()?;
    for i in 0..sessions.Size()? {
        let s = sessions.GetAt(i)?;
        if s.SourceAppUserModelId().is_ok_and(|id| is_spotify(&id.to_string())) {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// Is a Spotify window open, whether or not Spotify shares its playback with
/// Windows? Spotify can run without a media session (its "Show desktop
/// overlay when using media keys" setting off, or nothing played yet since it
/// started), and the island then explains that instead of "Spotify isn't open".
fn spotify_window_open() -> bool {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, IsWindowVisible, GW_OWNER};
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && crate::chatwatch::process_name(hwnd).as_deref().is_some_and(is_spotify_exe)
            {
                *(found as *mut bool) = true;
                return 0; // stop
            }
        }
        1
    }
    let mut found = false;
    unsafe { EnumWindows(Some(each), &mut found as *mut bool as LPARAM) };
    found
}

/// Both the installer and the Microsoft Store build run as Spotify.exe.
fn is_spotify_exe(name: &str) -> bool {
    name.eq_ignore_ascii_case("spotify.exe")
}

/// Wake the watcher now (after a command or a settings change).
pub fn poke() {
    *WAKE.lock().unwrap() = true;
    WAKE_CV.notify_one();
}

fn sleep(d: Duration) {
    let guard = WAKE.lock().unwrap();
    let (mut woke, _) = WAKE_CV.wait_timeout_while(guard, d, |w| !*w).unwrap();
    *woke = false;
}

/// Track length and position (ms) from Windows' timeline. Spotify updates
/// the position only on changes (play, pause, seek, new track), stamped with
/// LastUpdatedTime, so while playing the time since then is added.
fn timeline_ms(start: i64, end: i64, position: i64, updated: i64, playing: bool, now_ms: u64) -> (u64, u64) {
    let duration = ((end - start).max(0) / TICKS_PER_MS) as u64;
    let mut pos = ((position - start).max(0) / TICKS_PER_MS) as u64;
    if playing && updated > UNIX_EPOCH_TICKS {
        let updated_ms = ((updated - UNIX_EPOCH_TICKS) / TICKS_PER_MS) as u64;
        pos += now_ms.saturating_sub(updated_ms);
    }
    if duration > 0 {
        pos = pos.min(duration);
    }
    (duration, pos)
}

/// A stable id for a track (Windows gives no Spotify id): lyrics are keyed by it.
fn track_id(name: &str, artists: &str, album: &str) -> String {
    let digest = Sha256::digest(format!("{name}\u{1f}{artists}\u{1f}{album}").as_bytes());
    format!("pc-{}", digest.iter().take(8).map(|b| format!("{b:02x}")).collect::<String>())
}

struct Snapshot {
    props: MediaProperties,
    track: Option<Track>,
    playing: bool,
    can_seek: bool,
    position_ms: u64,
}

fn read(session: &Session) -> ::windows::core::Result<Snapshot> {
    let info = session.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == PlaybackStatus::Playing;
    let can_seek = info.Controls().and_then(|c| c.IsPlaybackPositionEnabled()).unwrap_or(false);
    let tl = session.GetTimelineProperties()?;
    let (duration_ms, position_ms) = timeline_ms(
        tl.StartTime()?.Duration,
        tl.EndTime()?.Duration,
        tl.Position()?.Duration,
        tl.LastUpdatedTime()?.UniversalTime,
        playing,
        now_ms(),
    );
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let name = props.Title()?.to_string();
    let artists = props.Artist()?.to_string();
    let album = props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default();
    // Spotify keeps a session with an empty title before anything is loaded.
    let track = (!name.trim().is_empty()).then(|| Track {
        id: track_id(&name, &artists, &album),
        name,
        artists,
        album,
        image: None,
        duration_ms,
    });
    Ok(Snapshot { props, track, playing, can_seek, position_ms })
}

/// The cover as a data: URL (the island's CSP allows data: images).
fn read_cover(props: &MediaProperties) -> Option<String> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_COVER_BYTES {
        return None;
    }
    let mime = stream.ContentType().map(|m| m.to_string()).ok().filter(|m| m.starts_with("image/")).unwrap_or_else(|| "image/jpeg".into());
    let buffer = Buffer::Create(size as u32).ok()?;
    let filled = stream.ReadAsync(&buffer, size as u32, InputStreamOptions::None).ok()?.join().ok()?;
    let mut bytes = vec![0u8; filled.Length().ok()? as usize];
    DataReader::FromBuffer(&filled).ok()?.ReadBytes(&mut bytes).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn this_pc() -> Device {
    Device { id: None, name: "this PC".into(), kind: "Computer".into(), is_active: true, is_restricted: false, volume: None }
}

#[derive(Default)]
struct Watch {
    key: Option<String>,
    cover: Option<String>,
    /// Spotify can hand over the new cover a moment after the new title,
    /// so the cover is read again on the first few looks at a new track.
    cover_reads_left: u8,
    /// Looks at the same track in a row; lyrics wait for 2 so the length
    /// (which LRCLIB matches within ±2 s) has caught up with the title.
    seen: u8,
}

/// One look at Spotify; returns how long to wait before the next one.
fn tick(app: &AppHandle, w: &mut Watch) -> Duration {
    let (on, lyrics_on) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (s.modules.spotify && s.spotify.connected, s.spotify.lyrics)
    };
    if !on {
        *w = Watch::default();
        crate::spotify::set_app_state(app, false, false);
        crate::spotify::publish_player(app, None);
        return Duration::from_secs(2);
    }
    let session = match spotify_session() {
        Ok(Some(s)) => s,
        Ok(None) => {
            *w = Watch::default();
            crate::spotify::set_app_state(app, false, spotify_window_open());
            crate::spotify::publish_player(app, None);
            return Duration::from_secs(2);
        }
        Err(e) => {
            eprintln!("[atoll] Windows media session unavailable: {e}");
            crate::spotify::set_app_state(app, false, spotify_window_open());
            return Duration::from_secs(5);
        }
    };
    crate::spotify::set_app_state(app, true, false);
    let snap = match read(&session) {
        Ok(s) => s,
        Err(_) => return Duration::from_secs(1), // mid track change: look again shortly
    };

    let mut track = snap.track;
    let key = track.as_ref().map(|t| t.id.clone());
    if key != w.key {
        *w = Watch { key, cover: None, cover_reads_left: 3, seen: 0 };
    }
    w.seen = w.seen.saturating_add(1);
    if track.is_some() && w.cover_reads_left > 0 {
        w.cover_reads_left -= 1;
        if let Some(c) = read_cover(&snap.props) {
            w.cover = Some(c);
        }
    }
    if let Some(t) = track.as_mut() {
        t.image = w.cover.clone();
        if lyrics_on && w.seen >= 2 && t.duration_ms > 0 {
            crate::spotify::want_lyrics(app, t);
        }
    }
    crate::spotify::publish_player(
        app,
        Some(PlayerView {
            is_playing: snap.playing,
            track,
            progress_ms: snap.position_ms,
            fetched_at: now_ms(),
            device: Some(this_pc()),
            can_seek: snap.can_seek,
        }),
    );
    Duration::from_millis(if snap.playing { 1000 } else { 2000 })
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("spotify-on-this-pc".into())
        .spawn(move || {
            let mut w = Watch::default();
            loop {
                let wait = tick(&app, &mut w);
                sleep(wait);
            }
        })
        .expect("could not start the Spotify watcher thread");
}

/// play | pause | toggle | next | previous | seek (value = ms).
/// Runs on the calling thread and blocks briefly: call from spawn_blocking.
pub fn command(action: &str, value_ms: Option<u64>) -> Result<(), String> {
    let s = spotify_session().map_err(|e| format!("Windows media controls unavailable: {e}"))?.ok_or("Spotify isn't open on this PC.")?;
    let op = match action {
        "play" => s.TryPlayAsync(),
        "pause" => s.TryPauseAsync(),
        "toggle" => s.TryTogglePlayPauseAsync(),
        "next" => s.TrySkipNextAsync(),
        "previous" => s.TrySkipPreviousAsync(),
        "seek" => {
            let ms = value_ms.ok_or("seek needs a position in ms")? as i64;
            let start = s.GetTimelineProperties().and_then(|t| t.StartTime()).map(|t| t.Duration).unwrap_or(0);
            s.TryChangePlaybackPositionAsync(start + ms * TICKS_PER_MS)
        }
        other => return Err(format!("unknown action {other}")),
    };
    let accepted = op.and_then(|o| o.join()).map_err(|e| format!("Spotify didn't respond: {e}"))?;
    poke();
    if accepted {
        Ok(())
    } else if action == "seek" {
        Err("Spotify doesn't accept seeking from Windows right now.".into())
    } else {
        Err("Spotify didn't accept that. Try it in the Spotify app.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = TICKS_PER_MS * 1000; // one second in ticks

    #[test]
    fn timeline_adds_time_since_update_only_while_playing() {
        let now = 1_700_000_000_000u64;
        let updated = UNIX_EPOCH_TICKS + (now as i64 - 4_000) * TICKS_PER_MS; // 4 s ago
        assert_eq!(timeline_ms(0, 200 * S, 30 * S, updated, true, now), (200_000, 34_000));
        assert_eq!(timeline_ms(0, 200 * S, 30 * S, updated, false, now), (200_000, 30_000));
        // Never runs past the end, and a missing stamp adds nothing.
        assert_eq!(timeline_ms(0, 200 * S, 199 * S, updated, true, now), (200_000, 200_000));
        assert_eq!(timeline_ms(0, 200 * S, 30 * S, 0, true, now), (200_000, 30_000));
    }

    #[test]
    fn recognises_both_spotify_builds() {
        assert!(is_spotify("Spotify.exe"));
        assert!(is_spotify("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"));
        assert!(!is_spotify("Chrome"));
    }

    #[test]
    fn recognises_spotify_process() {
        assert!(is_spotify_exe("spotify.exe"));
        assert!(is_spotify_exe("Spotify.exe"));
        assert!(!is_spotify_exe("spotifywebhelper.exe"));
        assert!(!is_spotify_exe("chrome.exe"));
    }

    /// With Spotify open, this must say true, whether or not it is playing:
    /// cargo test --lib probe_spotify_window -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_spotify_window() {
        println!("window_open={} session={:?}", spotify_window_open(), spotify_session().map(|s| s.is_some()));
    }

    /// Reads the real Spotify app. Open Spotify, then:
    /// cargo test --lib probe_spotify -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_spotify() {
        let s = spotify_session().expect("Windows media session").expect("Spotify isn't open");
        let snap = read(&s).expect("read session");
        let cover = read_cover(&snap.props);
        println!(
            "track={:?}\nplaying={} can_seek={} position_ms={}\ncover={:?}",
            snap.track,
            snap.playing,
            snap.can_seek,
            snap.position_ms,
            cover.as_deref().map(|c| (&c[..c.find(',').unwrap_or(0)], c.len()))
        );
        // While playing, the estimate should move about 2 s per 2 s.
        for _ in 0..2 {
            std::thread::sleep(Duration::from_secs(2));
            println!("position_ms={}", read(&s).expect("read session").position_ms);
        }
    }

    #[test]
    fn track_ids_are_stable_and_distinct() {
        assert_eq!(track_id("A", "B", "C"), track_id("A", "B", "C"));
        assert_ne!(track_id("A", "B", "C"), track_id("A", "B", "D"));
        assert!(track_id("A", "B", "C").starts_with("pc-"));
    }
}
