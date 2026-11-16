// The island's shape, shared with the native side (src-tauri/src/shape.rs).
//
// The glass under the island and the hover test live in Rust. Instead of
// telling them the island's rectangle after every layout (always a frame or
// more late), the page sends the description of each move once. Both sides
// then work out the same rectangle for any moment from the same formula.
//
// For that they need the same clock. The page measures the difference
// between its clock and the native one with a few quick round trips and
// keeps the estimate from the quickest (the least delayed) one.
import { invoke } from "@tauri-apps/api/core";
import { clock, type ShapeSegment } from "./spring";

/** Page clock minus native clock, ms. */
let offset = 0;
let syncedAt = 0;
let syncing = false;
/** Re-measure this often, and only when a move is sent (never on a timer). */
const RESYNC_MS = 60_000;
const SAMPLES = 5;

async function sync() {
  if (syncing) return;
  syncing = true;
  try {
    let best = Infinity;
    for (let i = 0; i < SAMPLES; i++) {
      const sent = clock();
      const native = await invoke<number>("island_clock");
      const back = clock();
      if (typeof native !== "number") return; // not running inside the app (the lab page)
      if (back - sent < best) {
        best = back - sent;
        offset = (sent + back) / 2 - native;
      }
    }
    syncedAt = clock();
    roundTripMs = best;
  } catch {
    // Old backend or no backend: the glass keeps following reported shapes.
  } finally {
    syncing = false;
  }
}

/** The quickest clock round trip seen (ms): the estimate is good to about half of it. */
export let roundTripMs = 0;
export const clockOffsetMs = () => offset;

/** Measure the clock difference once at start-up. */
export function startClock() {
  return sync();
}

/**
 * Tell the native side about a move (or a rest position).
 * `cx` and `top`: where the island sits in its window; it grows evenly around `cx`.
 */
export function sendMotion(segment: ShapeSegment, cx: number, top: number, hoverMin: number) {
  invoke("set_island_motion", {
    motion: { segment: { ...segment, t0: segment.t0 - offset }, cx, top, hoverMin },
  }).catch(() => {});
  if (clock() - syncedAt > RESYNC_MS) sync();
}
