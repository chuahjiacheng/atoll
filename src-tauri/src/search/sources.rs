//! Turns each module's data into search documents. Read-only: these call the
//! modules' public functions and never touch their files.

use tauri::{AppHandle, Manager};

use super::index::{Doc, Source};

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

fn file_name(p: &str) -> String {
    p.rsplit(['\\', '/']).next().unwrap_or(p).to_string()
}

/// "VS Code" for "Code.exe" (matches the clipboard tab).
fn app_name(source: &str) -> String {
    let bare = source.strip_suffix(".exe").or_else(|| source.strip_suffix(".EXE")).unwrap_or(source);
    match bare.to_ascii_lowercase().as_str() {
        "code" => "VS Code".into(),
        "winword" => "Word".into(),
        "excel" => "Excel".into(),
        "powerpnt" => "PowerPoint".into(),
        "chrome" => "Chrome".into(),
        "msedge" => "Edge".into(),
        "explorer" => "File Explorer".into(),
        _ => bare.to_string(),
    }
}

pub fn clipboard(app: &AppHandle) -> Vec<Doc> {
    crate::clipboard::list(app)
        .into_iter()
        // Secrets are never indexed, not even masked.
        .filter(|c| c.secret.is_none())
        .map(|c| {
            let (title, body, kind) = match c.kind.as_str() {
                "files" => (
                    match c.files.len() {
                        1 => file_name(&c.files[0]),
                        n => format!("{} + {} more", file_name(&c.files[0]), n - 1),
                    },
                    c.files.join("\n"),
                    "files",
                ),
                "image" => match &c.ocr {
                    Some(t) => (first_line(t, 120), t.clone(), "image"),
                    None => (format!("Image {} × {}", c.width, c.height), String::new(), "image"),
                },
                _ => {
                    let t = c.text.clone().unwrap_or_default();
                    (first_line(&t, 120), t, "text")
                }
            };
            let what = match (c.kind.as_str(), c.types.first().map(String::as_str)) {
                ("image", _) if c.ocr.is_some() => "Image with text",
                ("image", _) => "Image",
                ("files", _) => "File",
                (_, Some("link")) => "Link",
                (_, Some("colour")) => "Colour",
                (_, Some("code")) => "Code",
                (_, Some("json")) => "JSON",
                _ => "Text",
            };
            let sub = match &c.source {
                Some(s) => format!("Clipboard · {what} · {}", app_name(s)),
                None => format!("Clipboard · {what}"),
            };
            Doc { key: format!("clipboard:{}", c.id), source: Source::Clipboard, title, sub, body, at: c.at, pinned: c.pinned, kind: kind.into() }
        })
        .collect()
}

pub fn whiteboard(app: &AppHandle) -> Vec<Doc> {
    crate::board::get(app)
        .cards
        .into_iter()
        .map(|c| {
            let (title, body) = match c.kind.as_str() {
                "text" => {
                    let t = c.text.clone().unwrap_or_default();
                    (first_line(&t, 120), t)
                }
                _ => (c.name.clone().or_else(|| c.path.as_deref().map(file_name)).unwrap_or_else(|| "Image".into()), c.path.clone().unwrap_or_default()),
            };
            let what = match c.kind.as_str() {
                "text" => "Text card",
                "image" => "Image card",
                _ => "File card",
            };
            Doc { key: format!("whiteboard:{}", c.id), source: Source::Whiteboard, title, sub: format!("Whiteboard · {what}"), body, at: c.at, pinned: false, kind: c.kind }
        })
        .collect()
}

/// One document per non-empty line of the quick note (up to 2,000 lines).
pub fn note(_app: &AppHandle) -> Vec<Doc> {
    let text = crate::tray::note_get();
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .take(2_000)
        .map(|(i, l)| Doc {
            key: format!("note:{i}"),
            source: Source::Note,
            title: first_line(l, 160),
            sub: format!("Quick note · line {}", i + 1),
            body: l.to_string(),
            // Lines have no time of their own; earlier lines rank a touch higher.
            at: 0,
            pinned: false,
            kind: "line".into(),
        })
        .collect()
}

pub fn file_tray(app: &AppHandle) -> Vec<Doc> {
    crate::tray::list(app)
        .into_iter()
        .map(|f| Doc {
            key: format!("fileTray:{}", f.id),
            source: Source::FileTray,
            sub: format!("File tray · {}", parent(&f.path)),
            title: f.name,
            body: f.path,
            at: f.at,
            pinned: false,
            kind: if f.is_dir { "folder".into() } else { "file".into() },
        })
        .collect()
}

/// "C:\…\Desktop" style: the drive and the last folder.
fn parent(p: &str) -> String {
    let parts: Vec<&str> = p.split(['\\', '/']).collect();
    match parts.len() {
        0..=2 => parts.first().copied().unwrap_or("").to_string(),
        3 => format!("{}\\{}", parts[0], parts[1]),
        n => format!("{}\\…\\{}", parts[0], parts[n - 2]),
    }
}

pub fn calendar(app: &AppHandle) -> Vec<Doc> {
    crate::calendar::agenda(app)
        .events
        .into_iter()
        .filter(|e| !e.declined)
        .map(|e| {
            let at = parse_time(&e.start).unwrap_or(0);
            let when = if e.all_day { "All day".to_string() } else { e.start.get(11..16).unwrap_or("").to_string() };
            let day = e.start.get(..10).unwrap_or("");
            let mut body = e.title.clone();
            if let Some(l) = &e.location {
                body.push('\n');
                body.push_str(l);
            }
            Doc {
                key: format!("calendar:{}", e.id),
                source: Source::Calendar,
                title: e.title.clone(),
                sub: format!("Calendar · {day} {when}{}", e.location.as_deref().map(|l| format!(" · {l}")).unwrap_or_default()),
                body,
                at,
                pinned: false,
                kind: if e.meet.is_some() { "call".into() } else { "event".into() },
            }
        })
        .collect()
}

pub fn agent(app: &AppHandle, since_ms: i64) -> Vec<Doc> {
    super::agent::fetch_since(app, since_ms, 5_000)
        .into_iter()
        .map(|e| Doc {
            key: format!("agent:{}", e.id),
            source: Source::Agent,
            title: e.summary.clone(),
            sub: format!("Agent history · {}{}", e.kind, e.tool.as_deref().map(|t| format!(" · {t}")).unwrap_or_default()),
            body: e.session_id.clone(),
            at: e.ts.max(0) as u64,
            pinned: false,
            kind: e.kind,
        })
        .collect()
}

/// Is this module switched on? (Search leaves switched-off modules out.)
pub fn enabled(app: &AppHandle, source: Source) -> bool {
    let s = app.state::<crate::AppState>().settings.lock().unwrap().clone();
    match source {
        Source::Clipboard => s.modules.clipboard,
        Source::Whiteboard => s.modules.whiteboard,
        Source::Note => s.modules.note,
        Source::FileTray => s.modules.file_tray,
        Source::Calendar => s.modules.calendar,
        Source::Agent => s.modules.claude,
    }
}

/// RFC 3339 ("2026-11-15T09:30:00+08:00", "…Z") or a date ("2026-11-15") to ms.
pub fn parse_time(s: &str) -> Option<u64> {
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = if s.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    // Offset: trailing Z, or ±HH:MM.
    let offset = if s.len() > 19 {
        let tail = &s[19..];
        let tail = tail.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        match tail.chars().next() {
            Some(sign @ ('+' | '-')) if tail.len() >= 6 => {
                let h: i64 = tail.get(1..3)?.parse().ok()?;
                let m: i64 = tail.get(4..6)?.parse().ok()?;
                (h * 60 + m) * if sign == '+' { 1 } else { -1 }
            }
            _ => 0,
        }
    } else {
        0
    };
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3_600 + mm * 60 + ss - offset * 60;
    u64::try_from(secs).ok().map(|s| s * 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("1970-01-01"), Some(0));
        // Expected values checked with Python's datetime.
        assert_eq!(parse_time("2026-11-15T00:00:00Z"), Some(1_794_700_800_000));
        assert_eq!(parse_time("2026-11-15T08:00:00+08:00"), Some(1_794_700_800_000));
        assert_eq!(parse_time("2026-11-14T16:00:00.000-08:00"), Some(1_794_700_800_000));
        assert_eq!(parse_time("nope"), None);
    }

    #[test]
    fn short_paths() {
        assert_eq!(parent("C:\\Users\\me\\Desktop\\a.pdf"), "C:\\…\\Desktop");
        assert_eq!(first_line("\n  hello world  \nmore", 5), "hello…");
        assert_eq!(app_name("WINWORD.EXE"), "Word");
    }
}
