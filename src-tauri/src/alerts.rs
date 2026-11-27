//! Alerts: a sound, a Windows notification and an island colour flash, per event.
//!
//! Everything goes through `fire`, which checks the module switch and the
//! per-trigger settings, drops duplicates (same session + kind within a short
//! window), then plays the sound, shows the toast and tells the island to flash.
//!
//! Sounds are synthesised at runtime (no audio files) and played with the
//! Win32 `PlaySoundW` API. Toasts use the WinRT toast API through
//! `tauri-winrt-notification`, under Crest's own AppUserModelID which we
//! register per user in `HKCU\Software\Classes\AppUserModelId` so the toast
//! shows "Crest" instead of "Windows PowerShell".

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::windows::ISLAND;
use crate::AppState;

pub const AUMID: &str = "local.crest.island";

/// Same alert for the same session is dropped for this long.
const DEDUPE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A session finished its turn.
    Done,
    /// A session needs you (permission, question).
    Attention,
    /// A turn failed (rate limit, auth, API error).
    Error,
    /// Usage passed the warning threshold.
    Usage,
    /// Experimental: a reply finished in the Claude desktop Chat tab.
    ChatReply,
    /// A focus or break timer ended (focus mode).
    Timer,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub kind: Kind,
    pub session_id: Option<String>,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct AlertState {
    recent: Mutex<HashMap<(String, Kind), Instant>>,
    /// Keeps the WAV bytes alive while Windows plays them asynchronously.
    playing: Mutex<Vec<u8>>,
}

/// The settings entry for an alert kind.
fn event_of(s: &crate::settings::Settings, kind: Kind) -> crate::settings::EventAlert {
    let a = &s.alerts;
    match kind {
        Kind::Done => a.done.clone(),
        Kind::Attention => a.permission.clone(),
        Kind::Error => a.error.clone(),
        Kind::Usage => a.usage.clone(),
        Kind::ChatReply => a.chat.clone(),
        Kind::Timer => a.timer.clone(),
    }
}

/// Decide, dedupe and deliver one alert. `test` skips the module switch and dedupe.
pub fn fire(app: &AppHandle, alert: Alert, test: bool) {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let event = event_of(&s, alert.kind);
    if !test {
        let wanted = s.modules.alerts && (alert.kind != Kind::ChatReply || s.modules.chat_watcher);
        if !wanted {
            return;
        }
        let key = (alert.session_id.clone().unwrap_or_default(), alert.kind);
        let state = app.state::<AlertState>();
        let mut recent = state.recent.lock().unwrap();
        let now = Instant::now();
        recent.retain(|_, t| now.duration_since(*t) < DEDUPE);
        if recent.contains_key(&key) {
            return;
        }
        recent.insert(key, now);
    }

    // During a focus session, Settings → Focus timer decides (held for the digest, quiet, dropped).
    let decision = if test { crate::focus::Decision::Deliver } else { crate::focus::intercept(app, &alert) };
    let quiet = match decision {
        crate::focus::Decision::Hold | crate::focus::Decision::Drop => {
            println!("[atoll] alert {:?} {:?} during focus: {}", alert.kind, decision, alert.title);
            return;
        }
        crate::focus::Decision::Quiet => true,
        crate::focus::Decision::Deliver => false,
    };

    println!("[atoll] alert {:?}: {} - {}", alert.kind, alert.title, alert.body);
    if s.alerts.flash {
        let _ = app.emit_to(ISLAND, "alert", &alert);
    }
    let silent_done = alert.kind == Kind::Done && !s.claude.sound_on_done;
    if event.sound != "none" && event.volume > 0 && !silent_done && !quiet {
        play(app, &event.sound, event.volume);
    }
    if event.notify && !quiet {
        let (title, body) = (alert.title.clone(), alert.body.clone());
        std::thread::spawn(move || toast(&title, &body));
    }
}

// ---------- sound ----------
// Four original sounds, synthesised at runtime (no audio files):
// chime (two soft rising notes), pop (quick pitch drop), bell (inharmonic
// partials with a long ring) and wood block (short hollow knock).

const RATE: u32 = 22_050;

/// Raw samples (-1..1) for a named sound.
fn samples(sound: &str) -> Vec<f32> {
    use std::f32::consts::TAU;
    let n = |secs: f32| (secs * RATE as f32) as usize;
    let t = |i: usize| i as f32 / RATE as f32;
    let attack = |x: f32, a: f32| (x / a).min(1.0);
    match sound {
        "pop" => (0..n(0.12))
            .map(|i| {
                let x = t(i);
                // pitch falls from 880 Hz to ~330 Hz; phase is the integral of frequency
                let phase = TAU * (330.0 * x + 550.0 * (1.0 - (-x * 40.0).exp()) / 40.0);
                phase.sin() * attack(x, 0.002) * (-x * 38.0).exp()
            })
            .collect(),
        "bell" => (0..n(1.1))
            .map(|i| {
                let x = t(i);
                let f = 660.0;
                let v = (TAU * f * x).sin() * (-x * 3.2).exp()
                    + 0.45 * (TAU * f * 2.76 * x).sin() * (-x * 5.5).exp()
                    + 0.25 * (TAU * f * 5.40 * x).sin() * (-x * 9.0).exp();
                v * 0.6 * attack(x, 0.003)
            })
            .collect(),
        "wood" => (0..n(0.16))
            .map(|i| {
                let x = t(i);
                let v = (TAU * 1100.0 * x).sin() * (-x * 42.0).exp() + 0.5 * (TAU * 1680.0 * x).sin() * (-x * 60.0).exp();
                v * 0.7 * attack(x, 0.001)
            })
            .collect(),
        _ => {
            // chime: rising fifth, each note bell-like
            let mut out = Vec::new();
            for &(f, secs) in &[(784.0f32, 0.12f32), (1174.7, 0.42)] {
                out.extend((0..n(secs)).map(|i| {
                    let x = t(i);
                    ((TAU * f * x).sin() * 0.8 + (TAU * f * 2.0 * x).sin() * 0.2) * attack(x, 0.006) * (-x * 7.0).exp()
                }));
            }
            out
        }
    }
}

/// 16-bit mono PCM WAV of a named sound at a volume (0..=100).
pub fn synth_wav(sound: &str, volume: u32) -> Vec<u8> {
    let amp = (volume.min(100) as f32 / 100.0) * 0.45 * i16::MAX as f32;
    let mut pcm: Vec<i16> = samples(sound).into_iter().map(|v| (v.clamp(-1.0, 1.0) * amp) as i16).collect();
    // 30 ms tail of silence so the end isn't clipped.
    pcm.extend(std::iter::repeat(0).take((RATE as f32 * 0.03) as usize));

    let data_len = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for v in pcm {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav
}

/// Play a named sound now (also used by the preview buttons in Settings).
pub fn play(app: &AppHandle, sound: &str, volume: u32) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    if sound == "none" {
        return;
    }
    let wav = synth_wav(sound, volume);
    let state = app.state::<AlertState>();
    let mut playing = state.playing.lock().unwrap();
    // SAFETY: SND_MEMORY reads from `wav`; we start playback first, then move
    // the buffer into `playing`, which keeps it alive until the next sound
    // (PlaySoundW stops the previous sound before switching buffers).
    unsafe {
        PlaySoundW(wav.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
    *playing = wav;
}

// ---------- toast ----------

/// Register Crest's AppUserModelID for toasts (per user, no admin rights).
pub fn register_toast_identity() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let icon = crate::paths::local_dir().join("crest-icon.png");
    let _ = std::fs::create_dir_all(crate::paths::local_dir());
    let _ = std::fs::write(&icon, include_bytes!("../icons/128x128.png"));

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.create_subkey(format!("Software\\Classes\\AppUserModelId\\{AUMID}")) {
        Ok((key, _)) => {
            let _ = key.set_value("DisplayName", &"Atoll");
            let _ = key.set_value("IconUri", &icon.display().to_string());
        }
        Err(e) => eprintln!("[crest] could not register toast identity: {e}"),
    }
}

pub(crate) fn toast(title: &str, body: &str) {
    use tauri_winrt_notification::{Duration as ToastDuration, Toast};
    let result = Toast::new(AUMID)
        .title(title)
        .text1(body)
        .sound(None) // Atoll plays its own sound
        .duration(ToastDuration::Short)
        .show();
    if let Err(e) = result {
        eprintln!("[crest] toast failed: {e:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let wav = synth_wav("chime", 60);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        let riff_len = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_len, wav.len() - 8);
        // silence at volume 0
        assert!(synth_wav("chime", 0)[44..].iter().all(|b| *b == 0));
        for name in ["pop", "bell", "wood"] {
            let w = synth_wav(name, 80);
            assert!(w.len() > 44 + 2000, "{name} too short");
            assert!(w[44..].iter().any(|b| *b != 0), "{name} silent");
        }
    }
}
