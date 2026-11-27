<!--
  Pip, Atoll's mascot (shapes from design/…/Mascot.dc.html).
  Built from body, sprout, shine, cheeks, eyes and mouth so each part moves
  on its own. GSAP animates mood changes (face parts swap with a quick
  squash), a gentle bob while music plays, a wobble when poked, a small hop
  when Pip turns happy and a blink every few seconds (open-eyed moods only).
  Outfits are drawn on top of the body and tilt with it. The rules for which
  outfit shows, napping, blinking and hopping live in lib/pip.ts.
-->
<script lang="ts" module>
  import type { Mood, Outfit, Tone } from "../pip";
  export type { Mood, Outfit, Tone };

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
    sleepy: {
      eyesFill: "",
      eyesStroke: "M20 34.5q4 3.5 8 0M36 34.5q4 3.5 8 0",
      mouth: "M30 44.5q2 1.6 4 0",
      cheeks: eye(18, 41.5, 3, 2.2) + eye(46, 41.5, 3, 2.2),
      extra: "M50 9h6l-6 6h6M57.5 2h4l-4 4h4",
      tilt: 7,
    },
  };

  const LABELS: Record<Mood, string> = {
    idle: "Pip, idle",
    focused: "Pip, focused",
    curious: "Pip, curious",
    happy: "Pip, happy",
    dizzy: "Pip, dizzy",
    sleepy: "Pip, napping",
  };

  const WEARING: Record<Outfit, string> = {
    none: "",
    headphones: ", wearing headphones",
    beanie: ", wearing a beanie",
    glasses: ", wearing glasses",
    bow: ", wearing a bow",
    party: ", wearing a party hat",
  };
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { dur, ease, gsap, reduced, seconds } from "../motion";
  import { blinkDelay, canBlink, hidesSprout, shouldHop, wornOutfit } from "../pip";
  import { pipLook } from "../pipLook.svelte";

  let {
    mood = "idle",
    size = 24,
    tone = "peri",
    bob = false,
    pokeable = false,
    outfit,
    music,
    still = false,
  }: {
    mood?: Mood;
    size?: number;
    tone?: Tone;
    /** Gentle bounce in time with music (only while music plays). */
    bob?: boolean;
    /** Clicking Pip a few times makes it dizzy. */
    pokeable?: boolean;
    /** Show this outfit instead of the one picked in Settings (outfit picker). */
    outfit?: Outfit;
    /** Treat music as playing (Settings preview); otherwise follows the island. */
    music?: boolean;
    /** No blinking or hopping (small examples on the Settings page). */
    still?: boolean;
  } = $props();

  let svg = $state<SVGSVGElement>();
  let face = $state<SVGGElement>();
  let body = $state<SVGGElement>();
  let eyes = $state<SVGGElement>();
  let pokes: number[] = [];
  let poked = $state(false);
  const shown = $derived<Mood>(poked ? "dizzy" : mood);
  const f = $derived(FACES[shown] ?? FACES.idle);
  const worn = $derived<Outfit>(wornOutfit(outfit ?? pipLook.outfit, music ?? pipLook.music, pipLook.headphonesWithMusic));

  // Mood change: tilt to the new angle and give the face a quick squash.
  // Turning happy (Claude finished) adds a small hop.
  let first = true;
  let prev: Mood | undefined;
  $effect(() => {
    const tilt = f.tilt;
    const now = shown;
    if (!body || !face) return;
    const before = prev;
    prev = now;
    if (first || reduced()) {
      gsap.set(body, { rotation: tilt, svgOrigin: "32 36" });
      first = false;
      return;
    }
    gsap.to(body, { rotation: tilt, svgOrigin: "32 36", duration: dur("slow"), ease: ease(), overwrite: "auto" });
    gsap.fromTo(face, { scaleY: 0.6, svgOrigin: "32 38" }, { scaleY: 1, svgOrigin: "32 38", duration: dur("fast"), ease: ease() });
    if (!still && shouldHop(before, now, pipLook.celebrate)) {
      gsap.fromTo(body, { y: 0 }, { y: -7, duration: dur("fast"), ease: "power2.out", yoyo: true, repeat: 3 });
    }
  });

  // Blink: a quick squash of the eyes every 3.5–7.5 s, only for open-eyed
  // moods and only while the window is visible. One short tween per blink,
  // nothing runs in between.
  $effect(() => {
    if (still || !pipLook.blink || !canBlink(shown) || !eyes || reduced()) return;
    const el = eyes;
    let timer: ReturnType<typeof setTimeout>;
    const next = () => {
      timer = setTimeout(() => {
        if (document.visibilityState === "visible") {
          gsap.fromTo(el, { scaleY: 1 }, { scaleY: 0.1, svgOrigin: "32 34", duration: 0.07, ease: "power1.in", yoyo: true, repeat: 1 });
        }
        next();
      }, blinkDelay());
    };
    next();
    return () => {
      clearTimeout(timer);
      gsap.killTweensOf(el);
      gsap.set(el, { scaleY: 1, svgOrigin: "32 34" });
    };
  });

  // Music bob: a slow, small rise and fall in three steps while music plays
  // (the .bob class below). A CSS animation rather than a GSAP tween, so it
  // doesn't wake the page's script on every frame for as long as music plays.

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
    if (svg) gsap.killTweensOf([svg, body, face, eyes]);
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
  aria-label={LABELS[shown] + WEARING[worn]}
  class="pip tone-{tone}"
  class:bob
  onclick={poke}
>
  <g bind:this={body}>
    {#if !hidesSprout(worn)}<path class="sprout" d="M32 13c-2-5 1-9 7-10 0 5-3 9-7 10z" />{/if}
    <path class="body" d="M32 12c14 0 26 9 26 25 0 12-11 17-26 17S6 49 6 37C6 21 18 12 32 12z" stroke-width="1.5" />
    <path class="shine" d="M18 24c3-4 7-6 11-6.5" fill="none" stroke-width="3" stroke-linecap="round" />
    <g bind:this={face}>
      {#if f.cheeks}<path class="cheeks" d={f.cheeks} />{/if}
      <g bind:this={eyes}>
        {#if f.eyesFill}<path class="eyes" d={f.eyesFill} />{/if}
        {#if f.eyesStroke}<path class="lines" d={f.eyesStroke} stroke-width="3.2" />{/if}
      </g>
      <path class="lines" d={f.mouth} stroke-width="2.4" />
    </g>
    {#if worn === "headphones"}
      <path class="gear-band" d="M9 33C8 2 56 2 55 33" fill="none" stroke-width="3.4" stroke-linecap="round" />
      <rect class="gear" x="3.5" y="27" width="9" height="15" rx="4.2" stroke-width="1.4" />
      <rect class="gear" x="51.5" y="27" width="9" height="15" rx="4.2" stroke-width="1.4" />
      <path class="glint" d="M6.5 30.5v5M54.5 30.5v5" fill="none" stroke-width="1.6" stroke-linecap="round" />
    {:else if worn === "beanie"}
      <path class="knit" d="M8 24C8 -1 56 -1 56 24Z" />
      <path class="knit-rib" d="M20 10v11M32 6.5v14.5M44 10v11" fill="none" stroke-width="1.6" stroke-linecap="round" />
      <rect class="knit-brim" x="6" y="20.5" width="52" height="7" rx="3.5" />
      <circle class="knit-pom" cx="32" cy="4" r="4.4" stroke-width="1.2" />
    {:else if worn === "glasses"}
      <circle class="frames" cx="24" cy="34" r="7" fill="none" stroke-width="2.2" />
      <circle class="frames" cx="40" cy="34" r="7" fill="none" stroke-width="2.2" />
      <path class="frames" d="M31 33q1-1.4 2 0M17 32.5L7.5 30M47 32.5L56.5 30" fill="none" stroke-width="2.2" stroke-linecap="round" />
      <path class="glint" d="M19.6 31.4l2.4-2.4M35.6 31.4l2.4-2.4" fill="none" stroke-width="1.5" stroke-linecap="round" />
    {:else if worn === "bow"}
      <path class="bow" d="M20 16L10.5 10.5Q8.5 16 10.5 21.5ZM20 16L29.5 10.5Q31.5 16 29.5 21.5Z" stroke-width="1.2" stroke-linejoin="round" />
      <circle class="bow-knot" cx="20" cy="16" r="2.8" />
    {:else if worn === "party"}
      <path class="party" d="M21 15.5Q31.5 11.5 42.5 14.5L33 -7Z" stroke-width="1.2" stroke-linejoin="round" />
      <circle class="party-dot" cx="29" cy="9" r="1.8" />
      <circle class="party-dot" cx="36.5" cy="10" r="1.6" />
      <circle class="party-dot" cx="32.5" cy="2.5" r="1.5" />
      <circle class="bow" cx="33" cy="-7" r="3" stroke-width="1" />
    {/if}
  </g>
  {#if f.extra}<path class="extra" d={f.extra} stroke-width="2.6" />{/if}
</svg>

<style>
  .pip { display: block; flex: none; overflow: visible; }
  .pip.bob { animation: pip-bob var(--dur-bob) steps(3, end) infinite alternate; }
  :global([data-reduce-motion="true"]) .pip.bob { animation: none; }
  @keyframes pip-bob { to { transform: translateY(calc(-1 * var(--bob-height))); } }
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
  .gear-band { stroke: var(--pip-gear); }
  .gear { fill: var(--pip-gear); stroke: var(--pip-gear-edge); }
  .glint { stroke: var(--pip-glint); }
  .knit { fill: var(--pip-knit); }
  .knit-rib { stroke: var(--pip-knit-edge); }
  .knit-brim { fill: var(--pip-knit-edge); }
  .knit-pom { fill: var(--pip-knit-pom); stroke: var(--pip-knit-edge); }
  .frames { stroke: var(--pip-face); }
  .bow { fill: var(--pip-bow); stroke: var(--pip-bow-edge); }
  .bow-knot { fill: var(--pip-bow-edge); }
  .party { fill: var(--pip-party); stroke: var(--pip-party-edge); }
  .party-dot { fill: var(--pip-bow); }
</style>
