# Island metrics

Owner: `adaptive-island` · Version 0.1 (draft, awaiting approval) · 2026-11-10

What the island exposes to the test harness and the dev overlay: its current
state, its width animation and its frame times. Read-only. Nothing here leaves
the PC, and nothing is collected unless a reader is attached.

## 1. When it is available

- Debug builds (`npm run app`): always.
- Release builds: only when the hidden setting `dev.metrics` is `true`.

When it is off, `window.__atoll` is undefined and the `island_metrics` command
returns `null`. Check for that; do not assume it exists.

## 2. Page side: `window.__atoll.metrics`

```ts
interface IslandMetricsApi {
  /** Current values. Cheap; safe to call every frame. */
  snapshot(): IslandMetrics;
  /** Called on every state change and at the end of every animation. Returns an unsubscribe function. */
  subscribe(cb: (m: IslandMetrics) => void): () => void;
  /** Start a named recording; frame stats are counted from here. */
  begin(label: string): void;
  /** End it and get the result. */
  end(label: string): FrameStats;
  /** Clear the running totals. */
  reset(): void;
}

interface IslandMetrics {
  version: 1;
  /** Shared clock, ms (performance.timeOrigin + performance.now()). */
  at: number;
  state: {
    id: string;                 // IslandStateId from width-contract.md
    previous: string | null;
    since: number;              // shared clock, ms
    reason: string;             // why the arbiter chose it, one line
    queue: { id: string; source: string; at: number }[];
  };
  width: {
    current: number;            // CSS px
    target: number;
    velocity: number;           // px per second
    min: number; max: number;   // from the state's contract
    settled: boolean;
    retargets: number;          // retargets since the last rest
  };
  height: { current: number; target: number; settled: boolean };
  radius: number;
  /** True only while a frame loop is running. Must be false when collapsed and idle. */
  loopRunning: boolean;
  frames: FrameStats;           // running totals since reset()
  lyrics: null | {
    mode: "follow" | "smooth" | "fixed";
    line: number;               // index of the line being sung, -1 before the first
    nextChangeIn: number;       // ms until the next line
    lookaheadTarget: number;    // width chosen from the look-ahead window
  };
}

interface FrameStats {
  budgetMs: number;             // 1000 / display refresh rate
  count: number;                // frames while a loop was running
  dropped: number;              // frames longer than 1.5 × budget
  p50: number; p95: number; max: number;   // ms between frames
  longestJumpPx: number;        // largest width change between two frames
  durationMs: number;
}
```

`longestJumpPx` is how a retarget "jump" is detected: during a retarget it must
stay close to `velocity × frame time`.

## 3. Native side: `island_metrics` command

```ts
invoke<NativeMetrics | null>("island_metrics")

interface NativeMetrics {
  version: 1;
  at: number;                              // shared clock, ms
  clockOffsetMs: number;                   // page clock minus native clock, as measured
  shape: { x: number; y: number; w: number; h: number; r: number };  // physical px, what the glass last drew
  hit:   { x: number; y: number; w: number; h: number };            // physical px, what hover is tested against
  glass: {
    mode: "liquid" | "frosted" | "off";
    loopRunning: boolean;
    frames: FrameStats;                    // same shape as above, from present times
    lastLagMs: number;                     // glass frame time minus the page frame it matches
  };
  arbiter: { state: string; queue: number; lastDecision: string };
}
```

## 4. Events

| Event (Tauri, to the island window) | Payload | When |
| --- | --- | --- |
| `island-state` | `IslandMetrics["state"]` | every arbiter decision |
| `island-rest` | `{ width, height, frames: FrameStats }` | each time an animation settles |

## 5. Pass marks the harness can assert

- `frames.dropped === 0` over a recording that covers a morph.
- `frames.p95 <= budgetMs × 1.2` over a full song with lyrics in the pill.
- `loopRunning === false` (page and glass) within 500 ms of settling in `idle`.
- `|glass.lastLagMs| <= budgetMs`. [Unverified] that this bound can always be
  met: the page and the glass are composed by Windows from two separate
  surfaces, so the real on-screen difference cannot be read directly.

## Changelog

- 0.1 (2026-11-10): first draft.
