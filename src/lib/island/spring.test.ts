// Run: node --test src/lib/island/spring.test.ts   (Node 22.6+ strips the types)
import { test } from "node:test";
import assert from "node:assert/strict";
import { atRest, evaluate, retarget, still, type Shape } from "./spring.ts";

const shape = (w: number, h = 40, r = 20): Shape => ({ w, h, r });
const close = (a: number, b: number, eps: number, what: string) =>
  assert.ok(Math.abs(a - b) <= eps, `${what}: ${a} vs ${b} (allowed ${eps})`);

test("starts where it was and ends on target", () => {
  const seg = retarget(still(shape(72), 0), shape(384), 0.03, 0);
  close(evaluate(seg, 0).pos.w, 72, 1e-9, "start");
  close(evaluate(seg, 0).vel.w, 0, 1e-9, "start velocity");
  close(evaluate(seg, 1000).pos.w, 384, 0.01, "end");
  assert.ok(atRest(seg, 1000));
  assert.ok(!atRest(seg, 50));
});

test("never overshoots from rest, growing or shrinking", () => {
  for (const [from, to] of [[72, 384], [384, 72], [160, 172], [720, 72]]) {
    const seg = retarget(still(shape(from), 0), shape(to), 0.028, 0);
    const [lo, hi] = [Math.min(from, to), Math.max(from, to)];
    let prev = from;
    for (let t = 0; t <= 1200; t += 1) {
      const w = evaluate(seg, t).pos.w;
      assert.ok(w >= lo - 1e-9 && w <= hi + 1e-9, `${from}->${to} left its range at ${t} ms: ${w}`);
      assert.ok(to > from ? w >= prev - 1e-9 : w <= prev + 1e-9, `${from}->${to} went backwards at ${t} ms`);
      prev = w;
    }
  }
});

// Each row: start, first target, time of the retarget (ms), second target, stiffness after.
const RETARGETS: [number, number, number, number, number][] = [
  [72, 384, 40, 160, 0.018],
  [72, 384, 90, 72, 0.034],
  [384, 72, 16, 384, 0.03],
  [160, 300, 120, 320, 0.03],
  [72, 720, 60, 440, 0.028],
  [300, 160, 5, 384, 0.03],
];

test("retargeting keeps position and velocity (no jump, no kink)", () => {
  for (const [from, first, at, second, omega] of RETARGETS) {
    const a = retarget(still(shape(from), 0), shape(first), 0.03, 0);
    const before = evaluate(a, at);
    const b = retarget(a, shape(second), omega, at);
    const after = evaluate(b, at);
    for (const k of ["w", "h", "r"] as const) {
      close(after.pos[k], before.pos[k], 1e-9, `position ${k} at retarget`);
      close(after.vel[k], before.vel[k], 1e-9, `velocity ${k} at retarget`);
    }
    // One frame later (120 Hz and 60 Hz) the step is what the velocity predicts, not a jump.
    for (const frame of [8.33, 16.67]) {
      const next = evaluate(b, at + frame).pos.w;
      const predicted = before.pos.w + before.vel.w * frame;
      // Allow for the spring's own acceleration over one frame.
      const accel = omega * omega * Math.abs(second - before.pos.w) + 2 * omega * Math.abs(before.vel.w);
      close(next, predicted, 0.5 * accel * frame * frame + 1e-6, `one ${frame} ms frame after retargeting ${from}->${first}->${second}`);
    }
    close(evaluate(b, at + 2000).pos.w, second, 0.01, "settles on the new target");
  }
});

test("the result does not depend on frame rate", () => {
  const seg = retarget(still(shape(72), 0), shape(384), 0.03, 0);
  // Evaluating at 100 ms gives the same answer however many frames came before.
  const direct = evaluate(seg, 100).pos.w;
  let stepped = seg;
  for (let t = 7; t <= 100; t += 7) stepped = retarget(stepped, shape(384), 0.03, t);
  close(evaluate(stepped, 100).pos.w, direct, 1e-6, "re-based every 7 ms");
});

test("width, height and radius move in step", () => {
  const seg = retarget(still({ w: 72, h: 40, r: 20 }, 0), { w: 720, h: 440, r: 32 }, 0.028, 0);
  for (const t of [20, 60, 120, 200]) {
    const p = evaluate(seg, t).pos;
    const fw = (p.w - 72) / (720 - 72);
    close((p.h - 40) / 400, fw, 1e-9, `height fraction at ${t}`);
    close((p.r - 20) / 12, fw, 1e-9, `radius fraction at ${t}`);
  }
});
