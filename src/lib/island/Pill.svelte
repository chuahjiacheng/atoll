<!--
  Collapsed island contents (design: Island → collapsed views).
  idle   : Pip + one quiet dot
  status : Pip in the matching mood, status icon + word, one line, time
  music  : album art, title · artist (or the lyric line being sung), equaliser icon
           (progress line is on the shell). With lyrics on, each new line rolls
           up from below the pill to its middle while the old one leaves above.
  timer  : focus timer: small ring, time, "Focus" / "Break" / "Paused", and your
           intention or a distraction nudge when there is one
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon, { type IconName } from "../ui/Icon.svelte";
  import Pip, { type Mood, type Tone } from "../ui/Pip.svelte";
  import { dur, gsap, reduced, seconds } from "../motion";
  import { px } from "../theme";
  import Eq from "../ui/Eq.svelte";

  let {
    kind,
    tone,
    mood = "idle",
    icon = "dot",
    word = "",
    tint = "busy",
    text = "",
    meta = "",
    title = "",
    artist = "",
    lyric = null,
    lyricAt = null,
    roll = false,
    art = null,
    playing = false,
    ring = 0,
    time = "",
    bob = false,
    width = null,
    label,
    onclick,
  }: {
    kind: "idle" | "status" | "music" | "timer";
    tone: Tone;
    mood?: Mood;
    icon?: IconName;
    word?: string;
    /** Which colour the status word uses. Timer: "accent" = a nudge, "done" = a break, else quiet. */
    tint?: "busy" | "done" | "error" | "accent";
    text?: string;
    meta?: string;
    title?: string;
    artist?: string;
    /** Music: the synced line being sung; replaces title · artist when set. */
    lyric?: string | null;
    /** Music: when that line starts (ms), so a repeated line still rolls in as a new one. */
    lyricAt?: number | null;
    /** Music: lines (and the song name around them) roll in and out instead of switching. */
    roll?: boolean;
    art?: string | null;
    /** Music: the equaliser bounces while this is true. */
    playing?: boolean;
    /** Timer: 0..1 through the session (the small ring). */
    ring?: number;
    /** Timer: "24:13" */
    time?: string;
    bob?: boolean;
    /** The pill's own final width (px), from its width contract. It keeps this
     *  width while the island around it morphs, so its text never reflows. */
    width?: number | null;
    label: string;
    onclick: () => void;
  } = $props();

  // The busy arc turns while an agent works (stops otherwise; never on reduced motion).
  let spinner = $state<HTMLSpanElement>();
  let spin: gsap.core.Tween | null = null;
  $effect(() => {
    spin?.kill();
    spin = null;
    if (spinner && icon === "busy" && !reduced()) {
      spin = gsap.to(spinner, { rotation: 360, duration: seconds("--dur-spin"), ease: "none", repeat: -1 });
    }
  });
  onMount(() => () => spin?.kill());

  // Timer ring: r 9 in a 24 box, so a 3 px stroke fits.
  const MINI_R = 9;
  const MINI_C = 2 * Math.PI * MINI_R;

  // One key per thing shown: each sung line (by its start time), or the song between lines.
  const rollKey = $derived(roll ? (lyric !== null ? `line ${lyricAt ?? lyric}` : `song ${title}\n${artist}`) : "");

  /**
   * A line glides up from below the pill to the middle (`from: 1`), or from the
   * middle up and out (`from: -1`), fading as it goes. Both use one even in-out
   * curve, so the old and new lines move together like a single roll (the
   * island's settle curve front-loads the move and reads as a jump here).
   * Reduced motion: a short fade.
   */
  function rollLine(_node: Element, { from }: { from: 1 | -1 }) {
    if (!roll) return { duration: 0 };
    const duration = dur("lyric") * 1000;
    if (reduced()) return { duration, css: (t: number) => `opacity: ${t}` };
    const distance = px("--pill-h") * 0.6;
    return {
      duration,
      easing: gsap.parseEase("power2.inOut"),
      css: (t: number, u: number) => `opacity: ${t}; transform: translateY(${from * u * distance}px)`,
    };
  }

  /**
   * One state's content making way for the next: the old one blurs and
   * shrinks away quickly; the new one arrives a beat later, once the island
   * has started moving to its size. Each keeps its own final width throughout.
   * Reduced motion: a plain cross-fade.
   */
  /** Text changing within one state: a short cross-fade, no movement. */
  function fadeLine(_node: Element) {
    return { duration: dur("fast") * 1000, css: (t: number) => `opacity: ${t}` };
  }

  function swap(node: Element, { arriving }: { arriving: boolean }) {
    // The leaving copy is out of reach at once: no click, no focus, and screen
    // readers never meet two pills.
    if (!arriving) node.setAttribute("inert", "");
    if (reduced()) return { duration: seconds("--dur-reduced") * 1000, css: (t: number) => `opacity: ${t}` };
    const speed = px("--speed") || 1;
    const blur = px("--swap-blur");
    const shrink = 1 - (px("--swap-scale") || 1);
    return {
      delay: arriving ? (seconds("--dur-swap-delay") * 1000) / speed : 0,
      duration: (seconds(arriving ? "--dur-swap-in" : "--dur-swap-out") * 1000) / speed,
      easing: gsap.parseEase(arriving ? "power2.out" : "power1.in"),
      css: (t: number, u: number) => `opacity: ${t}; filter: blur(${u * blur}px); transform: scale(${1 - u * shrink})`,
    };
  }
</script>

<button
  class="pill {kind}"
  style:width={width ? `${width}px` : null}
  aria-label={label}
  title={text || title || undefined}
  {onclick}
  in:swap|global={{ arriving: true }}
  out:swap|global={{ arriving: false }}
>
  {#if kind === "music"}
    {#if art}
      <img class="art" src={art} alt="" />
    {:else}
      <span class="art blank" aria-hidden="true"><Icon name="music" size={14} /></span>
    {/if}
    <span class="line rows">
      {#key rollKey}
        <span class="row" class:pair={!lyric} dir="auto" in:rollLine={{ from: 1 }} out:rollLine={{ from: -1 }}>
          {#if lyric}
            <span class="strong">{lyric}</span>
          {:else}
            <!-- The separator is an expression: Svelte drops plain leading spaces
                 inside an element, which glued the dot to the title. -->
            <span class="strong">{title}</span><span class="muted">{` · ${artist}`}</span>
          {/if}
        </span>
      {/key}
    </span>
    <span class="eq" aria-hidden="true"><Eq {playing} size={16} /></span>
  {:else if kind === "timer"}
    <svg class="mini {tint}" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r={MINI_R} class="track" />
      {#if ring > 0}
        <circle
          cx="12"
          cy="12"
          r={MINI_R}
          class="arc"
          stroke-dasharray="{MINI_C * Math.min(1, ring)} {MINI_C}"
          transform="rotate(-90 12 12)"
        />
      {/if}
    </svg>
    <span class="time tabular">{time}</span>
    <span class="tword {tint}">{word}</span>
    {#if text}<span class="line note" dir="auto">{text}</span>{/if}
  {:else}
    <Pip {mood} {tone} {bob} size={24} />
    {#if kind === "idle"}
      <span class="flex"></span>
      <span class="dot" aria-hidden="true"></span>
    {:else}
      <span class="word {tint}">
        <span class="spin" bind:this={spinner}><Icon name={icon} size={16} /></span>
        <span>{word}</span>
      </span>
      <!-- A new step or message fades over the old one instead of switching. -->
      <span class="line rows">
        {#key text}
          <span class="row" dir="auto" in:fadeLine out:fadeLine>{text}</span>
        {/key}
      </span>
      {#if meta}<span class="meta tabular">{meta}</span>{/if}
    {/if}
  {/if}
</button>

<style>
  .pill {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    width: 100%;
    height: var(--pill-h);
    box-sizing: border-box;
    padding: 0 var(--sp-16) 0 var(--sp-8);
    text-align: left;
    white-space: nowrap;
  }
  /* Album art sits a little in from the rounded end, not pressed into its curve. */
  .pill.music { padding-left: var(--sp-12); }
  .flex { flex: 1; }
  .dot {
    width: var(--sp-8);
    height: var(--sp-8);
    border-radius: var(--r-pill);
    background: var(--c-muted);
    flex: none;
  }
  .word {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    flex: none;
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
  }
  .word.busy { color: var(--c-busy); }
  .word.done { color: var(--c-done); }
  .word.error { color: var(--c-error); }
  .word.accent { color: var(--c-accent-text); }
  .spin { display: flex; }
  .line {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-small);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .strong { font-weight: var(--fw-strong); }
  .muted, .meta { color: var(--c-muted); }
  .meta { flex: none; font-size: var(--fs-label); }
  .art {
    width: var(--size-pip);
    height: var(--size-pip);
    border-radius: var(--r-keycap);
    flex: none;
    object-fit: cover;
  }
  .art.blank { display: grid; place-items: center; background: var(--c-raised); color: var(--c-muted); }
  .eq { color: var(--c-accent); display: flex; flex: none; }
  /* Music line: the full pill height, so a rolling line enters at the bottom
     edge and leaves at the top edge. The leaving and arriving rows share one
     grid cell while they pass each other; each keeps its own ellipsis. */
  .rows {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    align-items: center;
    align-self: stretch;
  }
  /* Title · artist at the widest the pill gets: both give way together, each
     with its own ellipsis, so a long title never pushes the artist out. */
  .row.pair { display: flex; }
  .pair > .strong,
  .pair > .muted {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
  }
  .row {
    grid-area: 1 / 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Focus timer pill (design: Island → collapsed timer). */
  .pill.timer { padding-left: var(--sp-8); }
  .mini {
    width: var(--size-ring-pill);
    height: var(--size-ring-pill);
    flex: none;
    fill: none;
    stroke-width: var(--stroke-ring-pill);
  }
  .mini .track { stroke: var(--c-track); }
  .mini .arc { stroke: var(--c-accent); stroke-linecap: round; transition: stroke-dasharray var(--dur-slow) linear; }
  .mini.done .arc { stroke: var(--c-done); }
  .time {
    flex: none;
    font-size: var(--fs-body);
    font-weight: var(--fw-strong);
  }
  .tword {
    flex: none;
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
  .tword.accent { color: var(--c-accent-text); font-weight: var(--fw-strong); }
  .tword.done { color: var(--c-done); }
  .note { color: var(--c-ink); }
</style>
