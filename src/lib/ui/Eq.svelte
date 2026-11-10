<!--
  Equaliser icon for the music pill: four bars that sway smoothly while music
  plays and glide back to rest when it stops.

  Motion: each bar eases up and down through its own loop of heights. The
  loops have slightly different lengths, so the four drift against each other
  and never fall into a visible repeat. The loops are CSS animations on
  transform only, so the compositor runs them without touching the page's
  main thread. Static with Reduce motion.

  Shape: each bar is a rounded slot with a rounded fill sliding up and down
  inside it (not a stretched bar), so both ends stay round at every height.
-->
<script lang="ts">
  import { reduced, seconds } from "../motion";
  import { token } from "../theme";

  let { playing, size = 16 }: { playing: boolean; size?: number } = $props();

  /** Resting height of each bar (fraction of the icon). Matches keyframe 0. */
  const REST = [0.45, 0.85, 0.6, 0.35];
  const rest = (i: number) => `translateY(${(1 - REST[i]) * 100}%)`;

  let fills: HTMLSpanElement[] = $state([]);
  let was = false; // set on mount by the pre-effect below
  let from: string[] | null = null;

  // Before the DOM drops the loop, note where each bar is mid-swing...
  $effect.pre(() => {
    const now = playing;
    if (was && !now && !reduced()) from = fills.map((f) => getComputedStyle(f).transform);
    was = now;
  });

  // ...then glide from there to rest instead of snapping. Starting again
  // cancels the glide; the loop starts from the resting height.
  $effect(() => {
    const start = from;
    from = null;
    for (const f of fills) for (const a of f.getAnimations()) if (!(a instanceof CSSAnimation)) a.cancel();
    if (playing || !start || !fills[0]) return;
    // Read tokens here, not from the root: the Settings preview sets its own --speed.
    const speed = parseFloat(token("--speed", fills[0])) || 1;
    const duration = (seconds("--dur-eq", fills[0]) * 1000 * 1.6) / speed;
    const easing = token("--ease-settle", fills[0]) || "ease-out";
    fills.forEach((f, i) => {
      if (start[i] && start[i] !== "none") f.animate([{ transform: start[i] }, { transform: rest(i) }], { duration, easing });
    });
  });
</script>

<span class="eq" class:playing style="width: {size}px; height: {size}px" aria-hidden="true">
  {#each [0, 1, 2, 3] as i}
    <span class="slot"><span class="fill b{i}" bind:this={fills[i]}></span></span>
  {/each}
</span>

<style>
  .eq {
    display: inline-flex;
    align-items: stretch;
    justify-content: space-between;
    flex: none;
    padding: 1px;
    box-sizing: border-box;
    transition: opacity calc(var(--dur-eq) / var(--speed)) var(--ease-settle);
  }
  .eq:not(.playing) { opacity: 0.6; }

  .slot {
    width: 18%;
    border-radius: var(--r-pill);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: var(--r-pill);
    background: currentColor;
  }

  /* resting heights (keep in step with REST in the script) */
  .b0 { transform: translateY(55%); }
  .b1 { transform: translateY(15%); }
  .b2 { transform: translateY(40%); }
  .b3 { transform: translateY(65%); }

  /* A swing is one ease from a high to a low (or back): --dur-eq long on
     average. Every keyframe interval eases in and out, so the bars slow into
     each turn and never jump. */
  .playing .fill {
    animation-timing-function: cubic-bezier(0.45, 0, 0.55, 1);
    animation-iteration-count: infinite;
  }
  .playing .b0 { animation-name: eq0; animation-duration: calc(var(--dur-eq) * 6 / var(--speed)); }
  .playing .b1 { animation-name: eq1; animation-duration: calc(var(--dur-eq) * 5.93 / var(--speed)); }
  .playing .b2 { animation-name: eq2; animation-duration: calc(var(--dur-eq) * 6.03 / var(--speed)); }
  .playing .b3 { animation-name: eq3; animation-duration: calc(var(--dur-eq) * 5.87 / var(--speed)); }

  /* translateY(x%) leaves (100 - x)% of the bar showing */
  @keyframes eq0 {
    0%, 100% { transform: translateY(55%); }
    16.67% { transform: translateY(5%); }
    31.11% { transform: translateY(65%); }
    48.89% { transform: translateY(20%); }
    64.44% { transform: translateY(70%); }
    83.33% { transform: translateY(0%); }
  }
  @keyframes eq1 {
    0%, 100% { transform: translateY(15%); }
    15.73% { transform: translateY(60%); }
    34.27% { transform: translateY(0%); }
    48.31% { transform: translateY(50%); }
    65.73% { transform: translateY(10%); }
    82.02% { transform: translateY(65%); }
  }
  @keyframes eq2 {
    0%, 100% { transform: translateY(40%); }
    17.68% { transform: translateY(0%); }
    32.6% { transform: translateY(65%); }
    49.17% { transform: translateY(25%); }
    68.51% { transform: translateY(55%); }
    82.87% { transform: translateY(5%); }
  }
  @keyframes eq3 {
    0%, 100% { transform: translateY(65%); }
    14.77% { transform: translateY(30%); }
    32.39% { transform: translateY(70%); }
    48.86% { transform: translateY(5%); }
    64.2% { transform: translateY(60%); }
    82.95% { transform: translateY(20%); }
  }

  :global([data-reduce-motion="true"]) .playing .fill { animation: none; }
</style>
