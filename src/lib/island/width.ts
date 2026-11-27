// The width contract: see docs/contracts/width-contract.md.
// Every state the island shows declares how wide it wants to be; the island
// resolves one width from that and animates to it.

export type IslandStateId =
  | "idle"
  | "busy"
  | "done"
  | "error"
  | "attention"
  | "music"
  | "lyrics"
  | "focus"
  | "event"
  | "assistant"
  | "peek:permission"
  | "peek:question"
  | "peek:ask"
  | "peek:digest"
  | "peek:finished"
  | "panel";

export type TextStyle = "label" | "labelStrong" | "small" | "smallStrong" | "body" | "bodyStrong" | "tabular" | "tabularStrong";

export interface Measurer {
  /** False until the fonts have loaded; every state resolves to its min until then. */
  readonly ready: boolean;
  /** Width of a string in a type style from tokens.css, cached by string + font. */
  text(value: string, style: TextStyle): number;
  /** A size token in px, e.g. token("--sp-16"). */
  token(name: string): number;
}

export interface WidthContract {
  id: IslandStateId;
  /** Never narrower than this. */
  min: number;
  /** A fixed width, or "content" to use measure(). */
  preferred: number | "content";
  /** Never wider than this. Pill states are also capped by the Collapsed width setting. */
  max: number;
  /** Full width the content needs, padding and gaps included. Pure and synchronous. */
  measure(m: Measurer): number;
  /** What the text does when the content is wider than max. */
  overflow: "ellipsis" | "marquee" | "none";
  /** Pill height by default. Peeks use "content". */
  height?: number | "content";
}

export interface Resolved {
  /** The width the island should be (whole, even px). */
  width: number;
  /** What the content asked for before clamping. */
  wanted: number;
  /** The content is wider than the island may get: the overflow rule applies. */
  overflowing: boolean;
}

const even = (n: number) => 2 * Math.ceil(n / 2);

/** clamp(wanted, min, min(max, ceiling)), rounded up to an even number of px. */
export function resolveWidth(c: WidthContract, m: Measurer, ceiling = Infinity): Resolved {
  const top = Math.max(c.min, Math.min(c.max, ceiling));
  if (!m.ready) return { width: even(c.min), wanted: c.min, overflowing: false };
  const wanted = c.preferred === "content" ? c.measure(m) : c.preferred;
  const width = Math.min(even(Math.max(c.min, Math.min(wanted, top))), Math.floor(top / 2) * 2 || even(top));
  return { width, wanted, overflowing: wanted > top };
}

/**
 * The pill's fixed size: never narrower than `base` (Settings → Appearance →
 * Collapsed width), wider only when the state's own ceiling let it grow.
 */
export function withFixedSize(r: Resolved, base: number): Resolved {
  const floor = Math.floor(base / 2) * 2 || even(base);
  return r.width >= floor ? r : { ...r, width: floor };
}

/**
 * Small differences are ignored so the island does not twitch: within one
 * state, it only shrinks once the content is at least `deadband` narrower.
 * Growing is never held back (the content would be cut off).
 */
export function settleWidth(current: number, next: number, deadband: number): number {
  if (next >= current) return next;
  return current - next < deadband ? current : next;
}

// ---------- invalidation ----------

const listeners = new Set<(id: IslandStateId | null) => void>();

/** Call when your content changed and the width may need to change. */
export function invalidate(id: IslandStateId): void {
  listeners.forEach((cb) => cb(id));
}

/** The island listens here (owner only). */
export function onInvalidate(cb: (id: IslandStateId | null) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}
