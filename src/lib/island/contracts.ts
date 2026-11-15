// Width contracts for the collapsed pill's states. Each `measure` adds up the
// same parts Pill.svelte lays out (padding, gaps, icon sizes from the tokens,
// text through the Measurer), so the island is exactly as wide as its content.
// If Pill.svelte's layout changes, change the matching sum here; the lab page
// (island-lab.html → Run checks) fails when the two disagree by more than 1 px.
import type { IslandStateId, Measurer, WidthContract } from "./width";

const sum = (...parts: number[]) => Math.ceil(parts.reduce((a, b) => a + b, 0)) + 1; // +1: never ellipsis on a rounding edge

/** Pip on the left, or nothing when Settings → Mascot → Show Pip is off. */
const pip = (m: Measurer, shown: boolean) => (shown ? m.token("--size-pip") + m.token("--sp-8") : 0);

export function idleContract(showPip = true): WidthContract {
  return {
    id: "idle",
    min: 0, // set from tokens below
    preferred: "content",
    max: 0,
    overflow: "none",
    measure: (m) =>
      // padding · Pip · gap · (flexible space) · gap · dot · padding
      Math.ceil(m.token("--sp-8") + pip(m, showPip) + m.token("--sp-8") + m.token("--sp-8") + m.token("--sp-16")),
  };
}

export interface StatusContent {
  word: string;
  text: string;
  meta?: string;
  showPip?: boolean;
}

/** Busy, done, error, needs you, and an upcoming event: Pip · icon + word · one line · time. */
export function statusContract(id: IslandStateId, c: StatusContent): WidthContract {
  return {
    id,
    min: 0,
    preferred: "content",
    max: Infinity,
    overflow: "ellipsis",
    measure: (m) =>
      sum(
        m.token("--sp-8"),
        pip(m, c.showPip ?? true),
        m.token("--size-icon-sm") + m.token("--sp-4") + m.text(c.word, "labelStrong"),
        c.text ? m.token("--sp-8") + m.text(c.text, "small") : 0,
        c.meta ? m.token("--sp-8") + m.text(c.meta, "tabular") : 0,
        m.token("--sp-16"),
      ),
  };
}

export interface MusicContent {
  title: string;
  artist: string;
}

/** Album art · title · artist · equaliser. */
export function musicContract(c: MusicContent): WidthContract {
  return {
    id: "music",
    min: 0,
    preferred: "content",
    max: Infinity,
    overflow: "ellipsis",
    measure: (m) =>
      sum(
        m.token("--sp-12"),
        m.token("--size-pip"),
        m.token("--sp-8") + m.text(c.title, "smallStrong") + (c.artist ? m.text(` · ${c.artist}`, "small") : 0),
        m.token("--sp-8") + m.token("--size-icon-sm"),
        m.token("--sp-16"),
      ),
  };
}

/**
 * Lyrics in the pill. Until the look-ahead width (step 3) lands, the pill keeps
 * today's behaviour and stays at the Collapsed width while it sings along.
 */
export function lyricsContract(): WidthContract {
  return { id: "lyrics", min: 0, preferred: Infinity, max: Infinity, overflow: "marquee", measure: () => Infinity };
}

export interface FocusContent {
  time: string;
  word: string;
  /** The word is drawn strong (a nudge). */
  strong?: boolean;
  note?: string;
}

/** Small ring · time · Focus / Break / Paused · your intention or a nudge. */
export function focusContract(c: FocusContent): WidthContract {
  return {
    id: "focus",
    min: 0,
    preferred: "content",
    max: Infinity,
    overflow: "ellipsis",
    measure: (m) =>
      sum(
        m.token("--sp-8"),
        m.token("--size-ring-pill"),
        m.token("--sp-8") + m.text(c.time, "tabularStrong"),
        m.token("--sp-8") + m.text(c.word, c.strong ? "labelStrong" : "label"),
        c.note ? m.token("--sp-8") + m.text(c.note, "small") : 0,
        m.token("--sp-16"),
      ),
  };
}

/** Fill in the min / max every contract of a kind shares, from tokens.css. */
export function withLimits(c: WidthContract, m: Measurer): WidthContract {
  if (c.id === "idle") return { ...c, min: m.token("--size-idle-min"), max: m.token("--size-idle-max") };
  if (c.id === "focus") return { ...c, min: m.token("--size-timer-min") };
  return { ...c, min: m.token("--size-pill-min") };
}
