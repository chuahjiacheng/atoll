// node --experimental-strip-types --test src/lib/pip.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { blinkDelay, canBlink, hidesSprout, napDelay, shouldHop, wornOutfit } from "./pip.ts";

test("headphones replace the outfit only while music plays and the option is on", () => {
  assert.equal(wornOutfit("beanie", false, true), "beanie");
  assert.equal(wornOutfit("beanie", true, true), "headphones");
  assert.equal(wornOutfit("beanie", true, false), "beanie");
  assert.equal(wornOutfit("none", true, true), "headphones");
});

test("naps after the chosen minutes of quiet, never when off or busy", () => {
  assert.equal(napDelay(true, 10), 600_000);
  assert.equal(napDelay(true, 5), 300_000);
  assert.equal(napDelay(true, 0), null, "0 = never");
  assert.equal(napDelay(false, 10), null, "something is showing");
  assert.equal(napDelay(true, NaN), null);
  assert.equal(napDelay(true, -5), null);
});

test("nap timer in use: quiet, busy, quiet again starts the wait over", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let napping = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  // What App.svelte's effect does each time `quiet` or the setting changes.
  const update = (quiet: boolean, min: number) => {
    clearTimeout(timer);
    napping = false;
    const d = napDelay(quiet, min);
    if (d !== null) timer = setTimeout(() => (napping = true), d);
  };
  const min = 60_000;
  update(true, 5);
  t.mock.timers.tick(5 * min - 1);
  assert.equal(napping, false);
  t.mock.timers.tick(1);
  assert.equal(napping, true);
  update(false, 5); // Claude starts working: Pip wakes
  assert.equal(napping, false);
  t.mock.timers.tick(60 * min);
  assert.equal(napping, false);
  update(true, 5); // done and dismissed: the wait starts over
  t.mock.timers.tick(4 * min);
  assert.equal(napping, false);
  t.mock.timers.tick(1 * min);
  assert.equal(napping, true);
});

test("only open-eyed moods blink", () => {
  assert.equal(canBlink("idle"), true);
  assert.equal(canBlink("curious"), true);
  for (const m of ["focused", "happy", "dizzy", "sleepy"] as const) assert.equal(canBlink(m), false, m);
});

test("blink delay stays between 3.5 and 7.5 s, even with odd random values", () => {
  assert.equal(blinkDelay(() => 0), 3500);
  assert.equal(blinkDelay(() => 1), 7500);
  assert.equal(blinkDelay(() => 0.5), 5500);
  assert.equal(blinkDelay(() => -3), 3500);
  assert.equal(blinkDelay(() => 9), 7500);
  for (let i = 0; i < 200; i++) {
    const d = blinkDelay();
    assert.ok(d >= 3500 && d <= 7500, String(d));
  }
});

test("hops when turning happy, not on first show or while staying happy", () => {
  assert.equal(shouldHop("focused", "happy", true), true);
  assert.equal(shouldHop(undefined, "happy", true), false);
  assert.equal(shouldHop("happy", "happy", true), false);
  assert.equal(shouldHop("focused", "happy", false), false);
  assert.equal(shouldHop("happy", "idle", true), false);
});

test("hats hide the sprout, other outfits keep it", () => {
  assert.equal(hidesSprout("beanie"), true);
  assert.equal(hidesSprout("party"), true);
  for (const o of ["none", "headphones", "glasses", "bow"] as const) assert.equal(hidesSprout(o), false, o);
});
