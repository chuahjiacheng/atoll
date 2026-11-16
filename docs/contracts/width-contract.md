# Width contract

Owner: `adaptive-island` · Version 0.4 (draft, awaiting approval) · 2026-11-16

Every state the island can show declares how wide it wants to be. The island
resolves one width from that declaration and animates to it. No state sets or
animates the island's width itself.

Numbers marked *provisional* may change before v1.0; names and shapes will not
change without a note in this file's changelog.

## 1. Units

All sizes are CSS px **before** the island zoom (Settings → Appearance → Size).
Native code multiplies by `display scale × zoom`. Widths are whole, even numbers
so the island stays centred on the pixel grid while it grows both ways.

## 2. The interface

Lives in `src/lib/island/width.ts` (stub exports exist from the first commit of
this branch; code against them).

```ts
export type IslandStateId =
  | "idle" | "busy" | "done" | "error" | "attention"
  | "music" | "lyrics" | "focus" | "event"
  | "peek:permission" | "peek:question" | "peek:ask" | "peek:digest" | "peek:finished"
  | "panel";

export interface WidthContract {
  id: IslandStateId;
  /** Never narrower than this. */
  min: number;
  /** A fixed width, or "content" to use measure(). */
  preferred: number | "content";
  /** Never wider than this. Also capped by the user's Collapsed width setting. */
  max: number;
  /**
   * Full width the content needs, padding and gaps included.
   * Must be pure and synchronous: no DOM reads, no awaits. Text goes through
   * the Measurer; fixed parts (icons, gaps, padding) come from tokens.
   */
  measure(m: Measurer): number;
  /** What the text does when the content is wider than max. */
  overflow: "ellipsis" | "marquee" | "none";
  /** Pill height by default. Peeks use "content" (their own measured height). */
  height?: number | "content";
}

export interface Measurer {
  /** Width of a string in a type style from tokens.css, cached by string + font. */
  text(value: string, style: TextStyle): number;
  /** A size token in px, e.g. token("--sp-16"). */
  token(name: string): number;
}

export type TextStyle = "label" | "labelStrong" | "small" | "smallStrong" | "body" | "bodyStrong" | "tabular" | "tabularStrong";

/** Call when your content changed and the width may need to change. */
export function invalidate(id: IslandStateId): void;
```

## 3. How the width is resolved

```
wanted  = preferred === "content" ? measure(m) : preferred
ceiling = min(contract.max, settings.appearance.collapsedWidth)   // pill states only
width   = roundUpToEven(clamp(wanted, contract.min, ceiling))
```

- A change smaller than the deadband (*provisional* 12 px) is ignored, unless
  the content would be cut off without it.
- If `wanted > ceiling`, the island stops at the ceiling and the state's
  `overflow` rule applies.
- The island grows and shrinks evenly around its centre.

## 4. Declared states (*provisional* numbers, at pill height 40)

| State | min | preferred | max | overflow |
| --- | --- | --- | --- | --- |
| idle | 64 | content (about 72) | 96 | none |
| busy, done, error, attention | 160 | content | Collapsed width | ellipsis |
| music (title · artist) | 160 | content | Collapsed width | ellipsis |
| lyrics | 160 | planned from the lines to come | Collapsed width | ellipsis |
| focus (timer) | 120 | content | Collapsed width | ellipsis |
| event | 160 | content | Collapsed width | ellipsis |
| peek:* | 440 | 440 | 440 | n/a, height = content |
| panel | Expanded width | same | same | n/a |

Idle today is one third of Collapsed width (128 px by default); it becomes
content-sized.

## 5. Rules for anyone implementing a state

1. Lay your content out at its **final** width from the first frame. The island
   clips it while it morphs. Never let text reflow with the animated width.
2. Do not animate `width`, `height` or `border-radius` on anything the island
   sizes, and do not read the island's size to position things.
3. Measure text only through `Measurer.text`. Do not create your own canvas or
   hidden measuring elements.
4. Put `dir="auto"` on user-supplied text lines so right-to-left text aligns
   correctly.
5. Call `invalidate(id)` when your content changes; do not call it on a timer.
6. A peek's inner content (for example the permission peek, owned by
   `permission-intel`) may change height freely. Its width stays 440 unless
   that state's row above is changed through a request file.

## 6. What the measurer guarantees

- Nothing is measured before `document.fonts.ready`; until then every state
  resolves to its `min`.
- Cache key: font shorthand + string. The cache is cleared when fonts finish
  loading, when the display scale changes, when the island moves to another
  monitor, and when zoom or type settings change.
- Chinese, Japanese, Korean, Malay and right-to-left strings are measured with
  the same font fallback the page renders with. [Unverified] until the
  measurement tests compare canvas results with real layout for each script;
  if any differ by more than 1 px, that script is measured through a hidden
  element instead, behind the same `Measurer` interface.

## 7. Geometry feed (owner only, for reference)

The page and the native side animate the same rectangle from the same
description, instead of the page reporting its size after each layout:

```ts
interface ShapeSegment {
  t0: number;            // shared clock, ms
  from: { w: number; h: number; r: number };
  velocity: { w: number; h: number; r: number };   // px per ms at t0
  to: { w: number; h: number; r: number };
  omega: number;         // spring stiffness, 1/ms (critically damped)
}
```

One Tauri command, `set_island_motion(segment)`, replaces the per-frame use of
`set_hit_rect` and `set_backdrop` during animation. Other branches must not
call any of the three.

## Changelog

- 0.1 (2026-11-10): first draft.
- 0.3 (2026-11-16): after building step 3 (lyrics). The lyrics row is now: min 160, preferred = planned from the lines to come (`src/lib/island/lyrics-width.ts`), max Collapsed width, overflow ellipsis (not marquee). Smooth takes the longest line starting within 8 s, rounded up to 32 px, starts growing 180 ms before a longer line, and shrinks only when at least 48 px narrower for 2.5 s. Follow takes each line (grows 180 ms early, shrinks when the shorter line starts). Fixed stays at Collapsed width.
- 0.2 (2026-11-16): after building step 1. Added the `tabularStrong` text style and `Measurer.ready`. Pill content is anchored to the island's left edge (peeks and the panel stay centred). Within one state the pill grows at once but shrinks only after its content has stayed at least 12 px narrower for 2.5 s. The hover area is never narrower than 128 px. Text that is not plain Latin, and tabular numbers, are measured through a hidden element (canvas was 2.2 px off for Japanese and 1.85 px off for tabular digits).
- 0.4 (2026-11-16): lyrics changed at the owner's request. While lyrics show, the pill has one fixed size (Settings → Appearance → Collapsed width). It grows only for a line too long to fit, starting 180 ms early, up to the open panel's width, and returns to the fixed size when the next line that fits starts. This is now the default mode (`follow`). `smooth` (hold the longest line of the next 8 s) and `fixed` (never grow) remain and are also never narrower than the fixed size. Every other state is content-sized as before; for them Collapsed width is still the maximum. Where earlier sections of this file describe the lyrics width differently, this entry wins.

- 0.5 (2026-11-16): fixed size everywhere, at the owner's request after trying the merged build. Every pill state except idle is now exactly Collapsed width wide, whatever its content (`withFixedSize` in `src/lib/island/width.ts`); content wider than that ends in an ellipsis. Only lyrics grow past it (entry 0.4). Idle keeps its small resting size. The contracts still measure their content, but for these states the result no longer sets the width. Where earlier sections describe pill states as content-sized, this entry wins.
