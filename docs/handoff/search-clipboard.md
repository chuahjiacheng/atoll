# Handoff: `search-clipboard`

## What changed

### Part 1: clipboard intelligence (Milestone A, approved 2026-11-11)
- Text copies are typed as link, colour, JSON or code (`clipintel.rs`). The
  clipboard tab shows an icon or colour swatch per type and filter chips.
- Secrets (API keys and tokens with known prefixes, JWTs, PEM private keys,
  Luhn-valid card numbers, passwords in `name=value` or in URLs, single long
  random-looking copies) are masked before storing. The real value is kept
  DPAPI-encrypted and is what Paste gives. Pasting a secret asks Windows'
  clipboard history and cloud clipboard not to keep it. Secrets never go to
  the whiteboard (automatically or from the menu) and are never indexed.
- Secret items expire (default 5 min; 1/5/15/60/never). On expiry they also
  leave the Windows clipboard if still there. Pinning keeps one.
- Password-manager copies are still never read (unchanged).
- Copied images are read with Windows OCR (`ocr.rs`) on a short-lived thread;
  the text is searchable and copyable ("Copy text from image").
- Transforms: formatted JSON, one-line JSON, link without tracking.
- Settings → Clipboard: Hide secrets, Forget secrets after, Find text in images.

### Part 2: universal search and command palette (Milestone B)
- `src-tauri/src/search/`: in-memory index (`index.rs`), module sources
  (`sources.rs`), event-store stub (`agent.rs`), palette window (`palette.rs`).
- Sources: clipboard, whiteboard cards, quick note (one result per line),
  file tray, calendar, agent history (stub until `recorder-forecast` merges).
  Switched-off modules are left out.
- Ranking: every query word must start a word in the item; then whole-word
  matches, words in the title, recency (half weight after a day; calendar by
  closeness to now) and pins.
- Indexing is incremental and event-driven: the worker sleeps on a channel
  until a module emits its change event, waits 250 ms, re-reads that source
  (unchanged items are skipped). Agent history is read only past the newest
  event already indexed.
- The index is never written to disk: it is rebuilt from the modules' own
  stores at start-up, so no second plain copy of clipboard text or notes exists.
- Palette: Ctrl+Alt+K (Settings → Hotkeys → "Search everything"). Its own
  window, drawn as a peek under the island; it takes focus while open and
  hides on Esc or when it loses focus. Enter = first action, Ctrl+Enter copy,
  Alt+Enter open / show in folder, Tab / Shift+Tab change source.

## Files touched

Owned (new): `src-tauri/src/clipintel.rs`, `src-tauri/src/ocr.rs`,
`src-tauri/src/search/*`, `palette.html`, `src/palette/main.ts`,
`src/lib/palette/*`, `docs/requests/search-clipboard.md`, this file.

Owned (changed): `src-tauri/src/clipboard.rs`, `src-tauri/src/winclip.rs`,
`src/lib/ClipboardView.svelte`, `src/dashboard/pages/ClipboardPage.svelte`,
clipboard types in `src/lib/tools.svelte.ts`.

Shared, one marked `search-clipboard` block or line each:
`src-tauri/Cargo.toml` (windows features Media_Ocr, Graphics_Imaging,
Globalization), `src-tauri/src/lib.rs` (mods, commands, state, start-up,
settings hook), `src-tauri/src/settings.rs` (clipboard fields,
`SearchSettings`, seeding call), `src-tauri/src/shortcuts.rs` (palette
hotkey), `src-tauri/capabilities/default.json` (palette window),
`vite.config.js` (palette page), `src/lib/settings.ts`,
`src/dashboard/pages/HotkeysPage.svelte`.

No settings version bump: the palette hotkey is added once through
`search.seeded`, so other branches' migrations don't collide.

## How to test

Run from this worktree (stop the main checkout's `npm run app` first, the
app is single-instance): `npm ci` once, then `npm run app`.

1. Copy `ghp_abcdefghijklmnopqrstuvwxyz0123456789`: shown masked with a lock
   and "forgotten in 5 min"; clicking it pastes the real token.
2. Copy `https://example.com/p?id=7&utm_source=x&fbclid=y`, right-click →
   Copy without tracking → paste gives `https://example.com/p?id=7`.
3. Copy `{"a":1}` → right-click → Copy as formatted JSON.
4. Screenshot some text (Win+Shift+S): the row says "Image with text"; its
   words are searchable.
5. Press Ctrl+Alt+K anywhere: the palette opens under the island with recent
   items. Type part of a note line, a tray file name or an event title; Enter
   runs the action shown; Esc closes; clicking elsewhere closes.
6. Tab cycles All → Clipboard → Whiteboard → Note → Files → Calendar → Agent.

Automated: `cargo test --lib` (54 tests, plus 6 ignored probes). Benchmarks (release, ignored):
`search::index::tests::bench_50k`, `search::index::tests::idle_cpu`,
`ocr::tests::probe_ocr_reads_rendered_text`.

## Measured (2026-11-16, this PC, release build)

| | Result |
| --- | --- |
| Search latency, 50,000 items, 1-char query | p50 3.4 ms, p95 4.2 ms |
| 2-char query | p50 3.5 ms, p95 4.1 ms |
| 3-char query | p50 0.9 ms, p95 1.4 ms |
| Two words | p50 3.2 ms, p95 4.7 ms |
| Empty query (recent list) | 0.5 ms |
| Incremental update | 0.003 ms per item |
| Build 50,000 items | 1.3 s (background thread) |
| Index size in memory | 28.6 MB computed; process private memory grew 43.3 MB while building (allocator keeps freed build buffers) |
| Index size on disk | 0 (not persisted, by design) |
| Idle CPU, index loaded, nothing changing | 0.000 % (0 ms CPU in 30 s) |
| Re-indexing 200 items every second | 0.016 % of 16 cores (78 ms CPU in 30 s) |
| OCR of a 640 × 120 text image | 44 ms |

The synthetic items are heavier than real ones (about 47 distinct words each);
agent events have one ~200-character summary, so real memory will be lower.
[Inference]

## Open issues

- Agent history is a stub (returns nothing) until `recorder-forecast` merges.
- Palette width (560) awaits `adaptive-island`'s answer; see the request file.
- Not tried in the live app: idle CPU of the whole app and the palette window
  (focus, paste back, placement on a scaled monitor) need a run from this
  worktree. [Unverified]
- Ctrl+Alt+K takes AutoFormat away from Word (Word's own Ctrl+Alt+K).
- The palette can't be opened by mouse yet (hotkey only).
