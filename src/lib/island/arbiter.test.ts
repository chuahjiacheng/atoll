// Run: node --test src/lib/island/arbiter.test.ts
// The decisions themselves are tested in src-tauri/src/arbiter.rs; this covers
// what the page does with them.
import { test, mock } from "node:test";
import assert from "node:assert/strict";

// arbiter.ts imports the Tauri bridge; outside the app it only needs to exist.
mock.module("@tauri-apps/api/core", { namedExports: { invoke: async () => { throw new Error("not in the app"); } } });
const { DEFAULT_ORDER, createArbiter, pickPill } = await import("./arbiter.ts");
type Decision = import("./arbiter.ts").Decision;
type PillKey = import("./arbiter.ts").PillKey;
type ClaimKey = import("./arbiter.ts").ClaimKey;

const set = (...keys: ClaimKey[]) => new Set<ClaimKey>(keys);
const decision = (seq: number, pill: PillKey, recheckIn: number | null = null): Decision => ({
  seq, state: pill, pill, surface: null, reason: "", order: [pill, "idle"], queue: [], last: null, recheckIn,
});

test("the page shows the first state in the order that is still on offer", () => {
  // [order from the arbiter, what the page has now, shown, why]
  const rows: [PillKey[], Set<ClaimKey>, PillKey, string][] = [
    [["busy", "music", "idle"], set("busy", "music"), "busy", "the arbiter's choice"],
    [["busy", "music", "idle"], set("music"), "music", "busy ended: next in order, without asking"],
    [["busy", "music", "idle"], set(), "idle", "everything ended"],
    [["music", "idle"], set("music", "needs-you"), "music", "a new claim waits for the arbiter's answer"],
    [["music", "busy", "idle"], set("music", "busy"), "music", "busy is rate limited: the order keeps music first"],
    [["focus", "error", "music", "idle"], set("error", "music"), "error", "focus ended: the held error comes next"],
    [DEFAULT_ORDER, set("music", "done", "panel"), "done", "before the first answer: the default order; surfaces are not pill states"],
  ];
  for (const [order, offered, want, why] of rows) assert.equal(pickPill(order, offered), want, why);
});

test("an answer overtaken by a newer one is dropped", async () => {
  const replies: ((d: Decision) => void)[] = [];
  const got: number[] = [];
  const a = createArbiter((d) => got.push(d.seq), () => new Promise((r) => replies.push(r)));
  const policy = { musicFirst: false, focusDnd: false, errorsInFocus: false };
  a.post([{ key: "music" }], policy);
  a.post([{ key: "music" }, { key: "busy" }], policy);
  replies[1](decision(2, "busy")); // the second answer arrives first
  await Promise.resolve();
  replies[0](decision(1, "music"));
  await Promise.resolve();
  assert.deepEqual(got, [2]);
  a.stop();
});

test("a waiting claim is asked about again when its clock runs out, and only then", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let sent = 0;
  const answers = [decision(1, "music", 300), decision(2, "busy")];
  const got: PillKey[] = [];
  const a = createArbiter((d) => got.push(d.pill), async () => answers[sent++]);
  a.post([{ key: "music" }, { key: "busy" }], { musicFirst: false, focusDnd: false, errorsInFocus: false });
  await new Promise((r) => setImmediate(r));
  assert.equal(sent, 1);
  t.mock.timers.tick(299);
  assert.equal(sent, 1, "not before the dwell is over");
  t.mock.timers.tick(5);
  await new Promise((r) => setImmediate(r));
  assert.equal(sent, 2);
  assert.deepEqual(got, ["music", "busy"]);
  t.mock.timers.tick(60_000);
  assert.equal(sent, 2, "nothing waiting: no timer left running");
  a.stop();
});
