//! Claude chats (claude.ai) for the Agent tab, on any plan, Free included.
//!
//! The Chat tab has no hooks or API, but the claude.ai web app (which the Claude
//! desktop app runs) keeps a local cache in its IndexedDB:
//! - a react-query snapshot with the account (plan) and the recent chats list
//!   (names, times, the active message of each);
//! - a "conversation store" with the message tree of chats you opened.
//!
//! We read those files (never write, never touch the network). Chromium keeps
//! IndexedDB in LevelDB; big values sit in separate blob files. Values are
//! Blink/V8 "structured clone" bytes, sometimes Snappy-compressed. This module
//! has small readers for all three formats.
//!
//! [Unverified] The format is claude.ai's private cache, so an app update can
//! change it; everything here fails soft (no chats shown, Code still works).
//! Browsers (Chrome / Edge / Brave) are read the same way; only the desktop
//! app was checked against real data.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// How many chats the island lists.
const MAX_CHATS: usize = 8;
const ASK_CHARS: usize = 400;
const ANSWER_CHARS: usize = 6000;
/// Skip blob files bigger than this (a whole-cache snapshot is ~2 MB).
const MAX_BLOB: u64 = 40 << 20;

#[derive(Clone, Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub uuid: String,
    pub name: String,
    /// ms since epoch.
    pub updated_at: u64,
    /// Your last message in the chat (if the chat's messages are cached).
    pub ask: Option<String>,
    /// Claude's last reply (if cached).
    pub answer: Option<String>,
    /// When the last reply was written, ms (0 = unknown).
    pub answered_at: u64,
    pub message_count: u32,
    /// The chat changed after its messages were cached (a reply being
    /// written, or new messages not opened in Claude yet).
    pub behind: bool,
}

#[derive(Clone, Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatsView {
    /// A claude.ai cache was found and read.
    pub available: bool,
    /// "free" | "pro" | "max" | "team" | "enterprise" | "" (unknown).
    pub plan: String,
    /// Newest first.
    pub chats: Vec<Chat>,
}

#[derive(Default)]
pub struct ChatsState {
    view: Mutex<ChatsView>,
}

// ---------------------------------------------------------------- formats

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut r: u64 = 0;
    let mut shift = 0;
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        if shift > 63 {
            return None;
        }
        r |= ((c & 0x7f) as u64) << shift;
        if c & 0x80 == 0 {
            return Some(r);
        }
        shift += 7;
    }
}

/// Raw Snappy block decompression.
fn unsnappy(b: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    let n = varint(b, &mut i)? as usize;
    if n > 256 << 20 {
        return None;
    }
    let mut o = Vec::with_capacity(n);
    while i < b.len() {
        let t = b[i];
        i += 1;
        if t & 3 == 0 {
            let mut l = (t >> 2) as usize;
            if l >= 60 {
                let nb = l - 59;
                let s = b.get(i..i + nb)?;
                l = s.iter().enumerate().fold(0, |a, (j, &x)| a | (x as usize) << (8 * j));
                i += nb;
            }
            l += 1;
            o.extend_from_slice(b.get(i..i + l)?);
            i += l;
        } else {
            let (l, off) = match t & 3 {
                1 => {
                    let x = *b.get(i)? as usize;
                    i += 1;
                    (((t >> 2) & 7) as usize + 4, ((t as usize >> 5) << 8) | x)
                }
                2 => {
                    let s = b.get(i..i + 2)?;
                    i += 2;
                    ((t >> 2) as usize + 1, u16::from_le_bytes([s[0], s[1]]) as usize)
                }
                _ => {
                    let s = b.get(i..i + 4)?;
                    i += 4;
                    ((t >> 2) as usize + 1, u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
                }
            };
            if off == 0 || off > o.len() {
                return None;
            }
            let start = o.len() - off;
            for j in 0..l {
                let c = o[start + j];
                o.push(c);
            }
        }
        if o.len() > n {
            return None;
        }
    }
    (o.len() == n).then_some(o)
}

/// V8 ValueSerializer bytes → JSON (the parts JSON can hold).
struct V8<'a> {
    b: &'a [u8],
    i: usize,
    ids: Vec<Value>,
    depth: u32,
}

impl<'a> V8<'a> {
    fn new(b: &'a [u8]) -> Self {
        V8 { b, i: 0, ids: Vec::new(), depth: 0 }
    }

    fn run(mut self) -> Option<Value> {
        if self.b.first() == Some(&0xff) {
            self.i = 1;
            varint(self.b, &mut self.i)?;
        }
        self.value()
    }

    fn peek(&mut self) -> Option<u8> {
        while *self.b.get(self.i)? == 0 {
            self.i += 1; // padding before two-byte strings
        }
        self.b.get(self.i).copied()
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }

    fn num(&mut self) -> Option<u64> {
        varint(self.b, &mut self.i)
    }

    fn f64(&mut self) -> Option<f64> {
        let s = self.take(8)?;
        Some(f64::from_le_bytes(s.try_into().ok()?))
    }

    fn new_id(&mut self) -> usize {
        self.ids.push(Value::Null);
        self.ids.len() - 1
    }

    fn props(&mut self, end: u8) -> Option<Map<String, Value>> {
        let mut m = Map::new();
        loop {
            if self.peek()? == end {
                self.i += 1;
                return Some(m);
            }
            let k = match self.value()? {
                Value::String(s) => s,
                other => other.to_string(),
            };
            let v = self.value()?;
            m.insert(k, v);
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        let v = self.value_inner();
        self.depth -= 1;
        v
    }

    fn value_inner(&mut self) -> Option<Value> {
        let t = self.peek()?;
        self.i += 1;
        Some(match t {
            b'_' | b'0' | b'-' => Value::Null,
            b'T' => Value::Bool(true),
            b'F' => Value::Bool(false),
            b'I' => {
                let v = self.num()?;
                Value::from(((v >> 1) as i64) ^ -((v & 1) as i64))
            }
            b'U' => Value::from(self.num()?),
            b'N' => serde_json::Number::from_f64(self.f64()?).map(Value::Number).unwrap_or(Value::Null),
            b'D' => {
                let id = self.new_id();
                let v = serde_json::Number::from_f64(self.f64()?).map(Value::Number).unwrap_or(Value::Null);
                self.ids[id] = v.clone();
                v
            }
            b'Z' => {
                let bits = self.num()? as usize;
                self.take(bits >> 1)?;
                Value::Null
            }
            b'"' => {
                let n = self.num()? as usize;
                Value::String(self.take(n)?.iter().map(|&c| c as char).collect())
            }
            b'c' => {
                let n = self.num()? as usize;
                let s = self.take(n)?;
                let u: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Value::String(String::from_utf16_lossy(&u))
            }
            b'S' => {
                let n = self.num()? as usize;
                Value::String(String::from_utf8_lossy(self.take(n)?).into_owned())
            }
            b'^' => {
                let id = self.num()? as usize;
                self.ids.get(id).cloned().unwrap_or(Value::Null)
            }
            b'o' => {
                let id = self.new_id();
                let m = self.props(b'{')?;
                self.num()?;
                let v = Value::Object(m);
                self.ids[id] = v.clone();
                v
            }
            b'A' => {
                let n = self.num()? as usize;
                let id = self.new_id();
                let mut a = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    a.push(self.value()?);
                }
                self.props(b'$')?;
                self.num()?;
                self.num()?;
                let v = Value::Array(a);
                self.ids[id] = v.clone();
                v
            }
            b'a' => {
                let n = self.num()? as usize;
                let id = self.new_id();
                let m = self.props(b'@')?;
                self.num()?;
                self.num()?;
                let mut a = vec![Value::Null; n.min(100_000)];
                for (k, v) in m {
                    if let Some(slot) = k.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                        *slot = v;
                    }
                }
                let v = Value::Array(a);
                self.ids[id] = v.clone();
                v
            }
            b';' => {
                let id = self.new_id();
                let mut m = Map::new();
                loop {
                    if self.peek()? == b':' {
                        self.i += 1;
                        break;
                    }
                    let k = match self.value()? {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    let v = self.value()?;
                    m.insert(k, v);
                }
                self.num()?;
                let v = Value::Object(m);
                self.ids[id] = v.clone();
                v
            }
            b'\'' => {
                let id = self.new_id();
                let mut a = Vec::new();
                loop {
                    if self.peek()? == b',' {
                        self.i += 1;
                        break;
                    }
                    a.push(self.value()?);
                }
                self.num()?;
                let v = Value::Array(a);
                self.ids[id] = v.clone();
                v
            }
            b'y' | b'x' => {
                let id = self.new_id();
                let v = Value::Bool(t == b'y');
                self.ids[id] = v.clone();
                v
            }
            b'n' => {
                let id = self.new_id();
                let v = serde_json::Number::from_f64(self.f64()?).map(Value::Number).unwrap_or(Value::Null);
                self.ids[id] = v.clone();
                v
            }
            b's' => {
                let id = self.new_id();
                let v = self.value()?;
                self.ids[id] = v.clone();
                v
            }
            b'R' => {
                let id = self.new_id();
                let v = self.value()?;
                self.num()?;
                self.ids[id] = v.clone();
                v
            }
            b'B' => {
                self.new_id();
                let n = self.num()? as usize;
                self.take(n)?;
                Value::Null
            }
            b'V' => {
                self.new_id();
                self.i += 1; // view type
                self.num()?;
                self.num()?;
                // Newer versions add a flags varint; it is small and never a tag we'd
                // misread here because a view is always the last thing in its slot.
                if matches!(self.b.get(self.i), Some(x) if *x < 0x08) {
                    self.num()?;
                }
                Value::Null
            }
            _ => return None,
        })
    }
}

/// One IndexedDB value (Blink envelope, maybe Snappy-compressed) → JSON.
fn idb_value(b: &[u8]) -> Option<Value> {
    let mut owned: Vec<u8>;
    let mut b = b;
    for _ in 0..3 {
        if b.first() != Some(&0xff) {
            return None;
        }
        let mut i = 1;
        let version = varint(b, &mut i)?;
        match b.get(i) {
            // Blink 17+: the rest is Snappy-compressed.
            Some(0x02) if version >= 17 => {
                owned = unsnappy(&b[i + 1..])?;
                b = &owned;
                continue;
            }
            // Stored in a separate blob file; read there instead.
            Some(0x01) if version >= 17 => return None,
            // Trailer offset (8 bytes) + size (4 bytes).
            Some(0xfe) => i += 13,
            _ => {}
        }
        return if b.get(i) == Some(&0xff) { V8::new(&b[i..]).run() } else { V8::new(b).run() };
    }
    None
}

/// Every value in a LevelDB directory (log + tables). Order and deletes are
/// ignored: callers pick the newest copy by its own timestamp.
fn leveldb_values(dir: &Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        let Ok(bytes) = std::fs::read(&p) else { continue };
        match ext {
            "log" => read_log(&bytes, &mut out),
            "ldb" | "sst" => read_table(&bytes, &mut out),
            _ => {}
        }
    }
    out
}

fn read_batch(r: &[u8], out: &mut Vec<Vec<u8>>) -> Option<()> {
    let n = u32::from_le_bytes(r.get(8..12)?.try_into().ok()?);
    let mut i = 12;
    for _ in 0..n {
        let kind = *r.get(i)?;
        i += 1;
        let kl = varint(r, &mut i)? as usize;
        i += kl;
        if kind == 1 {
            let vl = varint(r, &mut i)? as usize;
            out.push(r.get(i..i + vl)?.to_vec());
            i += vl;
        }
    }
    Some(())
}

fn read_log(b: &[u8], out: &mut Vec<Vec<u8>>) {
    const BLOCK: usize = 32768;
    let mut frag: Vec<u8> = Vec::new();
    let mut blk = 0;
    while blk < b.len() {
        let end = (blk + BLOCK).min(b.len());
        let mut i = blk;
        while i + 7 <= end {
            let len = u16::from_le_bytes([b[i + 4], b[i + 5]]) as usize;
            let kind = b[i + 6];
            if kind == 0 && len == 0 {
                break;
            }
            let Some(d) = b.get(i + 7..i + 7 + len) else { break };
            i += 7 + len;
            match kind {
                1 => {
                    read_batch(d, out);
                }
                2 => frag = d.to_vec(),
                3 => frag.extend_from_slice(d),
                4 => {
                    frag.extend_from_slice(d);
                    read_batch(&frag, out);
                    frag.clear();
                }
                _ => {}
            }
        }
        blk += BLOCK;
    }
}

fn read_block_values(d: &[u8], out: &mut Vec<Vec<u8>>, handles: bool) -> Option<Vec<(usize, usize)>> {
    let nr = u32::from_le_bytes(d.get(d.len().checked_sub(4)?..)?.try_into().ok()?) as usize;
    let end = d.len().checked_sub(4 + 4 * nr)?;
    let mut i = 0;
    let mut hs = Vec::new();
    while i < end {
        varint(d, &mut i)?;
        let ns = varint(d, &mut i)? as usize;
        let vl = varint(d, &mut i)? as usize;
        i += ns;
        let v = d.get(i..i + vl)?;
        i += vl;
        if handles {
            let mut j = 0;
            hs.push((varint(v, &mut j)? as usize, varint(v, &mut j)? as usize));
        } else {
            out.push(v.to_vec());
        }
    }
    Some(hs)
}

fn read_table(b: &[u8], out: &mut Vec<Vec<u8>>) {
    let _ = (|| -> Option<()> {
        let foot = b.get(b.len().checked_sub(48)?..)?;
        let mut i = 0;
        varint(foot, &mut i)?;
        varint(foot, &mut i)?;
        let (io, is) = (varint(foot, &mut i)? as usize, varint(foot, &mut i)? as usize);
        let block = |o: usize, s: usize| -> Option<Vec<u8>> {
            let d = b.get(o..o + s)?;
            match b.get(o + s)? {
                0 => Some(d.to_vec()),
                1 => unsnappy(d),
                _ => None,
            }
        };
        let index = block(io, is)?;
        for (o, s) in read_block_values(&index, out, true)? {
            if let Some(d) = block(o, s) {
                read_block_values(&d, out, false);
            }
        }
        Some(())
    })();
}

/// A LevelDB value of an object store: a varint version, then the IDB value.
fn leveldb_idb_value(v: &[u8]) -> Option<Value> {
    let at = v.iter().take(6).position(|&c| c == 0xff)?;
    idb_value(&v[at..])
}

// ---------------------------------------------------------------- where

/// Every claude.ai IndexedDB on this PC: the Claude desktop app first, then browsers.
fn sources() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(a) = env("APPDATA") {
        roots.push(a.join("Claude"));
    }
    if let Some(l) = env("LOCALAPPDATA") {
        // The Store (MSIX) build keeps a private copy of AppData.
        if let Ok(rd) = std::fs::read_dir(l.join("Packages")) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("claude") {
                    roots.push(e.path().join("LocalCache").join("Roaming").join("Claude"));
                }
            }
        }
        for b in [&["Google", "Chrome"][..], &["Microsoft", "Edge"], &["BraveSoftware", "Brave-Browser"]] {
            let user = b.iter().fold(l.clone(), |p, s| p.join(s)).join("User Data");
            if let Ok(rd) = std::fs::read_dir(&user) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n == "Default" || n.starts_with("Profile ") {
                        roots.push(e.path());
                    }
                }
            }
        }
    }
    roots
        .into_iter()
        .map(|r| r.join("IndexedDB").join("https_claude.ai_0.indexeddb.leveldb"))
        .filter(|p| p.is_dir())
        .collect()
}

fn blob_dir(leveldb: &Path) -> PathBuf {
    leveldb.with_file_name("https_claude.ai_0.indexeddb.blob")
}

fn walk(dir: &Path, depth: u32, f: &mut dyn FnMut(&Path, &std::fs::Metadata)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            if depth > 0 {
                walk(&e.path(), depth - 1, f);
            }
        } else {
            f(&e.path(), &m);
        }
    }
}

/// Changes whenever any cache file changes (size or time).
fn signature(dirs: &[PathBuf]) -> u64 {
    let mut sig: u64 = dirs.len() as u64;
    for d in dirs {
        let mut add = |_: &Path, m: &std::fs::Metadata| {
            let t = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            sig = sig.wrapping_mul(31).wrapping_add(t ^ m.len());
        };
        walk(d, 0, &mut add);
        walk(&blob_dir(d), 3, &mut add);
    }
    sig
}

// ---------------------------------------------------------------- reading

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn ms(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_f64).map(|x| x as u64).unwrap_or(0)
}

fn iso_ms(t: &str) -> u64 {
    let base = crate::usage::parse_time(t).unwrap_or(0) * 1000;
    // Keep the milliseconds so two updates in the same second still sort.
    let frac = t.split_once('.').map(|(_, f)| f.chars().take_while(char::is_ascii_digit).take(3).collect::<String>()).unwrap_or_default();
    base + format!("{frac:0<3}").parse::<u64>().unwrap_or(0)
}

/// A message's visible text (text blocks only: no thinking or tool calls).
fn message_text(m: &Value) -> String {
    let blocks: Vec<&str> = m
        .get("content")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|c| s(c, "type") == "text").map(|c| s(c, "text").trim()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default();
    if blocks.is_empty() { s(m, "text").trim().to_string() } else { blocks.join("\n\n") }
}

fn clip(t: &str, n: usize) -> String {
    if t.chars().count() <= n {
        t.to_string()
    } else {
        let mut s: String = t.chars().take(n).collect();
        s.push('…');
        s
    }
}

/// The active branch of a chat: from the leaf back to the first message.
fn active_path<'a>(msgs: &'a [Value], leaf: &str) -> Vec<&'a Value> {
    let by_id: HashMap<&str, &Value> = msgs.iter().map(|m| (s(m, "uuid"), m)).collect();
    let start = by_id.get(leaf).copied().or_else(|| {
        msgs.iter().max_by(|a, b| {
            let ia = a.get("index").and_then(Value::as_u64).unwrap_or(0);
            let ib = b.get("index").and_then(Value::as_u64).unwrap_or(0);
            ia.cmp(&ib).then_with(|| s(a, "created_at").cmp(s(b, "created_at")))
        })
    });
    let mut path = Vec::new();
    let mut cur = start;
    while let Some(m) = cur {
        if path.len() > msgs.len() {
            break; // a loop; never expected
        }
        path.push(m);
        cur = by_id.get(s(m, "parent_message_uuid")).copied();
    }
    path.reverse();
    path
}

/// A cached conversation (conversation store record).
struct Record {
    at: u64,
    name: String,
    updated_at: u64,
    leaf: String,
    messages: Vec<Value>,
}

fn record_of(v: &Value) -> Option<(String, Record)> {
    let uuid = s(v, "conversationUuid");
    let tree = v.get("tree")?;
    if uuid.is_empty() || s(v, "product") == "cowork" {
        return None;
    }
    let messages = tree.get("chat_messages").or_else(|| tree.get("messages")).and_then(Value::as_array)?.clone();
    let at = ms(v, "writtenAt").max(ms(v, "fetchedAt"));
    Some((
        uuid.to_string(),
        Record {
            at,
            name: s(tree, "name").to_string(),
            updated_at: ms(v, "conversationUpdatedAt").max(iso_ms(s(tree, "updated_at"))),
            leaf: s(tree, "current_leaf_message_uuid").to_string(),
            messages,
        },
    ))
}

#[derive(Default)]
struct Found {
    /// Newest react-query snapshot (timestamp, value).
    snapshot: Option<(u64, Value)>,
    records: HashMap<String, Record>,
}

impl Found {
    fn add(&mut self, v: Value) {
        if v.get("clientState").and_then(|c| c.get("queries")).is_some_and(Value::is_array) {
            let t = ms(&v, "timestamp");
            if self.snapshot.as_ref().is_none_or(|(old, _)| t > *old) {
                self.snapshot = Some((t, v));
            }
            return;
        }
        if let Some((uuid, r)) = record_of(&v) {
            if self.records.get(&uuid).is_none_or(|old| r.at > old.at) {
                self.records.insert(uuid, r);
            }
        }
    }
}

fn read_source(dir: &Path, found: &mut Found) {
    for v in leveldb_values(dir) {
        if let Some(x) = leveldb_idb_value(&v) {
            found.add(x);
        }
    }
    let mut blobs: Vec<PathBuf> = Vec::new();
    walk(&blob_dir(dir), 3, &mut |p, m| {
        if m.len() <= MAX_BLOB {
            blobs.push(p.to_path_buf());
        }
    });
    for p in blobs {
        if let Some(x) = std::fs::read(&p).ok().and_then(|b| idb_value(&b)) {
            found.add(x);
        }
    }
}

fn queries(snapshot: &Value) -> impl Iterator<Item = &Value> {
    snapshot["clientState"]["queries"].as_array().into_iter().flatten()
}

/// The plan of the account's chat organization, from its capabilities.
fn plan_of(snapshot: &Value) -> String {
    let Some(acct) = queries(snapshot).find(|q| q["queryKey"][0] == "current_account").map(|q| &q["state"]["data"]["account"]) else {
        return String::new();
    };
    let org = queries(snapshot)
        .find(|q| q["queryKey"][0] == "chat_conversation_list")
        .and_then(|q| q["queryKey"][1]["orgUuid"].as_str())
        .unwrap_or("");
    let ms = acct["memberships"].as_array().cloned().unwrap_or_default();
    let caps = |m: &Value| -> Vec<String> {
        m["organization"]["capabilities"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
    };
    let chosen = ms
        .iter()
        .find(|m| !org.is_empty() && m["organization"]["uuid"] == org)
        .or_else(|| ms.iter().find(|m| caps(m).iter().any(|c| c == "chat")));
    let Some(m) = chosen else { return String::new() };
    plan_from_caps(&caps(m))
}

fn plan_from_caps(caps: &[String]) -> String {
    let has = |c: &str| caps.iter().any(|x| x == c);
    if has("enterprise") {
        "enterprise"
    } else if has("raven") {
        "team"
    } else if has("claude_max") {
        "max"
    } else if has("claude_pro") {
        "pro"
    } else if has("chat") {
        "free"
    } else {
        ""
    }
    .into()
}

/// Chats listed in the snapshot (recent + starred), newest first.
fn listed(snapshot: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for q in queries(snapshot).filter(|q| q["queryKey"][0] == "chat_conversation_list") {
        let data = &q["state"]["data"];
        let items: Vec<&Value> = if let Some(pages) = data["pages"].as_array() {
            pages.iter().flat_map(|p| p["data"].as_array().or(p.as_array()).into_iter().flatten()).collect()
        } else {
            data["data"].as_array().or(data.as_array()).into_iter().flatten().collect()
        };
        for c in items {
            if !s(c, "uuid").is_empty() && c["is_temporary"] != true && !out.iter().any(|o| o["uuid"] == c["uuid"]) {
                out.push(c.clone());
            }
        }
    }
    out
}

/// Open chats also sit in the snapshot as `hub_transcript` queries.
fn transcripts(snapshot: &Value) -> Vec<(String, Record)> {
    queries(snapshot)
        .filter(|q| q["queryKey"][0] == "hub_transcript")
        .filter_map(|q| {
            let uuid = q["queryKey"][2]["uuid"].as_str()?.to_string();
            let messages = q["state"]["data"]["messages"].as_array()?.clone();
            let at = ms(&q["state"], "dataUpdatedAt");
            Some((uuid, Record { at, name: String::new(), updated_at: 0, leaf: String::new(), messages }))
        })
        .collect()
}

fn build(found: Found) -> ChatsView {
    let mut records = found.records;
    if let Some((_, snap)) = &found.snapshot {
        for (uuid, r) in transcripts(snap) {
            if records.get(&uuid).is_none_or(|old| r.at > old.at) {
                records.insert(uuid, r);
            }
        }
    }
    let mut view = ChatsView::default();
    let mut chats: Vec<Chat> = Vec::new();
    let mut leaf_of: HashMap<String, String> = HashMap::new();
    let mut has_list = false;
    if let Some((_, snap)) = &found.snapshot {
        view.available = true;
        view.plan = plan_of(snap);
        for c in listed(snap) {
            has_list = true;
            leaf_of.insert(s(&c, "uuid").into(), s(&c, "current_leaf_message_uuid").into());
            chats.push(Chat { uuid: s(&c, "uuid").into(), name: s(&c, "name").into(), updated_at: iso_ms(s(&c, "updated_at")), ..Default::default() });
        }
    }
    // Chats only in the conversation store (no list yet) still count.
    for (uuid, r) in &records {
        view.available = true;
        if !chats.iter().any(|c| &c.uuid == uuid) && !has_list {
            chats.push(Chat { uuid: uuid.clone(), name: r.name.clone(), updated_at: r.updated_at, ..Default::default() });
        }
    }
    chats.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    chats.truncate(MAX_CHATS);
    for c in &mut chats {
        let Some(r) = records.get(&c.uuid) else {
            c.behind = true;
            continue;
        };
        // Cached before the chat's last change (a second of slack for clocks).
        c.behind = r.at > 0 && r.at + 1000 < c.updated_at;
        let leaf = leaf_of.get(&c.uuid).filter(|l| !l.is_empty()).cloned().unwrap_or_else(|| r.leaf.clone());
        let path = active_path(&r.messages, &leaf);
        c.message_count = path.len() as u32;
        if c.name.is_empty() {
            c.name = r.name.clone();
        }
        if let Some(m) = path.iter().rev().find(|m| s(m, "sender") == "human") {
            let t = message_text(m);
            c.ask = (!t.is_empty()).then(|| clip(&t, ASK_CHARS));
        }
        if let Some(m) = path.iter().rev().find(|m| s(m, "sender") == "assistant") {
            let t = message_text(m);
            c.answer = (!t.is_empty()).then(|| clip(&t, ANSWER_CHARS));
            c.answered_at = iso_ms(s(m, "updated_at")).max(iso_ms(s(m, "created_at")));
        }
    }
    view.chats = chats;
    view
}

/// Read every source now (tests).
#[cfg(test)]
fn read_all() -> ChatsView {
    let mut found = Found::default();
    for d in sources() {
        read_source(&d, &mut found);
    }
    let mut view = build(found);
    if view.plan.is_empty() {
        view.plan = code_plan().unwrap_or_default();
    }
    view
}

/// Claude Code's own sign-in knows the plan too (Pro and up only).
pub fn code_plan() -> Option<String> {
    let text = std::fs::read_to_string(crate::paths::claude_dir().join(".credentials.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let p = v["claudeAiOauth"]["subscriptionType"].as_str()?.to_ascii_lowercase();
    (!p.is_empty()).then_some(p)
}

// ---------------------------------------------------------------- app

fn wanted(app: &AppHandle) -> bool {
    let st = app.state::<AppState>().settings.lock().unwrap().clone();
    st.modules.claude && st.claude.show_chats
}

pub fn current(app: &AppHandle) -> ChatsView {
    app.state::<ChatsState>().view.lock().unwrap().clone()
}

/// Watch the cache files; re-read and emit `chats` when they change.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_sig = u64::MAX;
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut since_scan = u32::MAX;
        loop {
            if !wanted(&app) {
                if last_sig != u64::MAX {
                    last_sig = u64::MAX;
                    set(&app, ChatsView::default());
                }
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            // Look for new caches (a browser signed in later) once a minute.
            if since_scan >= 20 {
                dirs = sources();
                since_scan = 0;
            }
            since_scan += 1;
            let sig = signature(&dirs);
            if sig != last_sig {
                last_sig = sig;
                let mut found = Found::default();
                for d in &dirs {
                    read_source(d, &mut found);
                }
                let mut view = build(found);
                if view.plan.is_empty() {
                    view.plan = code_plan().unwrap_or_default();
                }
                set(&app, view);
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn set(app: &AppHandle, view: ChatsView) {
    let state = app.state::<ChatsState>();
    let mut cur = state.view.lock().unwrap();
    if *cur != view {
        *cur = view.clone();
        drop(cur);
        let _ = app.emit("chats", view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snappy-compress as literals only (valid Snappy, enough for tests).
    fn snappy_literal(data: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        let mut n = data.len();
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                o.push(b);
                break;
            }
            o.push(b | 0x80);
        }
        for chunk in data.chunks(60) {
            o.push(((chunk.len() - 1) as u8) << 2);
            o.extend_from_slice(chunk);
        }
        o
    }

    fn v8_str(s: &str) -> Vec<u8> {
        let mut o = vec![b'"', s.len() as u8];
        o.extend_from_slice(s.as_bytes());
        o
    }

    /// {conversationUuid, product, fetchedAt, tree:{name, messages:[…]}} in V8 bytes.
    fn record_bytes() -> Vec<u8> {
        let mut o = vec![0xff, 0x10, b'o'];
        o.extend(v8_str("conversationUuid"));
        o.extend(v8_str("c1"));
        o.extend(v8_str("product"));
        o.extend(v8_str("hub"));
        o.extend(v8_str("fetchedAt"));
        o.push(b'N');
        o.extend(1000.0f64.to_le_bytes());
        o.extend(v8_str("tree"));
        o.push(b'o');
        o.extend(v8_str("name"));
        // two-byte string with the padding byte V8 writes before it
        o.push(0);
        o.push(b'c');
        let name: Vec<u8> = "Plan ☕".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        o.push(name.len() as u8);
        o.extend(&name);
        o.extend(v8_str("messages"));
        o.extend([b'A', 3]);
        let msg = |o: &mut Vec<u8>, id: &str, parent: &str, sender: &str, text: &str, idx: u8| {
            o.push(b'o');
            o.extend(v8_str("uuid"));
            o.extend(v8_str(id));
            o.extend(v8_str("parent_message_uuid"));
            o.extend(v8_str(parent));
            o.extend(v8_str("sender"));
            o.extend(v8_str(sender));
            o.extend(v8_str("index"));
            o.extend([b'I', idx * 2]);
            o.extend(v8_str("content"));
            o.extend([b'A', 2]);
            o.push(b'o');
            o.extend(v8_str("type"));
            o.extend(v8_str("thinking"));
            o.extend([b'{', 1]);
            o.push(b'o');
            o.extend(v8_str("type"));
            o.extend(v8_str("text"));
            o.extend(v8_str("text"));
            o.extend(v8_str(text));
            o.extend([b'{', 2]);
            o.extend([b'$', 0, 2]);
            o.extend([b'{', 6]);
        };
        msg(&mut o, "m1", "", "human", "hi there", 0);
        msg(&mut o, "m2", "m1", "assistant", "hello!", 1);
        // an edited-away branch: same parent, older, not on the leaf's path
        msg(&mut o, "m3", "m1", "assistant", "old reply", 1);
        o.extend([b'$', 0, 3]);
        o.extend([b'{', 2]);
        o.extend([b'{', 4]);
        o
    }

    #[test]
    fn snappy_round_trip_with_copies() {
        // "abcabcabcabc": literal "abc" then a copy of 9 bytes at offset 3.
        let b = [12u8, 0x08, b'a', b'b', b'c', ((9 - 4) << 2 | 1) as u8, 3];
        assert_eq!(unsnappy(&b).unwrap(), b"abcabcabcabc");
        assert!(unsnappy(&[5, 0x08, b'a']).is_none(), "truncated input");
    }

    #[test]
    fn decodes_a_compressed_blink_value_into_a_chat() {
        let v8 = record_bytes();
        // Blink 21 envelope with a trailer offset, then V8.
        let mut inner = vec![0xff, 0x15, 0xfe];
        inner.extend([0u8; 12]);
        inner.extend(&v8);
        let mut wrapped = vec![0xff, 0x11, 0x02];
        wrapped.extend(snappy_literal(&inner));
        let v = idb_value(&wrapped).expect("decodes");
        assert_eq!(v["tree"]["name"], "Plan ☕");
        assert_eq!(v["tree"]["messages"][1]["index"], 1);

        // As a LevelDB row (varint version prefix) it reads the same.
        let mut row = vec![0x8f, 0x08];
        row.extend(&inner);
        assert_eq!(leveldb_idb_value(&row).unwrap(), v);

        let mut found = Found::default();
        found.add(v);
        let view = build(found);
        assert!(view.available);
        let c = &view.chats[0];
        assert_eq!(c.name, "Plan ☕");
        assert_eq!(c.ask.as_deref(), Some("hi there"));
        assert_eq!(c.message_count, 2);
        // No leaf known: the newest index wins; ties break on created_at (none), so either
        // branch is acceptable, but the text must be a text block, never thinking.
        assert!(matches!(c.answer.as_deref(), Some("hello!") | Some("old reply")));
    }

    #[test]
    fn the_listed_leaf_picks_the_branch() {
        let v = idb_value(&record_bytes()).unwrap();
        let snap = serde_json::json!({
            "timestamp": 5.0,
            "clientState": { "queries": [
                { "queryKey": ["current_account", "x"], "state": { "data": { "account": { "memberships": [
                    { "organization": { "uuid": "api-org", "capabilities": ["api"] } },
                    { "organization": { "uuid": "org", "capabilities": ["chat", "claude_pro"] } }
                ] } } } },
                { "queryKey": ["chat_conversation_list", { "orgUuid": "org" }, "infinite", {}], "state": { "data": { "pages": [ { "data": [
                    { "uuid": "c0", "name": "Older", "updated_at": "2026-11-20T07:53:30.073417Z" },
                    { "uuid": "c1", "name": "Plan", "updated_at": "2026-11-21T07:48:29.595612Z", "current_leaf_message_uuid": "m3" },
                    { "uuid": "tmp", "name": "Temp", "updated_at": "2026-11-21T09:00:00Z", "is_temporary": true }
                ] } ] } } }
            ] }
        });
        let mut found = Found::default();
        found.add(v);
        found.add(snap);
        let view = build(found);
        assert_eq!(view.plan, "pro");
        assert_eq!(view.chats.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(), ["c1", "c0"]);
        assert_eq!(view.chats[0].name, "Plan", "the list's name is the current one");
        assert_eq!(view.chats[0].answer.as_deref(), Some("old reply"));
        assert_eq!(view.chats[0].updated_at, 1795247309595);
        assert_eq!(view.chats[1].ask, None, "not cached: no messages");
        assert!(view.chats[0].behind, "cached in 1970, changed in 2026");
        assert!(view.chats[1].behind, "never cached");
    }

    #[test]
    fn an_open_chat_reads_from_the_snapshot_transcript() {
        let msg = |id: &str, parent: &str, sender: &str, text: &str| {
            serde_json::json!({ "uuid": id, "parent_message_uuid": parent, "sender": sender, "index": 0,
                "content": [{ "type": "text", "text": text }], "created_at": "2026-11-21T14:33:40Z" })
        };
        let snap = serde_json::json!({
            "timestamp": 9.0,
            "clientState": { "queries": [
                { "queryKey": ["chat_conversation_list", { "orgUuid": "o" }, "infinite", {}], "state": { "data": { "pages": [ { "data": [
                    { "uuid": "c9", "name": "Ideas", "updated_at": "2026-11-21T14:33:38.228Z", "current_leaf_message_uuid": "b" }
                ] } ] } } },
                { "queryKey": ["hub_transcript", { "orgUuid": "o" }, { "uuid": "c9" }], "state": {
                    "dataUpdatedAt": 1795271625241.0,
                    "data": { "messages": [ msg("a", "", "human", "any ideas?"), msg("b", "a", "assistant", "Short answer: …") ] } } }
            ] }
        });
        let mut found = Found::default();
        found.add(snap);
        let c = &build(found).chats[0];
        assert_eq!(c.ask.as_deref(), Some("any ideas?"));
        assert_eq!(c.answer.as_deref(), Some("Short answer: …"));
        assert!(!c.behind, "saved after the chat's last change");
    }

    #[test]
    fn plans_from_capabilities() {
        let p = |c: &[&str]| plan_from_caps(&c.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(p(&["chat"]), "free");
        assert_eq!(p(&["chat", "claude_pro"]), "pro");
        assert_eq!(p(&["chat", "claude_max"]), "max");
        assert_eq!(p(&["api"]), "");
    }

    /// Reads this PC's real cache (if any) and prints a summary without message text.
    /// `cargo test -p crest real_cache -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_cache() {
        let t = std::time::Instant::now();
        let v = read_all();
        println!("{} sources, plan {:?}, {} chats, {:?}", sources().len(), v.plan, v.chats.len(), t.elapsed());
        for c in &v.chats {
            println!(
                "  {} upd {} msgs {} ask {} answer {} answered {} behind {}",
                &c.uuid[..8.min(c.uuid.len())],
                c.updated_at,
                c.message_count,
                c.ask.as_ref().map_or(0, |s| s.len()),
                c.answer.as_ref().map_or(0, |s| s.len()),
                c.answered_at,
                c.behind
            );
        }
        assert!(v.available);
    }
}
