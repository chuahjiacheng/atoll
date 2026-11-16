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

test("retargeting never moves the shape: position carries over exactly", () => {
  for (const [from, first, at, second, omega] of RETARGETS) {
    const a = retarget(still(shape(from), 0), shape(first), 0.03, 0);
    const before = evaluate(a, at);
    const b = retarget(a, shape(second), omega, at);
    const after = evaluate(b, at);
    for (const k of ["w", "h", "r"] as const) close(after.pos[k], before.pos[k], 1e-9, `position ${k} at retarget`);
    // Velocity carries over unless it would carry the shape past the new target;
    // then it is only ever reduced, never reversed or increased.
    const gap = second - before.pos.w;
    const cap = omega * Math.abs(gap);
    const towards = Math.sign(gap) * before.vel.w;
    if (towards <= cap) close(after.vel.w, before.vel.w, 1e-9, "velocity kept");
    else close(Math.sign(gap) * after.vel.w, cap, 1e-9, "velocity capped at what the spring can absorb");
    // One frame later (120 Hz and 60 Hz) the step is no more than the old speed allows: no jump.
    for (const frame of [8.33, 16.67]) {
      const step = Math.abs(evaluate(b, at + frame).pos.w - before.pos.w);
      const accel = omega * omega * Math.abs(gap) + 2 * omega * Math.abs(before.vel.w);
      assert.ok(step <= Math.abs(before.vel.w) * frame + 0.5 * accel * frame * frame + 1e-6, `jump one ${frame} ms frame after ${from}->${first}->${second}: ${step}`);
    }
    close(evaluate(b, at + 2000).pos.w, second, 0.01, "settles on the new target");
  }
});

test("a retarget never passes its new target, however fast the shape was moving", () => {
  // Each row: start, first target, retarget time, second target.
  const rows: [number, number, number, number][] = [
    [72, 384, 30, 170],   // racing towards 384, new target just ahead
    [72, 384, 45, 300],
    [384, 72, 30, 290],   // shrinking fast, new target just below
    [72, 720, 50, 440],
    [160, 384, 20, 200],
  ];
  for (const [from, first, at, second] of rows) {
    for (const omega of [0.018, 0.03, 0.034]) {
      const a = retarget(still(shape(from), 0), shape(first), 0.03, 0);
      const start = evaluate(a, at).pos.w;
      const b = retarget(a, shape(second), omega, at);
      const [lo, hi] = [Math.min(start, second), Math.max(start, second)];
      for (let t = at; t <= at + 1500; t += 1) {
        const w = evaluate(b, t).pos.w;
        // Moving away first is allowed (it turns around); passing the target is not.
        if (Math.sign(second - start) === Math.sign(evaluate(a, at).vel.w)) {
          assert.ok(w >= lo - 1e-6 && w <= hi + 1e-6, `${from}->${first}->${second} passed its target at ${t} ms: ${w}`);
        }
      }
    }
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
