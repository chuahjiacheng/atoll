# Island arbiter

Owner: `adaptive-island` · Version 0.1 (draft, awaiting approval) · 2026-11-16

One place decides what the island shows. Sources do not rank themselves and do
not look at each other: each says what it has to offer (a claim), the arbiter
picks. Code: `src-tauri/src/arbiter.rs` (decisions and tests),
`src/lib/island/arbiter.ts` (the page's side).

## 1. Claims

A claim is a key and an optional token. A source keeps a claim up for as long
as it has the thing to show, and drops it when it has not.

| Key | Source | Region | Offered while |
| --- | --- | --- | --- |
| `peek:permission`, `peek:question` | agent | surface | a permission request or question is waiting |
| `peek:ask` | agent | surface | Claude ended on a question |
| `panel` | user | surface | the island is open (hover, click, hotkey, drag, typing) |
| `peek:digest` | focus | surface | a break started and alerts were held |
| `peek:finished` | agent | surface | a task just finished |
| `needs-you`, `error`, `busy`, `done` | agent | pill | the agent's headline status |
| `focus` | focus | pill | a focus session or break is on |
| `event` | calendar | pill | an event starts within the set minutes |
| `music` | music | pill | a track is in the pill (lyrics are a way of showing it) |
| `idle` | island | pill | always (never posted) |

`token` says what exactly is claimed (a session id). A new token on a claim
already up is the same kind of thing arriving again: it is counted with it
(`count`) if it comes within 1500 ms, and never restarts the claim.

## 2. Rules

Pill, best first: `needs-you`, `error`, `focus`, then `busy`, `event`, `done`,
`music` (with Settings → Spotify → music first: `event`, `music`, `busy`,
`done`), then `idle`. Two exceptions, both as before the arbiter existed:

- An event does not interrupt a working agent (`busy` blocks `event`).
- During focus with Do not disturb, `error` is held behind `focus` unless
  Settings → Focus timer says to show errors.

Timing (*provisional* numbers):

| Rule | Value | Applies to |
| --- | --- | --- |
| Preempt at once | — | `needs-you`, `error` (not held), `focus` |
| Dwell | 400 ms | any other claim replacing a state still on (never protects `idle`) |
| Rate limit | agent 1000 ms, music 1000 ms, calendar 3000 ms | a source taking the pill from another source's state |
| Return | at once | the state on screen ended: the best waiting claim shows, no dwell, no limit |

One source moving on (`busy` to `done`) is not a takeover and is not limited.

Surfaces follow a fixed order (the table in §1, top to bottom) and cover the
pill. The page applies that same fixed order itself without waiting for the
answer, because surfaces are the person's own action or cannot wait.

## 3. Calling it

```ts
invoke<Decision>("arbiter_post", { input: { seq, claims, policy } })
// policy: { musicFirst, focusDnd, errorsInFocus }
// Decision: { seq, state, pill, surface, reason, order, queue, last, recheckIn }
```

- Post the whole list of claims whenever the set changes. Never on a tick.
- `order` is the pill states on offer, best first. Show the first one you still
  have: a state that ends is then replaced without waiting for an answer.
- `recheckIn` (ms) is set while a claim waits on a dwell or rate limit: post the
  same list again after that long. The arbiter has no timers of its own.
- `queue` lists every claim not showing: `{ key, source, status, reason, count, waitedMs }`,
  status `waiting`, `held`, `dwell`, `rate-limited` or `covered`.
- Event `island-state` (`{ id, previous, since, reason, queue }`) is emitted to
  all windows whenever `state` changes.

Only the island page posts today. A Rust module that wants a new claim key
asks in `docs/requests/` first: keys and their rank are part of this contract.

## Changelog

- 0.1 (2026-11-16): first version, as built in step 4.
