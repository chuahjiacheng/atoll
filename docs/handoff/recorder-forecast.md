# Handoff: recorder-forecast

Status: **Step A (recorder core) built, awaiting approval.** Steps B (Agent tab
UI) and C (forecast) not started.

## Note on branches

This folder is not a git repository, so there is no `recorder-forecast`
branch or worktree. Every change below was made directly in the shared
folder. Shared files carry `recorder-forecast` markers so the edits can be
found and moved later.

## What changed (Step A)

New, owned by this session:
- `src-tauri/src/recorder/mod.rs`: the store (schema v1, WAL, one writer thread, reads, full-text search).
- `src-tauri/src/recorder/redact.rs`: secret redaction (event-store.md §3).
- `src-tauri/src/recorder/breakdown.rs`: per-session time breakdown.
- `src-tauri/src/recorder/commands.rs`: Tauri state, start-up, 6-hourly prune, `recorder_*` commands.
- `src-tauri/src/recorder/tests.rs`: tests, simulated-day generator, write-cost benchmark.
- `docs/contracts/event-store.md`: the contract (v1.1).

Shared files, one marked block or line each:
- `src-tauri/Cargo.toml`: `rusqlite 0.32 (bundled)`, `regex 1`.
- `src-tauri/src/lib.rs`: `mod recorder;`, `recorder::commands::start` in setup (before the listener), settings-changed hook, 9 commands.
- `src-tauri/src/server.rs`: `record_hook` at the top of `handle_hook` (after the "following is off" check).
- `src-tauri/src/usage.rs`: `record_usage` after each of the 3 updates (log, sign-in, status line). Milestone 2 behaviour unchanged.
- `src-tauri/src/settings.rs`: `Recorder { enabled: true, retention_days: 30 }`, clamp 0..=3650.
- `src-tauri/src/data.rs`: "Clear all data" also deletes the recorder history.

Not touched: `hooks.rs` (SubagentStart/Stop not added, waiting for the
owner's decision), `bridge/`, any Svelte file.

## How to test

```
cargo test -p crest --lib recorder
cargo test -p crest --lib recorder::tests::bench_write_cost -- --ignored --nocapture
```

Live: run Atoll, use Claude Code for a few turns, then open
`%LOCALAPPDATA%\Crest\recorder\events.db` in any SQLite viewer (read-only).
There is no UI yet (Step B).

## Measured (Step A, 2026-11-10)

Benchmark: 14 simulated days, 13,586 events (about 970 a day), release build
without LTO, on the owner's PC. The simulated days are synthetic, so real
sizes depend on real usage [Inference].

| What | Result |
| --- | --- |
| Cost to the hook handler (clone + queue) | p50 1.8 µs, p95 2.8 µs |
| One event written alone, with commit | p50 169 µs, p95 386 µs |
| Bulk write (batched) | 65 µs per event |
| Database growth | about 1.35 MB a day, about 1.4 KB an event (file + write-ahead log) |
| Size with the default 30-day retention | about 40 MB [Inference, extrapolated] |
| Search over 13,586 events | 1.6 ms |
| Events dropped | 0 |
| Idle CPU | Writer thread blocks on its queue; prune thread sleeps 6 h. No polling. |

## Open issues

- Subagent start/stop needs `SubagentStart`/`SubagentStop` in the installed
  hooks (owner decision). Tool events from subagents already carry
  `agent_id` per the hooks docs [Unverified on the installed version].
- No parent id exists in hook inputs, so the subagent tree is one level deep.
- The bridge cuts strings at 4,000 characters, so long edits show cut short
  in the diff view (`truncated: true`).
- Pill warning needs a new `usage` state id in `width-contract.md`
  (owner: `adaptive-island`); request to be filed in Step C.
