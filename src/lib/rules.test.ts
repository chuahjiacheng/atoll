// node --test src/lib/rules.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { blankRule, fingerprint, toolLabel } from "./rules.ts";

test("the page's fingerprint matches the Rust side", () => {
  // Same rule as rules::logic::parity in Rust: order, case, spaces and repeats don't matter.
  const rule = { ...blankRule(), tool: "Bash", project: "c:/users/me/proj", pattern: ["npm test", " Tail ", "npm test"] };
  assert.equal(fingerprint(rule), "c264f5babbc7a4a1");
  assert.equal(fingerprint({ ...rule, pattern: ["tail", "npm test"] }), "c264f5babbc7a4a1");
  assert.notEqual(fingerprint({ ...rule, pattern: ["npm test"] }), "c264f5babbc7a4a1");
  assert.notEqual(fingerprint({ ...rule, project: null }), "c264f5babbc7a4a1");
});

test("tool labels", () => {
  assert.equal(toolLabel("Bash"), "Commands");
  assert.equal(toolLabel("files"), "File edits");
  assert.equal(toolLabel("mcp__github__create_issue"), "create issue (github connector)");
});
