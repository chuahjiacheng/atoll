<!--
  Pip, Atoll's mascot (shapes from design/…/Mascot.dc.html).
  Built from body, sprout, shine, cheeks, eyes and mouth so each part moves
  on its own. GSAP animates mood changes (face parts swap with a quick
  squash), a gentle bob while music plays, and a wobble when poked.
  Nothing loops while Pip is idle.
-->
<script lang="ts" module>
  export type Mood = "idle" | "focused" | "curious" | "happy" | "dizzy";
  export type Tone = "peri" | "mint" | "peach" | "lilac";

  const eye = (cx: number, cy: number, rx: number, ry: number) =>
    `M${cx - rx} ${cy}a${rx} ${ry} 0 1 0 ${2 * rx} 0a${rx} ${ry} 0 1 0 ${-2 * rx} 0`;

  const FACES: Record<Mood, { eyesFill: string; eyesStroke: string; mouth: string; cheeks: string; extra: string; tilt: number }> = {
    idle: { eyesFill: eye(24, 34, 3.4, 4.6) + eye(40, 34, 3.4, 4.6), eyesStroke: "", mouth: "M29 43.5q3 2.5 6 0", cheeks: "", extra: "", tilt: 0 },
    focused: { eyesFill: "", eyesStroke: "M20 35h8M36 35h8M20 29.5l7 2M44 29.5l-7 2", mouth: "M29.5 44h5", cheeks: "", extra: "", tilt: 0 },
    curious: {
      eyesFill: eye(24, 33, 4.6, 5.6) + eye(40, 34, 3, 3.8),
      eyesStroke: "",
      mouth: "M30 44.5a2 2 0 1 0 4 0a2 2 0 1 0-4 0",
      cheeks: "",
      extra: "M51 8.5a4 4 0 1 1 5 3.9c-1 .3-1.2 1-1.2 2.3M54.8 19v.3",
      tilt: -8,
    },
    happy: {
      eyesFill: "",
      eyesStroke: "M20 36q4-5.5 8 0M36 36q4-5.5 8 0",
      mouth: "M26 41.5q6 6.5 12 0",
      cheeks: eye(18, 41.5, 3, 2.2) + eye(46, 41.5, 3, 2.2),
      extra: "",
      tilt: 0,
    },
    dizzy: {
      eyesFill: "",
      eyesStroke:
        "M24 34a1.5 1.5 0 1 1 1.5 1.5a3 3 0 1 1-3-3a4.4 4.4 0 1 1 4.4 4.4M40 34a1.5 1.5 0 1 1 1.5 1.5a3 3 0 1 1-3-3a4.4 4.4 0 1 1 4.4 4.4",
      mouth: "M26 46q2-2.5 4 0t4 0t4 0",
      cheeks: "",
      extra: "M11 10l1.4 2.8 2.8 1.4-2.8 1.4L11 18.4l-1.4-2.8-2.8-1.4 2.8-1.4zM54 6l1 2 2 1-2 1-1 2-1-2-2-1 2-1z",
      tilt: 10,
    },
  };

  const LABELS: Record<Mood, string> = {
    idle: "Pip, idle",
    focused: "Pip, focused",
    curious: "Pip, curious",
    happy: "Pip, happy",
    dizzy: "Pip, dizzy",
  };
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { dur, ease, gsap, reduced, seconds } from "../motion";
  import { px } from "../theme";

  let {
    mood = "idle",
    size = 24,
    tone = "peri",
    bob = false,
    pokeable = false,
  }: {
    mood?: Mood;
    size?: number;
    tone?: Tone;
    /** Gentle bounce in time with music (only while music plays). */
    bob?: boolean;
    /** Clicking Pip a few times makes it dizzy. */
    pokeable?: boolean;
  } = $props();

  let svg = $state<SVGSVGElement>();
  let face = $state<SVGGElement>();
  let body = $state<SVGGElement>();
  let pokes: number[] = [];
  let poked = $state(false);
  const shown = $derived<Mood>(poked ? "dizzy" : mood);
  const f = $derived(FACES[shown] ?? FACES.idle);

  // Mood change: tilt to the new angle and give the face a quick squash.
  let first = true;
  $effect(() => {
    const tilt = f.tilt;
    if (!body || !face) return;
    if (first || reduced()) {
      gsap.set(body, { rotation: tilt, svgOrigin: "32 36" });
      first = false;
      return;
    }
    gsap.to(body, { rotation: tilt, svgOrigin: "32 36", duration: dur("slow"), ease: ease(), overwrite: "auto" });
    gsap.fromTo(face, { scaleY: 0.6, svgOrigin: "32 38" }, { scaleY: 1, svgOrigin: "32 38", duration: dur("fast"), ease: ease() });
  });

  // Music bob: a slow, small rise and fall. Stops (and resets) when music stops.
  let bobTween: gsap.core.Tween | null = null;
  $effect(() => {
    const on = bob && !reduced();
    if (!svg) return;
    bobTween?.kill();
    bobTween = null;
    if (on) {
      bobTween = gsap.to(svg, { y: -px("--bob-height"), duration: seconds("--dur-bob"), ease: "steps(3)", yoyo: true, repeat: -1 });
    } else {
      gsap.to(svg, { y: 0, duration: dur("fast"), ease: ease() });
    }
  });

  function poke(e: MouseEvent) {
    if (!pokeable) return;
    e.stopPropagation();
    const now = Date.now();
    pokes = [...pokes.filter((t) => now - t < 1500), now];
    if (!reduced() && body) gsap.fromTo(body, { rotation: f.tilt - 6 }, { rotation: f.tilt, svgOrigin: "32 36", duration: dur("slow"), ease: ease() });
    if (pokes.length >= 4) {
      pokes = [];
      poked = true;
      setTimeout(() => (poked = false), seconds("--dur-dizzy") * 1000);
    }
  }

  onMount(() => () => {
    bobTween?.kill();
    if (svg) gsap.killTweensOf([svg, body, face]);
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<svg
  bind:this={svg}
  width={size}
  height={size}
  viewBox="0 0 64 64"
  role="img"
  aria-label={LABELS[shown]}
  class="pip tone-{tone}"
  onclick={poke}
>
  <g bind:this={body}>
    <path class="sprout" d="M32 13c-2-5 1-9 7-10 0 5-3 9-7 10z" />
    <path class="body" d="M32 12c14 0 26 9 26 25 0 12-11 17-26 17S6 49 6 37C6 21 18 12 32 12z" stroke-width="1.5" />
    <path class="shine" d="M18 24c3-4 7-6 11-6.5" fill="none" stroke-width="3" stroke-linecap="round" />
    <g bind:this={face}>
      {#if f.cheeks}<path class="cheeks" d={f.cheeks} />{/if}
      {#if f.eyesFill}<path class="eyes" d={f.eyesFill} />{/if}
      {#if f.eyesStroke}<path class="lines" d={f.eyesStroke} stroke-width="3.2" />{/if}
      <path class="lines" d={f.mouth} stroke-width="2.4" />
    </g>
  </g>
  {#if f.extra}<path class="extra" d={f.extra} stroke-width="2.6" />{/if}
</svg>

<style>
  .pip { display: block; flex: none; overflow: visible; }
  .tone-peri { --pip-body: var(--pip-peri); --pip-edge: var(--pip-peri-edge); }
  .tone-mint { --pip-body: var(--pip-mint); --pip-edge: var(--pip-mint-edge); }
  .tone-peach { --pip-body: var(--pip-peach); --pip-edge: var(--pip-peach-edge); }
  .tone-lilac { --pip-body: var(--pip-lilac); --pip-edge: var(--pip-lilac-edge); }
  .sprout { fill: var(--pip-edge); }
  .body { fill: var(--pip-body); stroke: var(--pip-edge); }
  .shine { stroke: var(--pip-shine); }
  .cheeks { fill: var(--pip-blush); fill-opacity: 0.75; }
  .eyes { fill: var(--pip-face); }
  .lines { fill: none; stroke: var(--pip-face); stroke-linecap: round; stroke-linejoin: round; }
  .extra { fill: none; stroke: var(--pip-extra); stroke-linecap: round; stroke-linejoin: round; }
</style>
