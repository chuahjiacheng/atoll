<!--
  Dev overlay (debug builds): the island's state, target width, spring and
  frame times, in a small readout under the island. It redraws only on frames
  the island was drawing anyway and on state changes: it has no timer or loop
  of its own, so it shows "loop off" truthfully while the island rests.
  Toggle: Ctrl + Shift + click the island.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { metrics, type IslandMetrics } from "./metrics";
  import { measurer } from "./measure";

  let m = $state<IslandMetrics>(metrics.snapshot());
  let cache = $state({ size: 0, hits: 0, misses: 0 });

  onMount(() => {
    const read = () => {
      m = metrics.snapshot();
      cache = { size: measurer.cacheSize, hits: measurer.hits, misses: measurer.misses };
    };
    const stops = [metrics.onEveryFrame(read), metrics.subscribe(read)];
    read();
    return () => stops.forEach((stop) => stop());
  });

  const n = (v: number, digits = 0) => v.toFixed(digits);
  const over = $derived(m.frames.p95 > m.frames.budgetMs * 1.2);
</script>

<dl class="overlay tabular" aria-hidden="true">
  <div><dt>State</dt><dd>{m.state.id}{m.state.previous ? ` (was ${m.state.previous})` : ""}</dd></div>
  <div><dt>Why</dt><dd>{m.state.reason}</dd></div>
  <div><dt>Width</dt><dd>{n(m.width.current, 1)} → {n(m.width.target)} px, limits {n(m.width.min)}–{n(m.width.max)}</dd></div>
  <div>
    <dt>Spring</dt>
    <dd class:live={m.loopRunning}>
      {m.loopRunning ? "moving" : "at rest, loop off"}, {n(m.width.velocity)} px/s, {m.width.retargets} retargets
    </dd>
  </div>
  <div>
    <dt>Frames</dt>
    <dd class:bad={over || m.frames.dropped > 0}>
      {n(m.frames.p50, 1)} / {n(m.frames.p95, 1)} / {n(m.frames.max, 1)} ms (budget {n(m.frames.budgetMs, 1)}), {m.frames.dropped} dropped of {m.frames.count}
    </dd>
  </div>
  <div><dt>Jump</dt><dd>{n(m.frames.longestJumpPx, 1)} px largest step</dd></div>
  <div><dt>Text</dt><dd>{cache.size} cached, {cache.hits} hits, {cache.misses} measured</dd></div>
</dl>

<style>
  .overlay {
    margin: var(--sp-8) 0 0;
    padding: var(--sp-8) var(--sp-12);
    display: grid;
    gap: var(--sp-2);
    border-radius: var(--r-control);
    background: rgba(var(--c-hull-rgb), 0.92);
    color: var(--c-ink);
    font-size: var(--fs-label);
    line-height: var(--lh-tight);
    pointer-events: none;
    white-space: nowrap;
  }
  .overlay div {
    display: grid;
    grid-template-columns: var(--sp-48) auto;
    gap: var(--sp-8);
  }
  dt { color: var(--c-muted); }
  dd { margin: 0; }
  .live { color: var(--c-busy); }
  .bad { color: var(--c-error); }
</style>
