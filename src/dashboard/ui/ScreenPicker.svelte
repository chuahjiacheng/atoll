<!--
  General → Position (design: mini screen). Drag the island on a small copy
  of the chosen display, or select it and use the arrow keys (Shift for
  bigger steps). The real island moves as you go. Placement maths mirror
  windows::place_island in Rust: the centre follows X, but the widest state
  keeps 16 px from both screen edges.
-->
<script lang="ts">
  import GButton from "../../lib/glass/GButton.svelte";
  import { px } from "../../lib/theme";
  import type { Appearance, MonitorInfo } from "../../lib/settings";

  let {
    a,
    monitor,
    onmove,
  }: { a: Appearance; monitor: MonitorInfo | null; onmove: (x: number, y: number) => void } = $props();

  const MAX_Y = 400; // settings.rs clamps position_y to 0..400

  // Logical (CSS) size of the display.
  const W = $derived(monitor ? monitor.width / monitor.scale : 1920);
  const H = $derived(monitor ? monitor.height / monitor.scale : 1080);
  const boxW = $derived(px("--size-screen-w"));
  const k = $derived(boxW / W);
  const boxH = $derived(Math.round(H * k));
  const edge = $derived(px("--sp-16"));

  const zoom = $derived(a.scale / 100);
  const islandW = $derived(a.collapsedWidth * zoom);
  const islandH = $derived(a.collapsedHeight * zoom);
  const widest = $derived(Math.max(a.expandedWidth, a.collapsedWidth, px("--size-peek-w")) * zoom);
  const cx = $derived.by(() => {
    const want = (W * a.positionX) / 100;
    const lo = widest / 2 + edge;
    const hi = W - widest / 2 - edge;
    return hi < lo ? W / 2 : Math.min(hi, Math.max(lo, want));
  });
  const centred = $derived(a.positionX === 50);
  const fromLeft = $derived(Math.round(cx - islandW / 2));

  const pw = $derived(Math.max(px("--sp-6"), islandW * k));
  const ph = $derived(Math.max(px("--sp-6"), islandH * k));
  const pr = $derived((ph / 2) * (a.roundness / 100));

  const hText = $derived(centred ? "Horizontal: centred" : `Horizontal: ${fromLeft} px from the left edge`);
  const vText = $derived(`Vertical: ${a.positionY} px from the top edge`);

  let screen = $state<HTMLElement>();
  let dragging = false;

  function moveTo(e: PointerEvent) {
    if (!screen) return;
    const b = screen.getBoundingClientRect();
    let x = ((e.clientX - b.left) / b.width) * 100;
    if (Math.abs(x - 50) < 2) x = 50; // snap to centre
    const y = (e.clientY - b.top) / k - islandH / 2;
    onmove(Math.round(Math.min(100, Math.max(0, x))), Math.round(Math.min(MAX_Y, Math.max(0, y))));
  }

  function key(e: KeyboardEvent) {
    const big = e.shiftKey;
    let dx = 0;
    let dy = 0;
    if (e.key === "ArrowLeft") dx = big ? -5 : -1;
    else if (e.key === "ArrowRight") dx = big ? 5 : 1;
    else if (e.key === "ArrowUp") dy = big ? -16 : -4;
    else if (e.key === "ArrowDown") dy = big ? 16 : 4;
    else return;
    e.preventDefault();
    onmove(Math.min(100, Math.max(0, a.positionX + dx)), Math.min(MAX_Y, Math.max(0, a.positionY + dy)));
  }
</script>

<div class="wrap">
  <div class="screen" bind:this={screen} style:width="{boxW}px" style:height="{boxH}px">
    <svg width={boxW} height={boxH} aria-hidden="true">
      <rect x="3" y="3" width={boxW - 6} height={boxH - 16} rx="4" class="frame" />
      <rect x="1" y={boxH - 11} width={boxW - 2} height="10" rx="4" class="taskbar" />
      {#if centred}<line x1={boxW / 2} y1="0" x2={boxW / 2} y2={boxH - 11} class="centre" />{/if}
    </svg>
    <button
      class="handle"
      style:left="{cx * k}px"
      style:top="{a.positionY * k + ph / 2}px"
      aria-label="Island position: {centred ? 'centred' : `${fromLeft} pixels from the left`}, {a.positionY} pixels from the top. Use the arrow keys to move it."
      onpointerdown={(e) => {
        dragging = true;
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onpointermove={(e) => dragging && moveTo(e)}
      onpointerup={() => (dragging = false)}
      onpointercancel={() => (dragging = false)}
      onkeydown={key}
    >
      <span class="pill" style:width="{pw}px" style:height="{ph}px" style:border-radius="{pr}px"></span>
    </button>
  </div>
  <div class="side">
    <span class="t">Island position</span>
    <span class="help">Drag the island on the screen, or select it and use the arrow keys. Hold Shift for bigger steps. It keeps 16 px from both sides.</span>
    <span class="read first tabular">{hText}</span>
    <span class="read tabular">{vText}</span>
    <span class="btn-row"><GButton label="Centre horizontally" disabled={centred} onclick={() => onmove(50, a.positionY)} /></span>
  </div>
</div>

<style>
  .wrap {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-24);
    align-items: flex-start;
  }
  .screen {
    position: relative;
    flex: none;
    border-radius: var(--r-nav);
    background: var(--c-desk);
    box-shadow: inset 0 0 0 1px var(--c-ctrl);
  }
  svg {
    position: absolute;
    inset: 0;
  }
  .frame {
    fill: none;
    stroke: var(--c-ctrl);
    stroke-dasharray: 3 3;
    opacity: var(--alpha-disabled);
  }
  .taskbar {
    fill: var(--c-desk-bar);
  }
  .centre {
    stroke: var(--c-accent);
    stroke-dasharray: 3 3;
  }
  .handle {
    position: absolute;
    translate: -50% -50%;
    padding: var(--sp-8);
    border-radius: var(--r-nav);
    cursor: grab;
    touch-action: none;
    display: flex;
  }
  .handle:active {
    cursor: grabbing;
  }
  .pill {
    display: block;
    background: rgb(var(--c-hull-rgb));
    box-shadow: 0 0 0 var(--focus-width) var(--c-accent);
  }
  .side {
    flex: 1;
    min-width: var(--size-art-lg);
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
  }
  .help {
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .read {
    font-size: var(--fs-small);
  }
  .read.first {
    margin-top: var(--sp-6);
  }
  .btn-row {
    margin-top: var(--sp-8);
    display: flex;
  }
</style>
