/**
 * Pip's rules that don't need the page: which outfit it wears, when it naps,
 * when it blinks and when it hops. Kept pure so they can be unit-tested
 * (pip.test.ts); Pip.svelte and App.svelte only call these.
 */

export type Mood = "idle" | "focused" | "curious" | "happy" | "dizzy" | "sleepy";
export type Tone = "peri" | "mint" | "peach" | "lilac";
export type Outfit = "none" | "headphones" | "beanie" | "glasses" | "bow" | "party";

export const OUTFITS: { id: Outfit; name: string }[] = [
  { id: "none", name: "None" },
  { id: "headphones", name: "Headphones" },
  { id: "beanie", name: "Beanie" },
  { id: "glasses", name: "Glasses" },
  { id: "bow", name: "Bow" },
  { id: "party", name: "Party hat" },
];

/** Settings → Mascot → Nap when quiet, in minutes (0 = never). */
export const NAP_CHOICES = [0, 5, 10, 30] as const;

/** The outfit Pip actually shows: headphones win while music plays, if that option is on. */
export function wornOutfit(chosen: Outfit, musicPlaying: boolean, headphonesWithMusic: boolean): Outfit {
  return musicPlaying && headphonesWithMusic ? "headphones" : chosen;
}

/**
 * How long until Pip naps, in ms, or null for never. `quiet` is true while
 * the island is a pill showing nothing but Pip. The island's clock doesn't
 * tick while idle, so App.svelte sets one timer for this delay and wakes Pip
 * as soon as `quiet` turns false.
 */
export function napDelay(quiet: boolean, napAfterMin: number): number | null {
  if (!quiet || !(napAfterMin > 0)) return null;
  return napAfterMin * 60_000;
}

/** Only moods with open, round eyes blink. */
export function canBlink(mood: Mood): boolean {
  return mood === "idle" || mood === "curious";
}

/** Milliseconds until the next blink: 3.5 to 7.5 s, so it never looks mechanical. */
export function blinkDelay(rand: () => number = Math.random): number {
  return 3500 + Math.max(0, Math.min(1, rand())) * 4000;
}

/** A small hop when Pip turns happy (e.g. Claude just finished), not on first show. */
export function shouldHop(prev: Mood | undefined, next: Mood, enabled: boolean): boolean {
  return enabled && prev !== undefined && prev !== "happy" && next === "happy";
}

/** Hats replace the sprout on top of Pip's head. */
export function hidesSprout(outfit: Outfit): boolean {
  return outfit === "beanie" || outfit === "party";
}
