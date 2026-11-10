use super::*;
use serde_json::json;

struct TempDb(PathBuf);
impl TempDb {
    fn new(name: &str) -> TempDb {
        let mut b = [0u8; 6];
        getrandom::fill(&mut b).unwrap();
        let dir = std::env::temp_dir().join(format!("atoll-recorder-{name}-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDb(dir)
    }
    fn path(&self) -> PathBuf {
        self.0.join("events.db")
    }
}
impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hook(session: &str, kind: &str, extra: Value) -> Value {
    let mut v = json!({ "session_id": session, "hook_event_name": kind, "cwd": "C:\\Users\\me\\proj\\atoll", "crest_entrypoint": "cli" });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

#[test]
fn records_redacts_searches_and_breaks_down() {
    let t = TempDb::new("basic");
    let store = Store::open(&t.path()).unwrap();
    let notices = Arc::new(Mutex::new(Vec::<String>::new()));
    let n2 = notices.clone();
    store.on_change(move |s| n2.lock().unwrap().push(s.to_string()));

    store.record_hook_at(hook("s1", "SessionStart", json!({ "source": "startup" })), 1_000);
    store.record_hook_at(hook("s1", "UserPromptSubmit", json!({ "prompt": "deploy with API_KEY=supersecret99 please" })), 2_000);
    store.record_hook_at(
        hook("s1", "PreToolUse", json!({ "tool_name": "Bash", "tool_use_id": "t1", "tool_input": { "command": "curl -H 'Authorization: Bearer abcdefgh12345' https://x" } })),
        3_000,
    );
    store.record_hook_at(hook("s1", "PostToolUse", json!({ "tool_name": "Bash", "tool_use_id": "t1", "tool_input": { "command": "curl" } })), 5_000);
    store.record_hook_at(
        hook("s1", "PreToolUse", json!({ "tool_name": "Edit", "tool_use_id": "t2", "agent_id": "agent-1", "agent_type": "Explore",
            "tool_input": { "file_path": "src/main.rs", "old_string": "fn a()", "new_string": "fn b()" } })),
        6_000,
    );
    store.record_hook_at(hook("s1", "SubagentStop", json!({ "agent_id": "agent-1", "agent_type": "Explore" })), 7_000);
    store.record_hook_at(hook("s1", "Stop", json!({ "last_assistant_message": "All done, renamed the function." })), 8_000);
    store.record_hook_at(hook("s1", "SessionEnd", json!({ "reason": "exit" })), 9_000);
    store.flush();

    let sessions = store.sessions(&SessionQuery::default()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].project, "atoll");
    assert_eq!(sessions[0].entrypoint, "cli");
    assert_eq!(sessions[0].event_count, 8);
    assert_eq!(sessions[0].ended_at, Some(9_000));

    let events = store.events(&EventQuery { session_id: Some("s1".into()), ..Default::default() }).unwrap();
    assert_eq!(events.len(), 8);
    assert_eq!(events[1].summary, "You: deploy with API_KEY=[redacted] please");
    assert!(events[2].summary.contains("Bearer [redacted]"), "{}", events[2].summary);

    // Nothing secret anywhere in the file's rows.
    for e in &events {
        let d = store.event(e.id).unwrap().unwrap();
        let all = format!("{} {} {}", d.row.summary, d.detail, d.payload);
        assert!(!all.contains("supersecret99") && !all.contains("abcdefgh12345"), "{all}");
    }

    let edit = store.event(events[4].id).unwrap().unwrap();
    assert_eq!(edit.diff, Some(vec![DiffHunk { path: "src/main.rs".into(), old: "fn a()".into(), new: "fn b()".into(), truncated: false }]));
    assert_eq!(edit.row.agent_id.as_deref(), Some("agent-1"));

    assert_eq!(store.search("renamed func", 10).unwrap().len(), 1);
    assert_eq!(store.search("\"unbalanced", 10).unwrap().len(), 0);
    assert!(store.search("supersecret99", 10).unwrap().is_empty());

    let tree = store.agent_tree("s1").unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].ended_at, Some(7_000));

    let b = store.breakdown("s1").unwrap();
    assert_eq!(b.total, 8_000);
    assert_eq!(b.tools[0], ("Bash".to_string(), 2_000));

    // Change notices are throttled per session but the last one is not lost.
    std::thread::sleep(Duration::from_millis(1200));
    store.flush();
    assert!(notices.lock().unwrap().iter().all(|s| s == "s1"));
    assert!(!notices.lock().unwrap().is_empty());
}

#[test]
fn usage_samples_are_deduplicated() {
    let t = TempDb::new("usage");
    let store = Store::open(&t.path()).unwrap();
    let s = |ts: i64, used: f64| UsageSample { ts, window: Window::FiveHour, used, resets_at: 99, source: "statusline".into() };
    store.record_usage(s(0, 10.0));
    store.record_usage(s(60_000, 10.0)); // same, too soon: skipped
    store.record_usage(s(120_000, 12.0)); // changed: kept
    store.record_usage(s(120_000 + USAGE_EVERY_MS, 12.0)); // unchanged but 10 min later: kept
    store.flush();
    let got: Vec<i64> = store.usage_samples(Window::FiveHour, 0).unwrap().iter().map(|u| u.ts).collect();
    assert_eq!(got, vec![0, 120_000, 120_000 + USAGE_EVERY_MS]);
    assert!(store.usage_samples(Window::SevenDay, 0).unwrap().is_empty());
}

#[test]
fn prune_and_delete_all() {
    let t = TempDb::new("prune");
    let store = Store::open(&t.path()).unwrap();
    let day = 86_400_000i64;
    let now = 100 * day;
    store.record_hook_at(hook("old", "Stop", json!({})), now - 40 * day);
    store.record_hook_at(hook("new", "Stop", json!({})), now - day);
    store.record_usage(UsageSample { ts: now - 40 * day, window: Window::SevenDay, used: 5.0, resets_at: 1, source: "log".into() });
    store.prune_at(30, now);
    store.flush();
    let s = store.stats().unwrap();
    assert_eq!((s.events, s.sessions, s.usage_samples), (1, 1, 0));
    assert_eq!(store.search("Finished", 10).unwrap().len(), 1);

    store.delete_all().unwrap();
    let s = store.stats().unwrap();
    assert_eq!((s.events, s.sessions, s.usage_samples, s.dropped), (0, 0, 0, 0));
    assert!(store.search("Finished", 10).unwrap().is_empty());
    // Still usable afterwards.
    store.record_hook_at(hook("again", "Stop", json!({})), now);
    store.flush();
    assert_eq!(store.stats().unwrap().events, 1);
}

#[test]
fn reopening_keeps_data_and_schema() {
    let t = TempDb::new("reopen");
    {
        let store = Store::open(&t.path()).unwrap();
        store.record_hook_at(hook("s", "Stop", json!({})), 5);
        store.flush();
    }
    let store = Store::open(&t.path()).unwrap();
    assert_eq!(store.stats().unwrap().events, 1);
}

// ------------------------------------------------------------- simulation + benchmark

/// A deterministic pseudo-random generator (no extra crates).
pub(crate) struct Rng(u64);
impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1)
    }
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub(crate) fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

const TOOLS: &[(&str, &str)] = &[
    ("Read", "file_path"),
    ("Edit", "file_path"),
    ("Bash", "command"),
    ("Grep", "pattern"),
    ("Glob", "pattern"),
    ("Write", "file_path"),
    ("WebFetch", "url"),
];

/// One simulated working day of hook events, shaped like real sessions:
/// a few sessions, each with turns of 2–25 tool calls, some edits carrying
/// code, the odd permission request and subagent.
pub(crate) fn simulated_day(day_start: i64, rng: &mut Rng) -> Vec<(i64, Value)> {
    let mut out = Vec::new();
    let sessions = rng.range(2, 5);
    let mut t = day_start + 9 * 3_600_000;
    for s in 0..sessions {
        let sid = format!("sim-{day_start}-{s}");
        out.push((t, hook(&sid, "SessionStart", json!({ "source": "startup" }))));
        for turn in 0..rng.range(4, 14) {
            t += rng.range(20_000, 600_000) as i64;
            let words = rng.range(8, 60) as usize;
            out.push((t, hook(&sid, "UserPromptSubmit", json!({ "prompt": "please change the thing ".repeat(words / 4 + 1) }))));
            for call in 0..rng.range(2, 25) {
                let (tool, field) = TOOLS[rng.range(0, TOOLS.len() as u64 - 1) as usize];
                let id = format!("tu-{turn}-{call}");
                let agent = (rng.unit() < 0.1).then(|| format!("agent-{turn}"));
                let mut input = json!({ field: format!("src/lib/module_{}.rs", rng.range(1, 40)) });
                if tool == "Edit" {
                    let n = rng.range(80, 1500) as usize;
                    input["old_string"] = json!("let x = 1;\n".repeat(n / 11 + 1));
                    input["new_string"] = json!("let x = 2;\n".repeat(n / 11 + 1));
                }
                if tool == "Bash" {
                    input["command"] = json!(format!("cargo test --lib recorder::{} -- --nocapture", rng.range(1, 99)));
                }
                let mut pre = json!({ "tool_name": tool, "tool_use_id": id, "tool_input": input });
                if let Some(a) = &agent {
                    pre["agent_id"] = json!(a);
                    pre["agent_type"] = json!("Explore");
                }
                t += rng.range(500, 20_000) as i64;
                out.push((t, hook(&sid, "PreToolUse", pre.clone())));
                if rng.unit() < 0.05 {
                    t += rng.range(2_000, 60_000) as i64;
                    out.push((t, hook(&sid, "PermissionRequest", pre.clone())));
                }
                t += rng.range(100, 30_000) as i64;
                out.push((t, hook(&sid, "PostToolUse", pre)));
            }
            t += rng.range(2_000, 30_000) as i64;
            out.push((t, hook(&sid, "Stop", json!({ "last_assistant_message": "Done. I changed the module and the tests pass. ".repeat(rng.range(1, 8) as usize) }))));
        }
        t += rng.range(60_000, 3_600_000) as i64;
        out.push((t, hook(&sid, "SessionEnd", json!({ "reason": "exit" }))));
    }
    out
}

/// Write cost per event and size growth per day. Run with:
/// `cargo test -p crest --lib recorder::tests::bench_write_cost -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_write_cost() {
    let t = TempDb::new("bench");
    let path = t.path();
    let days = 14;
    let mut rng = Rng::new(7);
    let all: Vec<Vec<(i64, Value)>> = (0..days).map(|d| simulated_day(d as i64 * 86_400_000, &mut rng)).collect();
    let total: usize = all.iter().map(Vec::len).sum();

    // 1. Caller-side cost: what the hook handler pays (clone + try_send).
    let store = Store::open(&path).unwrap();
    let sample: Vec<Value> = all[0].iter().take(500).map(|(_, v)| v.clone()).collect();
    let mut enqueue = Vec::new();
    for v in &sample {
        let t0 = Instant::now();
        store.record_hook(v);
        enqueue.push(t0.elapsed().as_nanos() as f64 / 1000.0);
    }
    store.flush();
    store.delete_all().unwrap();

    // 2. Writer-side cost, one event at a time (worst case: one commit each).
    let mut single = Vec::new();
    for (ts, v) in all[0].iter().take(500) {
        let t0 = Instant::now();
        store.record_hook_at(v.clone(), *ts);
        store.flush();
        single.push(t0.elapsed().as_nanos() as f64 / 1000.0);
    }
    store.delete_all().unwrap();

    // 3. Growth: write every simulated day, measure the file after each.
    let mut sizes = Vec::new();
    let t0 = Instant::now();
    for day in &all {
        for (ts, v) in day {
            store.record_hook_at(v.clone(), *ts);
        }
        store.flush();
        sizes.push(store.stats().unwrap().file_bytes);
    }
    let bulk = t0.elapsed();
    let st = store.stats().unwrap();

    let pct = |v: &mut Vec<f64>, p: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * p) as usize]
    };
    println!("events simulated: {total} over {days} days ({:.0}/day)", total as f64 / days as f64);
    println!("enqueue (hook handler): p50 {:.1} us, p95 {:.1} us", pct(&mut enqueue, 0.5), pct(&mut enqueue, 0.95));
    println!("single write incl. commit: p50 {:.0} us, p95 {:.0} us", pct(&mut single, 0.5), pct(&mut single, 0.95));
    println!("bulk write: {:.1} us/event", bulk.as_secs_f64() * 1e6 / total as f64);
    println!("file after {days} days: {} bytes ({:.0} KB/day, {:.0} bytes/event)", st.file_bytes, st.file_bytes as f64 / 1024.0 / days as f64, st.file_bytes as f64 / total as f64);
    println!("size by day (KB): {:?}", sizes.iter().map(|b| b / 1024).collect::<Vec<_>>());
    let t1 = Instant::now();
    let hits = store.search("module_12", 50).unwrap().len();
    println!("search over {} events: {} hits in {:.1} ms", st.events, hits, t1.elapsed().as_secs_f64() * 1e3);
    assert_eq!(st.events as usize, total);
    assert_eq!(st.dropped, 0);
}
