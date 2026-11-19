// node --experimental-strip-types --test src/lib/holdoff.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHoldoff } from "./holdoff.ts";

function setup() {
  let t = 1000;
  const h = createHoldoff(6000, () => t);
  return { h, at: (n) => (t = n) };
}

test("a poll with no edit is accepted", () => {
  const { h } = setup();
  assert.equal(h.accepts(h.now()), true);
});

test("a poll that started before the edit is dropped, even after the hold-off", () => {
  const { h, at } = setup();
  const started = h.now(); // poll begins at t=1000
  at(2000);
  h.touch(); // user drags at t=2000
  at(20000);
  assert.equal(h.accepts(started), false);
});

test("a poll right after the edit is dropped (Spotify hasn't caught up)", () => {
  const { h, at } = setup();
  h.touch();
  at(1000 + 3000);
  assert.equal(h.accepts(h.now()), false);
});

test("a poll after the hold-off is accepted", () => {
  const { h, at } = setup();
  h.touch();
  at(1000 + 6000);
  assert.equal(h.accepts(h.now()), true);
});

test("every new edit restarts the hold-off", () => {
  const { h, at } = setup();
  h.touch();
  at(5000);
  h.touch();
  at(10000); // 5 s after the second edit
  assert.equal(h.accepts(h.now()), false);
  at(11000);
  assert.equal(h.accepts(h.now()), true);
});
