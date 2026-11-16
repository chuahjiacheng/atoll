// The page's side of the arbiter (src-tauri/src/arbiter.rs).
//
// The page lists what every source has to offer (claims) and posts the list
// whenever it changes. The arbiter answers with what to show and an ORDER of
// the pill states on offer. The page shows the first state in that order it
// still has. That one rule keeps the pill free of flicker even though the
// answer takes a millisecond or two to arrive:
//
// - a state that ends is replaced at once by the next in the order;
// - a state that appears is shown when the arbiter's answer arrives.
//
// Nothing here runs on its own. One timer is set only while the arbiter says
// a claim is waiting on a clock (a dwell or a rate limit).
import { invoke } from "@tauri-apps/api/core";

export type SurfaceKey = "peek:permission" | "peek:question" | "peek:ask" | "panel" | "peek:digest" | "peek:finished";
export type PillKey = "needs-you" | "error" | "focus" | "event" | "busy" | "done" | "music" | "idle";
export type ClaimKey = SurfaceKey | PillKey;

export interface Claim {
  key: ClaimKey;
  /** What exactly is claimed (a session id). A new one on a claim already there is counted with it. */
  token?: string;
}

export interface Policy {
  musicFirst: boolean;
  focusDnd: boolean;
  errorsInFocus: boolean;
}

export interface QueueItem {
  key: ClaimKey;
  source: "agent" | "focus" | "calendar" | "music" | "user" | "island";
  status: "waiting" | "held" | "dwell" | "rate-limited" | "covered";
  reason: string;
  count: number;
  waitedMs: number;
}

export interface Decision {
  seq: number;
  state: ClaimKey;
  pill: PillKey;
  surface: SurfaceKey | null;
  reason: string;
  order: PillKey[];
  queue: QueueItem[];
  last: { from: ClaimKey; to: ClaimKey; cause: "preempted" | "replaced" | "returned" | "followed"; at: number } | null;
  recheckIn: number | null;
}

/** Used until the first answer arrives, and outside the app (a browser tab has no arbiter). */
export const DEFAULT_ORDER: PillKey[] = ["needs-you", "error", "focus", "busy", "event", "done", "music", "idle"];

/** The first state in the arbiter's order that is still on offer. */
export function pickPill(order: readonly PillKey[], offered: ReadonlySet<ClaimKey>): PillKey {
  for (const key of order) if (key === "idle" || offered.has(key)) return key;
  return "idle";
}

export interface ArbiterClient {
  post(claims: Claim[], policy: Policy): void;
  stop(): void;
}

export function createArbiter(onDecision: (d: Decision) => void, send: (input: object) => Promise<Decision> = (input) => invoke<Decision>("arbiter_post", { input })): ArbiterClient {
  let seq = 0;
  let applied = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let latest: { claims: Claim[]; policy: Policy } | null = null;
  let stopped = false;

  function post(claims: Claim[], policy: Policy) {
    latest = { claims, policy };
    clearTimeout(timer);
    const mine = ++seq;
    send({ seq: mine, claims, policy })
      .then((d) => {
        // An answer overtaken by a newer one is dropped.
        if (stopped || d.seq < applied) return;
        applied = d.seq;
        onDecision(d);
        if (d.seq === seq && d.recheckIn !== null && latest) {
          const again = latest;
          timer = setTimeout(() => post(again.claims, again.policy), Math.max(4, Math.ceil(d.recheckIn) + 1));
        }
      })
      .catch(() => {}); // not in the app: the default order stays in force
  }

  return {
    post,
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
