//! The universal search index (owned by the `search-clipboard` branch).
//!
//! In memory only, by design: everything it holds is rebuilt from the
//! modules' own stores at start-up, so it never writes a second, unencrypted
//! copy of your clipboard, notes or agent history to disk.
//!
//! - Words are lower-cased and split on anything that isn't a letter or digit;
//!   each Chinese / Japanese / Korean character is its own word, so CJK text
//!   matches without spaces.
//! - A query matches a document when every query word is the start of some
//!   word in it ("rep" finds "Report.pdf").
//! - Score = how well it matched (whole words, words in the title) +
//!   recency + pins.
//! - Updates are incremental: `upsert` / `remove` touch one document. Removed
//!   documents leave a tombstone; the index compacts itself when over half
//!   of it is tombstones.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Clipboard,
    Whiteboard,
    Note,
    FileTray,
    Calendar,
    Agent,
}

impl Source {
    pub const ALL: [Source; 6] = [Source::Clipboard, Source::Whiteboard, Source::Note, Source::FileTray, Source::Calendar, Source::Agent];
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    /// Unique across sources, e.g. "clipboard:<id>".
    pub key: String,
    pub source: Source,
    /// One line shown in the result.
    pub title: String,
    /// Second line (source detail: app, path, time…).
    pub sub: String,
    /// Extra searchable text that isn't shown (full text, OCR, paths).
    #[serde(skip)]
    pub body: String,
    /// ms since epoch: copy time, edit time, event start…
    pub at: u64,
    pub pinned: bool,
    /// Kind within the source, e.g. "text" / "image" / "files" for clips.
    pub kind: String,
}

/// The stored document keeps no body: once its words are indexed only a
/// hash is kept, to tell whether a re-read changed anything.
struct Slot {
    doc: Doc,
    title_lc: String,
    body_hash: u64,
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Longest text read from one document (title + body), in characters.
const MAX_INDEXED_CHARS: usize = 2_000;
const MAX_TOKEN: usize = 40;

#[derive(Default)]
pub struct Index {
    slots: Vec<Option<Slot>>,
    by_key: HashMap<String, u32>,
    /// word -> word id
    dict: BTreeMap<Box<str>, u32>,
    /// word id -> slots containing it (may point at tombstones)
    postings: Vec<Vec<u32>>,
    dead: usize,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Lower-cased words of `text`, at most `MAX_INDEXED_CHARS` characters read.
pub fn tokens(text: &str, out: &mut Vec<String>) {
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut Vec<String>| {
        if !w.is_empty() {
            out.push(w.chars().take(MAX_TOKEN).collect());
            w.clear();
        }
    };
    for c in text.chars().take(MAX_INDEXED_CHARS) {
        if is_cjk(c) {
            flush(&mut word, out);
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else {
            flush(&mut word, out);
        }
    }
    flush(&mut word, out);
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    #[serde(flatten)]
    pub doc: Doc,
    pub score: f32,
}

impl Index {
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn get(&self, key: &str) -> Option<Doc> {
        self.by_key.get(key).and_then(|&s| self.slots[s as usize].as_ref()).map(|s| s.doc.clone())
    }

    pub fn upsert(&mut self, doc: Doc) {
        self.remove(&doc.key.clone());
        let slot = self.slots.len() as u32;
        let mut words = Vec::new();
        tokens(&doc.title, &mut words);
        let left = MAX_INDEXED_CHARS.saturating_sub(doc.title.chars().count());
        tokens(&doc.body.chars().take(left).collect::<String>(), &mut words);
        // The sub line starts with the source's name ("Clipboard · Link · Chrome");
        // index only what follows, or "clip" would match every clipboard item.
        tokens(doc.sub.split_once(" · ").map_or("", |(_, rest)| rest), &mut words);
        words.sort_unstable();
        words.dedup();
        for w in words {
            let id = match self.dict.get(w.as_str()) {
                Some(&id) => id,
                None => {
                    let id = self.postings.len() as u32;
                    self.dict.insert(w.into_boxed_str(), id);
                    self.postings.push(Vec::new());
                    id
                }
            };
            self.postings[id as usize].push(slot);
        }
        self.by_key.insert(doc.key.clone(), slot);
        let title_lc = doc.title.to_lowercase();
        let body_hash = hash(&doc.body);
        let doc = Doc { body: String::new(), ..doc };
        self.slots.push(Some(Slot { doc, title_lc, body_hash }));
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(slot) = self.by_key.remove(key) {
            self.slots[slot as usize] = None;
            self.dead += 1;
            if self.dead > 1_000 && self.dead * 2 > self.slots.len() {
                self.compact();
            }
        }
    }

    /// Replace every document of one source (small sources are re-read whole).
    pub fn replace_source(&mut self, source: Source, docs: Vec<Doc>) {
        let old: Vec<String> = self.slots.iter().flatten().filter(|s| s.doc.source == source).map(|s| s.doc.key.clone()).collect();
        let keep: std::collections::HashSet<&str> = docs.iter().map(|d| d.key.as_str()).collect();
        for k in old.iter().filter(|k| !keep.contains(k.as_str())) {
            self.remove(k);
        }
        for d in docs {
            // Skip unchanged documents so a re-read touches nothing.
            if let Some(&s) = self.by_key.get(&d.key) {
                if let Some(cur) = &self.slots[s as usize] {
                    let c = &cur.doc;
                    if c.title == d.title && cur.body_hash == hash(&d.body) && c.sub == d.sub && c.at == d.at && c.pinned == d.pinned {
                        continue;
                    }
                }
            }
            self.upsert(d);
        }
    }

    pub fn clear_source(&mut self, source: Source) {
        self.replace_source(source, Vec::new());
    }

    /// Give back the spare capacity growing lists keep (after a bulk load).
    pub fn shrink(&mut self) {
        for p in &mut self.postings {
            p.shrink_to_fit();
        }
        self.postings.shrink_to_fit();
        self.slots.shrink_to_fit();
    }

    /// Rebuild without tombstones.
    fn compact(&mut self) {
        let docs: Vec<Doc> = std::mem::take(&mut self.slots).into_iter().flatten().map(|s| s.doc).collect();
        *self = Index::default();
        for d in docs {
            self.upsert(d);
        }
    }

    /// Best `limit` matches for `query`, optionally from one source.
    /// An empty query lists the most recent documents (pins first).
    pub fn search(&self, query: &str, limit: usize, only: Option<Source>, now: u64) -> Vec<Hit> {
        let mut terms = Vec::new();
        tokens(query, &mut terms);
        terms.sort_unstable();
        terms.dedup();
        let n = self.slots.len();
        let live = |s: u32| self.slots[s as usize].as_ref().filter(|x| only.is_none_or(|o| x.doc.source == o));

        let mut scored: Vec<(f32, u32)> = Vec::new();
        if terms.is_empty() {
            for (i, s) in self.slots.iter().enumerate() {
                if let Some(s) = s.as_ref().filter(|x| only.is_none_or(|o| x.doc.source == o)) {
                    scored.push((rank(&s.doc, 0.0, now), i as u32));
                }
            }
        } else {
            // matched[d] = number of terms matched; exact[d] = how many as whole words.
            let mut matched = vec![0u8; n];
            let mut exact = vec![0u8; n];
            let mut seen = vec![u16::MAX; n];
            let mut seen_exact = vec![u16::MAX; n];
            for (t, term) in terms.iter().enumerate() {
                let t = t as u16;
                let exact_id = self.dict.get(term.as_str()).copied();
                for (_, &id) in self.dict.range::<str, _>((std::ops::Bound::Included(term.as_str()), std::ops::Bound::Unbounded)).take_while(|(w, _)| w.starts_with(term.as_str())) {
                    for &s in &self.postings[id as usize] {
                        let s = s as usize;
                        if seen[s] != t && matched[s] == t as u8 {
                            seen[s] = t;
                            matched[s] += 1;
                        }
                        if Some(id) == exact_id && seen_exact[s] != t {
                            seen_exact[s] = t;
                            exact[s] += 1;
                        }
                    }
                }
            }
            let want = terms.len() as u8;
            for s in 0..n {
                if matched[s] != want {
                    continue;
                }
                let Some(slot) = live(s as u32) else { continue };
                let whole = exact[s] as f32 / want as f32;
                let in_title = terms.iter().filter(|t| slot.title_lc.contains(t.as_str())).count() as f32 / want as f32;
                scored.push((rank(&slot.doc, 3.0 * whole + 2.0 * in_title, now), s as u32));
            }
        }

        let k = limit.min(scored.len());
        if k == 0 {
            return Vec::new();
        }
        let cmp = |a: &(f32, u32), b: &(f32, u32)| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1));
        if scored.len() > k {
            scored.select_nth_unstable_by(k - 1, cmp);
            scored.truncate(k);
        }
        scored.sort_unstable_by(cmp);
        scored.into_iter().map(|(score, s)| Hit { doc: self.slots[s as usize].as_ref().unwrap().doc.clone(), score }).collect()
    }

    /// Approximate memory held by the index, in bytes.
    pub fn bytes(&self) -> usize {
        let docs: usize = self
            .slots
            .iter()
            .flatten()
            // Heap text only; the structs themselves are counted in `tombs` (the slot array).
            .map(|s| s.doc.key.len() * 2 + s.doc.title.len() + s.title_lc.len() + s.doc.sub.len() + s.doc.kind.len())
            .sum();
        let tombs = self.slots.len() * std::mem::size_of::<Option<Slot>>();
        let dict: usize = self.dict.keys().map(|k| k.len() + 16 + 8).sum::<usize>() + self.dict.len() * 16;
        let posts: usize = self.postings.iter().map(|p| p.capacity() * 4 + 24).sum();
        let keys = self.by_key.len() * (24 + 4 + 16);
        docs + tombs + dict + posts + keys
    }
}

/// Match quality + recency (hyperbolic, half weight after a day) + pin.
fn rank(d: &Doc, quality: f32, now: u64) -> f32 {
    let age_h = now.saturating_sub(d.at) as f32 / 3_600_000.0;
    // Calendar events are about the future: closeness either way counts.
    let age_h = if d.source == Source::Calendar { (d.at as f32 - now as f32).abs() / 3_600_000.0 } else { age_h };
    let recency = 1.0 / (1.0 + age_h / 24.0);
    quality + 2.0 * recency + if d.pinned { 2.5 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(key: &str, source: Source, title: &str, body: &str, at: u64, pinned: bool) -> Doc {
        Doc { key: key.into(), source, title: title.into(), sub: String::new(), body: body.into(), at, pinned, kind: "text".into() }
    }

    const NOW: u64 = 1_800_000_000_000;
    const H: u64 = 3_600_000;

    #[test]
    fn prefix_and_and() {
        let mut ix = Index::default();
        ix.upsert(doc("a", Source::Clipboard, "Quarterly report draft", "", NOW, false));
        ix.upsert(doc("b", Source::FileTray, "Report.pdf", "C:\\Users\\me\\Desktop\\Report.pdf", NOW, false));
        ix.upsert(doc("c", Source::Note, "buy milk", "", NOW, false));
        let keys = |q: &str| ix.search(q, 10, None, NOW).into_iter().map(|h| h.doc.key).collect::<Vec<_>>();
        let mut r = keys("rep");
        r.sort();
        assert_eq!(r, ["a", "b"]);
        assert_eq!(keys("rep pdf"), ["b"]);
        assert_eq!(keys("desktop"), ["b"]);
        assert!(keys("zzz").is_empty());
        assert_eq!(ix.search("rep", 10, Some(Source::Note), NOW).len(), 0);
    }

    #[test]
    fn ranks_whole_words_title_recency_and_pins() {
        let mut ix = Index::default();
        ix.upsert(doc("old", Source::Clipboard, "report", "", NOW - 30 * 24 * H, false));
        ix.upsert(doc("new", Source::Clipboard, "report", "", NOW - H, false));
        assert_eq!(ix.search("report", 10, None, NOW)[0].doc.key, "new");
        // Pins come first among equally good matches, as in the clipboard tab.
        ix.upsert(doc("pin", Source::Clipboard, "report", "", NOW - 30 * 24 * H, true));
        assert_eq!(ix.search("report", 10, None, NOW)[0].doc.key, "pin");
        // A better match still beats a pin: whole word in the title vs a body prefix.
        ix.upsert(doc("pin2", Source::Clipboard, "x", "reporting", NOW - 30 * 24 * H, true));
        let r: Vec<String> = ix.search("report", 10, None, NOW).into_iter().map(|h| h.doc.key).collect();
        assert!(r.iter().position(|k| k == "new") < r.iter().position(|k| k == "pin2"));
        // A pin beats a week of age.
        ix.upsert(doc("week", Source::Clipboard, "report", "", NOW - 7 * 24 * H, false));
        let r: Vec<String> = ix.search("report", 10, None, NOW).into_iter().map(|h| h.doc.key).collect();
        assert!(r.iter().position(|k| k == "pin") < r.iter().position(|k| k == "week"));
        // Whole word in the title beats a prefix in the body.
        let mut ix = Index::default();
        ix.upsert(doc("body", Source::Note, "x", "cats everywhere", NOW, false));
        ix.upsert(doc("title", Source::Note, "cat", "", NOW, false));
        assert_eq!(ix.search("cat", 10, None, NOW)[0].doc.key, "title");
    }

    #[test]
    fn incremental_updates_and_compaction() {
        let mut ix = Index::default();
        for i in 0..3_000 {
            ix.upsert(doc(&format!("k{i}"), Source::Agent, &format!("event {i}"), "", NOW, false));
        }
        for i in 0..2_000 {
            ix.remove(&format!("k{i}"));
        }
        assert_eq!(ix.len(), 1_000);
        assert!(ix.slots.len() < 3_000, "compacted");
        assert_eq!(ix.search("event", 5_000, None, NOW).len(), 1_000);
        ix.upsert(doc("k2500", Source::Agent, "renamed", "", NOW, false));
        assert_eq!(ix.search("renamed", 10, None, NOW).len(), 1);
        assert_eq!(ix.search("event", 5_000, None, NOW).len(), 999);
    }

    #[test]
    fn replace_source_keeps_others() {
        let mut ix = Index::default();
        ix.upsert(doc("n1", Source::Note, "alpha", "", NOW, false));
        ix.upsert(doc("t1", Source::FileTray, "alpha.txt", "", NOW, false));
        ix.replace_source(Source::Note, vec![doc("n2", Source::Note, "beta", "", NOW, false)]);
        let r: Vec<String> = ix.search("", 10, None, NOW).into_iter().map(|h| h.doc.key).collect();
        assert!(r.contains(&"n2".into()) && r.contains(&"t1".into()) && !r.contains(&"n1".into()));
    }

    #[test]
    fn source_name_in_sub_is_not_searchable() {
        let mut ix = Index::default();
        let mut d = doc("a", Source::Agent, "Edited index.rs", "", NOW, false);
        d.sub = "Agent history · PostToolUse · Edit".into();
        ix.upsert(d);
        assert!(ix.search("hist", 10, None, NOW).is_empty());
        assert_eq!(ix.search("posttooluse", 10, None, NOW).len(), 1);
    }

    #[test]
    fn cjk_and_case() {
        let mut ix = Index::default();
        ix.upsert(doc("zh", Source::Note, "历史考试复习", "", NOW, false));
        ix.upsert(doc("en", Source::Note, "ÉCOLE Notes", "", NOW, false));
        assert_eq!(ix.search("考试", 10, None, NOW)[0].doc.key, "zh");
        assert_eq!(ix.search("école", 10, None, NOW)[0].doc.key, "en");
    }

    // ---------- benchmark: 50,000 documents ----------

    /// Run with: cargo test --release --lib search::index::tests::bench_50k -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_50k() {
        let words: Vec<String> = (0..20_000).map(|i| synth_word(i)).collect();
        let mut rng = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let sources = Source::ALL;
        let private_before = private_bytes();
        let t0 = std::time::Instant::now();
        let mut ix = Index::default();
        for i in 0..50_000u64 {
            // Mixed lengths: short titles, some long bodies (agent events, notes, clips).
            let tl = 3 + (next() % 8) as usize;
            let bl = if i % 5 == 0 { 120 } else { (next() % 40) as usize };
            // Zipf-ish: common words much more often than rare ones.
            // Squared uniform: low word numbers (common words) come up far more often.
            let mut pick = || {
                let u = (next() % 1_000_000) as f64 / 1_000_000.0;
                words[(u * u * 20_000.0) as usize].clone()
            };
            let title: Vec<String> = (0..tl).map(|_| pick()).collect();
            let body: Vec<String> = (0..bl).map(|_| pick()).collect();
            ix.upsert(Doc {
                key: format!("k{i}"),
                source: sources[(i % 6) as usize],
                title: title.join(" "),
                sub: "VS Code · 3 min ago".into(),
                body: body.join(" "),
                at: NOW - (next() % (90 * 24 * H)),
                pinned: i % 500 == 0,
                kind: "text".into(),
            });
        }
        ix.shrink();
        let build = t0.elapsed();
        println!(
            "measured: process private memory grew {:.1} MB while building (includes the synthetic word list, ~0.3 MB)",
            (private_bytes().saturating_sub(private_before)) as f64 / 1e6
        );
        println!("built 50,000 docs in {build:?}; {} distinct words; index ≈ {:.1} MB in memory", ix.dict.len(), ix.bytes() as f64 / 1e6);

        let queries: Vec<String> = (0..400)
            .map(|i| match i % 4 {
                0 => words[(next() % 200) as usize][..1].to_string(),
                1 => words[(next() % 2_000) as usize][..2].to_string(),
                2 => words[(next() % 20_000) as usize][..3].to_string(),
                _ => format!("{} {}", &words[(next() % 500) as usize][..3], &words[(next() % 5_000) as usize][..2]),
            })
            .collect();
        let labels = ["1 char", "2 chars", "3 chars", "two words"];
        for (k, label) in labels.iter().enumerate() {
            let mut times: Vec<f64> = Vec::new();
            let mut hits = 0;
            for q in queries.iter().skip(k).step_by(4) {
                let t = std::time::Instant::now();
                let r = ix.search(q, 50, None, NOW);
                times.push(t.elapsed().as_secs_f64() * 1e3);
                hits += r.len();
            }
            times.sort_by(f64::total_cmp);
            let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
            println!("{label:>9}: p50 {:.2} ms  p95 {:.2} ms  max {:.2} ms  (avg {} results)", p(0.5), p(0.95), p(1.0), hits / times.len());
        }
        let t = std::time::Instant::now();
        for i in 0..1_000u64 {
            ix.upsert(doc(&format!("k{i}"), Source::Clipboard, "updated item", "", NOW, false));
        }
        println!("1,000 incremental updates: {:.3} ms each", t.elapsed().as_secs_f64() * 1e3 / 1_000.0);
        let t = std::time::Instant::now();
        let r = ix.search("", 50, None, NOW);
        println!("empty query (recent list): {:.2} ms, {} results", t.elapsed().as_secs_f64() * 1e3, r.len());
    }

    /// Idle CPU of the indexing worker with 50,000 documents loaded.
    /// Run with: cargo test --release --lib search::index::tests::idle_cpu -- --ignored --nocapture
    #[test]
    #[ignore]
    fn idle_cpu() {
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::{Duration, Instant};
        let ix = Arc::new(Mutex::new(Index::default()));
        {
            let mut g = ix.lock().unwrap();
            for i in 0..50_000u64 {
                g.upsert(doc(&format!("agent:{i}"), Source::Agent, &format!("Edited file number {i} in project"), "", NOW - i * 1_000, false));
            }
            g.shrink();
        }
        // The same shape as search::start: block on a channel, settle 250 ms, re-read the source.
        let (tx, rx) = mpsc::channel::<Source>();
        let worker_ix = ix.clone();
        std::thread::spawn(move || {
            while let Ok(_first) = rx.recv() {
                while rx.recv_timeout(Duration::from_millis(250)).is_ok() {}
                let docs: Vec<Doc> = (0..200u64).map(|i| doc(&format!("clipboard:{i}"), Source::Clipboard, &format!("copied text {i} {}", crate::media::now_ms()), "", NOW, false)).collect();
                worker_ix.lock().unwrap().replace_source(Source::Clipboard, docs);
            }
        });
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
        let measure = |label: &str, secs: u64, poke: Option<&mpsc::Sender<Source>>| {
            let (c0, t0) = (cpu_seconds(), Instant::now());
            while t0.elapsed() < Duration::from_secs(secs) {
                if let Some(tx) = poke {
                    let _ = tx.send(Source::Clipboard);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            let cpu = cpu_seconds() - c0;
            let wall = t0.elapsed().as_secs_f64();
            println!("{label}: {:.3} % of the whole CPU ({cpus} logical cores), {:.0} ms CPU in {:.0} s", cpu / wall / cpus * 100.0, cpu * 1e3, wall);
        };
        measure("idle, index loaded, worker waiting", 30, None);
        measure("re-indexing 200 clipboard items every second", 30, Some(&tx));
    }

    /// CPU time (user + kernel) this process has used, in seconds.
    fn cpu_seconds() -> f64 {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut a, mut b, mut k, mut u) = (z, z, z, z);
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) };
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
        t(k) + t(u)
    }

    /// This process's private bytes, read with PowerShell (no extra crate features).
    fn private_bytes() -> u64 {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("(Get-Process -Id {}).PrivateMemorySize64", std::process::id())])
            .output()
            .expect("powershell");
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
    }

    fn synth_word(i: usize) -> String {
        const SYL: [&str; 16] = ["ka", "lo", "mi", "re", "su", "ta", "ne", "po", "an", "el", "ri", "co", "du", "ve", "sh", "qu"];
        let mut n = i + 7;
        let mut w = String::new();
        while w.len() < 4 || n > 0 {
            w.push_str(SYL[n % 16]);
            n /= 16;
        }
        w
    }
}
