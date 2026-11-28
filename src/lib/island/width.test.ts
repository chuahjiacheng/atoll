import { test } from "node:test";
import assert from "node:assert/strict";
import { withFixedSize } from "./width.ts";

test("fixed size: content narrower than the fixed size still gets the fixed size", () => {
  assert.equal(withFixedSize({ width: 212, wanted: 211, overflowing: false }, 448).width, 448);
});

test("fixed size: a width that grew past the fixed size is kept", () => {
  assert.equal(withFixedSize({ width: 520, wanted: 519, overflowing: false }, 448).width, 520);
});

test("fixed size: an odd setting rounds down to an even width, and the rest is untouched", () => {
  const r = withFixedSize({ width: 100, wanted: 600, overflowing: true }, 385);
  assert.deepEqual(r, { width: 384, wanted: 600, overflowing: true });
});

import { pillCeilingFor, resolveWidth, type Measurer } from "./width.ts";
import { statusContract } from "./contracts.ts";

// A stand-in measurer: 7px per character, every token 8px.
const fake: Measurer = { ready: true, text: (v) => v.length * 7, token: () => 8 };

test("ceiling: Claude's states and lyrics may grow to the expanded width; others stay fixed", () => {
  for (const id of ["busy", "done", "error", "attention", "lyrics"] as const) assert.equal(pillCeilingFor(id, 384, 560), 560, id);
  for (const id of ["idle", "music", "focus", "event", "assistant"] as const) assert.equal(pillCeilingFor(id, 384, 560), 384, id);
});

test("ceiling: an expanded width smaller than the fixed size never shrinks the pill", () => {
  assert.equal(pillCeilingFor("busy", 384, 300), 384);
});

test("busy: a long task-list step grows the pill past the fixed size instead of cutting it", () => {
  const step = "1/5 · Look at clipboard image reading and the snip flow";
  const c = statusContract("busy", { word: "Busy", text: step, meta: "0:51", showPip: true });
  const r = withFixedSize(resolveWidth(c, fake, pillCeilingFor("busy", 384, 560)), 384);
  assert.ok(r.width > 384, `grew to ${r.width}`);
  assert.equal(r.overflowing, false);
});

test("busy: a short step keeps the fixed size", () => {
  const c = statusContract("busy", { word: "Busy", text: "1/2 · Test", meta: "0:05", showPip: true });
  assert.equal(withFixedSize(resolveWidth(c, fake, pillCeilingFor("busy", 384, 560)), 384).width, 384);
});

test("busy: a step longer than the expanded width stops there and ends in an ellipsis", () => {
  const c = statusContract("busy", { word: "Busy", text: "x".repeat(200), meta: "0:05", showPip: true });
  const r = resolveWidth(c, fake, pillCeilingFor("busy", 384, 560));
  assert.equal(r.width, 560);
  assert.equal(r.overflowing, true);
});
