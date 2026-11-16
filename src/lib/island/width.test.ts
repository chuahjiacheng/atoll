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
