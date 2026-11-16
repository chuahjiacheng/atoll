// How wide the pill should be while it sings along.
//
// The lyrics are synced, so the lines to come are known. That allows two
// things a pill that only reacts cannot do:
//
// - start widening a little BEFORE a longer line arrives, so the island is
//   already the right size when the line rolls in;
// - not pulse on every line: in "smooth" mode the width comes from the longest
//   line in a short window ahead, so a run of short and long lines holds one
//   width instead of breathing with each of them.
//
// Growing fast and shrinking slowly, and ignoring small differences, is the
// island's general rule for one state (App.svelte); this module only decides
// what width the lyrics ask for at a given moment, and when to ask again.
// No DOM and no timers here, so it can be unit tested (lyrics-width.test.ts).

export type LyricsMode = "follow" | "smooth" | "fixed";

export interface LyricLine {
  /** When the line starts, ms into the track. */
  t: number;
  text: string;
}

export interface LyricsPlanInput {
  mode: LyricsMode;
  lines: LyricLine[];
  /** Width each line needs in the pill (same order as `lines`), px. */
  widths: number[];
  /** Width of the "title · artist" pill, shown before the first line. */
  titleWidth: number;
  /** Playback position, ms. */
  pos: number;
  /** The width change starts this long before a line does, ms. */
  lead: number;
  /** "smooth": how far ahead to look for the longest line, ms. */
  lookahead: number;
  /** "smooth": widths are rounded up to this step, so near-equal lines share one width. */
  step: number;
}

export interface LyricsPlan {
  /** The line being sung at `pos` (-1 before the first). */
  line: number;
  /** The width to ask for now; Infinity = as wide as the pill may get. */
  width: number;
  /** ms until this plan changes (a line starts, or its width change is due); null = never. */
  wakeIn: number | null;
}

/** Index of the line being sung at `pos`, or -1 before the first line. */
export function lineAt(lines: LyricLine[], pos: number): number {
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].t <= pos) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/** ms until the next line starts or the next width change is due; null after the last line. */
export function wakeIn(lines: LyricLine[], pos: number, lead: number): number | null {
  // The first line that has not started yet.
  const next = lineAt(lines, pos) + 1;
  if (next >= lines.length) return null;
  const untilStart = lines[next].t - pos;
  // Its width change comes first, unless that moment has already passed.
  const untilLead = untilStart - lead;
  return untilLead > 0 ? untilLead : untilStart;
}

export function planLyrics(input: LyricsPlanInput): LyricsPlan {
  const { mode, lines, widths, titleWidth, pos, lead, lookahead, step } = input;
  const line = lineAt(lines, pos);
  const wake = wakeIn(lines, pos, lead);
  if (mode === "fixed") return { line, width: Infinity, wakeIn: wake };

  const widthOf = (i: number) => (i < 0 ? titleWidth : (widths[i] ?? titleWidth));
  // Sizing looks `lead` ahead of the playhead, so the island starts growing
  // before a longer line arrives. The line on screen always counts too: the
  // island never shrinks to a shorter next line until that line has started.
  const ahead = pos + lead;
  const sizing = lineAt(lines, ahead);
  let width = Math.max(widthOf(line), widthOf(sizing));
  if (mode === "follow") return { line, width, wakeIn: wake };

  // Smooth: also every line starting within the window.
  for (let i = sizing + 1; i < lines.length && lines[i].t <= ahead + lookahead; i++) width = Math.max(width, widthOf(i));
  if (step > 1) width = Math.ceil(width / step) * step;
  return { line, width, wakeIn: wake };
}
