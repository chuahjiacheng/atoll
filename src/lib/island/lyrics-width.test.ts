// Run: node --test src/lib/island/lyrics-width.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { lineAt, planLyrics, wakeIn, type LyricLine, type LyricsMode } from "./lyrics-width.ts";

// A verse of mixed lengths, an instrumental gap, then a chorus.
const LINES: LyricLine[] = [
  { t: 10_000, text: "short" },           // 0  width 200
  { t: 13_000, text: "a much longer line" }, // 1  width 330
  { t: 16_000, text: "short again" },     // 2  width 220
  { t: 19_000, text: "medium line here" },// 3  width 290
  { t: 22_000, text: "" },                // 4  width 110  (instrumental marker, "♪")
  { t: 52_000, text: "chorus, the widest line of all" }, // 5  width 400
  { t: 55_000, text: "tiny" },            // 6  width 180
];
const WIDTHS = [200, 330, 220, 290, 110, 400, 180];
const BASE = { lines: LINES, widths: WIDTHS, titleWidth: 250, lead: 180, lookahead: 8000, step: 16 };
const plan = (mode: LyricsMode, pos: number) => planLyrics({ ...BASE, mode, pos });
const up16 = (n: number) => Math.ceil(n / 16) * 16;

test("lineAt finds the line being sung", () => {
  const rows: [number, number][] = [[0, -1], [9_999, -1], [10_000, 0], [12_999, 0], [13_000, 1], [54_999, 5], [55_000, 6], [999_000, 6]];
  for (const [pos, want] of rows) assert.equal(lineAt(LINES, pos), want, `at ${pos}`);
  assert.equal(lineAt([], 5000), -1);
});

test("the next wake-up is the width change first, then the line itself", () => {
  // [position, ms until the plan changes]
  const rows: [number, number | null][] = [
    [0, 9_820],        // first line at 10 000: its width change is due 180 ms earlier
    [9_820, 180],      // width change done: next is the line starting
    [9_900, 100],
    [10_000, 2_820],   // line 0 started: next is line 1's width change
    [22_000, 29_820],  // across the instrumental gap
    [55_000, null],    // last line: nothing more to wait for
  ];
  for (const [pos, want] of rows) assert.equal(wakeIn(LINES, pos, 180), want, `at ${pos}`);
});

test("fixed: always as wide as the pill may get", () => {
  for (const pos of [0, 10_500, 30_000, 60_000]) assert.equal(plan("fixed", pos).width, Infinity);
});

test("follow: each line's own width; growing starts early, shrinking waits for the line", () => {
  // [position, width, why]
  const rows: [number, number, string][] = [
    [0, 250, "before the first line: the title"],
    [9_700, 250, "not yet within the lead"],
    [9_820, 250, "line 0 is shorter than the title on screen: no early shrink"],
    [10_000, 200, "line 0 has started: now its width"],
    [15_900, 330, "100 ms before the shorter line 2: still line 1's width"],
    [12_000, 200, "line 0"],
    [12_900, 330, "100 ms before line 1: already widening"],
    [16_500, 220, "line 2"],
    [30_000, 110, "instrumental marker"],
    [51_900, 400, "100 ms before the chorus"],
    [56_000, 180, "last line"],
  ];
  for (const [pos, want, why] of rows) assert.equal(plan("follow", pos).width, want, why);
});

test("smooth: the longest line in the window ahead, so the pill does not pulse", () => {
  // [position, width, why]
  const rows: [number, number, string][] = [
    [0, 250 + 6, "title only: no line within 8 s (rounded up to a 16 px step)"],
    [5_000, up16(330), "lines 0 and 1 start within the window: already at the longer one"],
    [10_500, up16(330), "singing the short line 0, holding line 1's width"],
    [13_500, up16(330), "line 1"],
    [16_500, up16(290), "line 1 has passed; longest ahead is line 3"],
    [19_500, up16(290), "line 3"],
    [22_500, up16(110), "instrumental gap, chorus still 29 s away: small"],
    [43_000, up16(110), "9 s before the chorus: not yet in the window"],
    [44_000, up16(400), "chorus within 8 s (plus the lead): widen ahead of it"],
    [52_500, up16(400), "chorus"],
    [55_500, up16(180), "last line, nothing longer ahead"],
  ];
  for (const [pos, want, why] of rows) assert.equal(plan("smooth", pos).width, want, why);
});

test("smooth changes width far less often than follow over the whole song", () => {
  const changes = (mode: LyricsMode) => {
    let n = 0;
    let last = plan(mode, 0).width;
    for (let pos = 0; pos <= 60_000; pos += 50) {
      const w = plan(mode, pos).width;
      if (w !== last) n++;
      last = w;
    }
    return n;
  };
  assert.equal(changes("follow"), 7);  // once per line
  assert.equal(changes("smooth"), 5);
  assert.ok(changes("smooth") < changes("follow"));
});

test("seeking: the plan depends only on the position, so a jump lands on the right width at once", () => {
  assert.equal(plan("smooth", 52_500).width, up16(400));
  assert.equal(plan("smooth", 22_500).width, up16(110)); // seek back into the gap
  assert.equal(plan("smooth", 13_500).line, 1);
});

test("no synced lines: the title width, and nothing to wait for", () => {
  const p = planLyrics({ ...BASE, lines: [], widths: [], mode: "smooth", pos: 30_000 });
  assert.deepEqual(p, { line: -1, width: up16(250), wakeIn: null });
});
