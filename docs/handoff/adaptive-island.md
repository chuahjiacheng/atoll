# Handoff: adaptive-island

Branch `adaptive-island`, worktree `.claude/worktrees/adaptive-island`. Written 2026-11-16.
All four steps and the fixed-size lyrics are approved (step 4 and the lyrics change on 2026-11-16, after a run in the live island). The branch was merged into main on 2026-11-16 (merge commit `baa18fc`) and the merged build was approved after a run as the live island.

## What changed

1. **Content-sized pill on a spring (step 1).** Each pill state declares a width contract (min, preferred, max, how to measure). The island resolves one width and springs to it. Text is measured after fonts load and cached. The spring is closed-form and critically damped: a new target keeps position and speed, so there is no jump and no overshoot. Reduce-motion gives a plain cross-fade and an instant width.
2. **Glass and hover follow the shape natively (step 2).** The page sends each move (start, end, stiffness, start time on a shared clock) to `shape.rs`. The glass helper window and the click-through test evaluate the same curve every frame, so they do not trail the page.
3. **Lyrics: fixed size, grows for long lines (step 3, changed after step 4).** While lyrics show, the pill sits at the Collapsed width setting. A line too long to fit makes it grow, starting 180 ms early, up to the open panel's width; it returns to the fixed size when the next line that fits starts. Other states stay content-sized.
4. **Arbiter (step 4).** `src-tauri/src/arbiter.rs` decides which state gets the pill: preemption, a 400 ms dwell, per-source rate limits, hold-back of errors during focus, counted repeats, clean return. The page posts claims and shows the first state in the arbiter's order that it still has. The dev overlay shows state, reason, queue and last transition.

## Files touched

New:
- `src/lib/island/`: `spring.ts`, `measure.ts`, `width.ts`, `contracts.ts`, `metrics.ts`, `native.ts`, `lyrics-width.ts`, `arbiter.ts`, `DevOverlay.svelte`, tests `spring.test.ts`, `lyrics-width.test.ts`, `arbiter.test.ts`, and the lab (`lab/Lab.svelte`, `lab/main.ts`, `island-lab.html` at the root).
- `src-tauri/src/shape.rs`, `src-tauri/src/arbiter.rs`.
- `docs/contracts/width-contract.md` (0.3), `metrics.md` (0.3), `arbiter.md` (0.1); `docs/requests/adaptive-island.md`.

Changed:
- `src/App.svelte` (width section, lyric timer, claims and arbiter, dev overlay), `src/lib/island/Shell.svelte`, `src/lib/island/Pill.svelte`, `src/styles/tokens.css` (new size, spring, swap and lyrics tokens).
- `src-tauri/src/backdrop.rs`, `src-tauri/src/liquid.rs` (glass follows the shape, frame pacing).
- Shared files, each one marked block: `src-tauri/src/lib.rs` (`mod arbiter; mod shape;`, four commands between `// adaptive-island: begin` and `end`, the click-through read), `src-tauri/Cargo.toml` (`Win32_System_Performance`).

## How to test

```bash
node --experimental-test-module-mocks --test src/lib/island/spring.test.ts src/lib/island/lyrics-width.test.ts src/lib/island/arbiter.test.ts
```

```bash
cd src-tauri && cargo test
```

- Expect 18 page tests and 56 Rust tests to pass (4 more are probes, marked ignored).
- Lab (page only, no app): serve the worktree with Vite and open `/island-lab.html`; `window.__runLab()` runs the checks.
- Live: run the dev build, Ctrl + Shift + click the island for the overlay. Play a song with lyrics in the pill; start a focus timer; let an agent work while music plays. The overlay's Arbiter and Queue rows show what waits and why.

## Open issues

- Live check of step 4 covered music, a busy agent, the focus timer and the panel. Needs-you, errors, events, the dwell and the rate limits were not seen live; they are covered by the Rust tests only.
- Contracts on `main` are still version 0.1; the current ones are on this branch.
- Settings needed from the settings owner: `spotify.lyricsWidth` (Follow / Smooth / Fixed; always Follow until then). See `docs/requests/adaptive-island.md` for all six requests, including the equaliser's CPU cost while music plays.
- Fixed-size lyrics, live, 75 s with a temporary 320 px fixed size: 7 grows, 5 returns, 0 dropped frames on the page (490) and the glass (795). Not tested at widths past 450 px or towards the 720 px upper limit.
- Not built: marquee for text that overflows at the maximum width (it is cut with an ellipsis), the `island-rest` event, the `dev.metrics` setting.
- Arbiter timing numbers (400 ms dwell, 1000 / 1000 / 3000 ms rate limits) are first guesses; they are constants at the top of `arbiter.rs`.
- The count of coalesced repeats is reported but no peek shows it yet.
- Known from step 2: one slow frame on the first open after start-up, and a few dropped page frames when open and close interrupt each other.
- Accessibility leftovers: the pill button is rebuilt per state (keyboard focus drops on a state change); the focus ring may be clipped [Unverified]; the idle dot sits off-centre with Pip switched off.
- Impeccable: only the detector ran for steps 2 to 4 (no findings); critique and polish ran for step 1 only.
- On-screen alignment of glass and page cannot be measured from inside the app; it was checked by frame times and by eye only.

## After the merge (2026-11-16)

The owner found the content-sized pill too compact. Every pill state except idle now has the fixed size (Collapsed width); only lyrics grow past it for a long line. Idle keeps its small size. Change: `withFixedSize` in `src/lib/island/width.ts`, used by `pillFit` in `src/App.svelte`; tests in `src/lib/island/width.test.ts`. Not measured live (frames, glass). The lab page still resolves content widths.
