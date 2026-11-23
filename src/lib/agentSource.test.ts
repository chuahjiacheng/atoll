// node --experimental-strip-types --test src/lib/agentSource.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { pickAgentSource, CODE_ACTIVE_MS } from "./agentSource.ts";

const NOW = 10_000_000_000;
const chats = (plan: string, updatedAt = NOW - 60_000) => ({
  available: true,
  plan,
  chats: [{ uuid: "c", name: "Chat", updatedAt, ask: null, answer: null, answeredAt: 0, messageCount: 0 }],
});
const session = (status: string, updatedAt: number) => ({ id: "s", status, updatedAt }) as any;

test("free plan: chats only, no switch", () => {
  assert.deepEqual(pickAgentSource("auto", chats("free"), [], NOW), { show: "chat", canSwitch: false });
  assert.deepEqual(pickAgentSource("code", chats("free"), [], NOW), { show: "chat", canSwitch: false });
});

test("no chats cache: Code as before", () => {
  assert.deepEqual(pickAgentSource("chat", null, [], NOW), { show: "code", canSwitch: false });
  assert.deepEqual(pickAgentSource("auto", { available: true, plan: "pro", chats: [] }, [], NOW), { show: "code", canSwitch: false });
});

test("pro, Code not in use, chatting: chats", () => {
  assert.equal(pickAgentSource("auto", chats("pro"), [], NOW).show, "chat");
  const old = session("idle", NOW - CODE_ACTIVE_MS - 120_000);
  assert.deepEqual(pickAgentSource("auto", chats("pro"), [old], NOW), { show: "chat", canSwitch: true });
});

test("pro, both in use: Code wins on auto", () => {
  assert.equal(pickAgentSource("auto", chats("max"), [session("working", NOW - 30 * 60_000)], NOW).show, "code");
  assert.equal(pickAgentSource("auto", chats("pro"), [session("idle", NOW - 1000)], NOW).show, "code");
});

test("pro, neither in use: the most recent one", () => {
  const s = session("done", NOW - 3 * CODE_ACTIVE_MS);
  assert.equal(pickAgentSource("auto", chats("pro", NOW - 2 * CODE_ACTIVE_MS), [s], NOW).show, "chat");
  assert.equal(pickAgentSource("auto", chats("pro", NOW - 5 * CODE_ACTIVE_MS), [s], NOW).show, "code");
});

test("the user's switch beats auto", () => {
  assert.deepEqual(pickAgentSource("chat", chats("pro"), [session("working", NOW)], NOW), { show: "chat", canSwitch: true });
  assert.equal(pickAgentSource("code", chats("pro"), [], NOW).show, "code");
});

test("unknown plan with an API-key session still offers both", () => {
  assert.equal(pickAgentSource("auto", chats(""), [session("working", NOW)], NOW).canSwitch, true);
});
