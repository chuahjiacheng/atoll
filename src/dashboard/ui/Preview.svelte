<!--
  Appearance → Live preview (design: sticky preview card). A small desk with
  the island drawn from the current settings, collapsed (idle, busy, done,
  error) or expanded at 25 %. Sizes animate with GSAP using the island's own
  durations; choosing Expanded also opens the real island so you can size it.
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { emitTo } from "@tauri-apps/api/event";
  import Icon, { type IconName } from "../../lib/ui/Icon.svelte";
  import Pip, { type Mood } from "../../lib/ui/Pip.svelte";
  import { dur, ease, gsap, reduced } from "../../lib/motion";
  import { px } from "../../lib/theme";
  import type { Appearance, Settings } from "../../lib/settings";
  import Seg from "./Seg.svelte";

  let { a, tone }: { a: Appearance; tone: Settings["mascot"]["tone"] } = $props();

  type Status = "idle" | "busy" | "done" | "error";
  let mode = $state<"collapsed" | "expanded">("collapsed");
  let status = $state<Status>("busy");

  const LOOK: Record<Exclude<Status, "idle">, { mood: Mood; icon: IconName; word: string; label: string; meta: string }> = {
    busy: { mood: "focused", icon: "busy", word: "Busy", label: "Refactoring the login form", meta: "2:14" },
    done: { mood: "happy", icon: "check", word: "Done", label: "Task finished", meta: "now" },
    error: { mood: "dizzy", icon: "alert", word: "Error", label: "Build failed", meta: "" },
  };

  const zoom = $derived(a.scale / 100);
  const idle = $derived(status === "idle");
  const look = $derived(idle ? null : LOOK[status as Exclude<Status, "idle">]);
  // Collapsed size as it appears on screen (scale included).
  const w = $derived(Math.round((idle ? a.collapsedWidth / 3 : a.collapsedWidth) * zoom));
  const h = $derived(Math.round(a.collapsedHeight * zoom));
  const r = $derived(Math.round((a.collapsedHeight / 2) * (a.roundness / 100)));
  const k = $derived(px("--preview-scale"));
  const ew = $derived(Math.round(a.expandedWidth * zoom * k));
  const eh = $derived(Math.round(a.expandedHeight * zoom * k));
  const er = $derived(Math.round(px("--r-panel") * (a.roundness / 100)));
  const pipSize = $derived(a.collapsedHeight * zoom >= px("--size-tab") ? px("--size-pip") : px("--size-icon"));

  const caption = $derived(
    mode === "collapsed"
      ? `Collapsed ${idle ? Math.round(a.collapsedWidth / 3) : a.collapsedWidth} × ${a.collapsedHeight} px · scale ${a.scale}% · corners ${r} px · opacity ${a.opacity}% · blur ${a.blur} px`
      : `Expanded ${a.expandedWidth} × ${a.expandedHeight} px, drawn at ${Math.round(k * 100)}% · scale ${a.scale}% · corners ${er} px · opacity ${a.opacity}%`,
  );
  const aria = $derived(
    mode === "collapsed"
      ? `Island preview, collapsed, ${idle ? "idle" : `${look!.word}: ${look!.label}`}, ${w} by ${h} pixels`
      : `Island preview, expanded, ${a.expandedWidth} by ${a.expandedHeight} pixels`,
  );

  let pill = $state<HTMLElement>();
  let panel = $state<HTMLElement>();

  // Size changes ease like the island does; a new element or Reduce motion sets them at once.
  const seen = new WeakSet<HTMLElement>();
  function size(el: HTMLElement | undefined, to: gsap.TweenVars) {
    if (!el) return;
    if (!seen.has(el) || reduced()) {
      seen.add(el);
      gsap.set(el, to);
    } else gsap.to(el, { ...to, duration: dur("expand"), ease: ease(), overwrite: "auto" });
  }
  $effect(() => size(pill, { width: w, height: h, borderRadius: Math.round(r * zoom) }));
  $effect(() => size(panel, { width: ew, height: eh, borderRadius: Math.round(er * zoom * k) }));

  // Switching mode: the new shape grows in from the pill (collapse shrinks back).
  function pickMode(m: string) {
    mode = m as typeof mode;
    emitTo("island", "preview-expand", mode === "expanded").catch(() => {});
    requestAnimationFrame(() => {
      const el = mode === "expanded" ? panel : pill;
      if (!el) return;
      if (reduced()) gsap.fromTo(el, { autoAlpha: 0 }, { autoAlpha: 1, duration: dur("fast") });
      else
        gsap.fromTo(
          el,
          { autoAlpha: 0, scale: mode === "expanded" ? k * 2 : 1.2 },
          { autoAlpha: 1, scale: 1, duration: dur(mode === "expanded" ? "expand" : "collapse"), ease: ease(), clearProps: "scale" },
        );
    });
  }

  onDestroy(() => {
    if (mode === "expanded") emitTo("island", "preview-expand", false).catch(() => {});
  });
</script>

<div class="card">
  <div class="top">
    <span class="title">Live preview</span>
    <Seg
      label="Preview size"
      items={[
        { id: "collapsed", label: "Collapsed" },
        { id: "expanded", label: "Expanded" },
      ]}
      value={mode}
      onchange={pickMode}
    />
    <Seg
      label="Preview status"
      items={[
        { id: "idle", label: "Idle" },
        { id: "busy", label: "Busy" },
        { id: "done", label: "Done" },
        { id: "error", label: "Error" },
      ]}
      value={status}
      narrow
      disabled={mode === "expanded"}
      onchange={(v) => (status = v as Status)}
    />
  </div>

  <div class="desk" role="img" aria-label={aria}>
    <!-- desk scenery (drawing coordinates) -->
    <svg class="scene left" width="300" height="200" aria-hidden="true">
      <rect x="24" y="56" width="260" height="140" rx="8" class="win" />
      <rect x="40" y="76" width="140" height="8" rx="4" class="bar" />
      <rect x="40" y="94" width="200" height="8" rx="4" class="bar" />
    </svg>
    <svg class="scene mid" width="340" height="90" viewBox="0 0 340 90" aria-hidden="true">
      <rect x="0" y="12" width="150" height="44" rx="22" class="blockA" />
      <rect x="230" y="4" width="80" height="80" rx="40" class="blockB" />
    </svg>

    {#if mode === "collapsed"}
      <div
        class="pill"
        class:idle
        bind:this={pill}
        style:--pv-c={look ? `var(--c-${status})` : "var(--c-muted)"}
        style:--pv-alpha={idle ? "var(--hull-alpha)" : "var(--pill-alpha)"}
      >
        <Pip mood={look?.mood ?? "idle"} {tone} size={pipSize} />
        {#if look}
          <span class="word"><Icon name={look.icon} size={16} /><span>{look.word}</span></span>
          <span class="label">{look.label}</span>
          <span class="meta tabular">{look.meta}</span>
        {:else}
          <span class="grow"></span>
          <span class="dot"></span>
        {/if}
      </div>
    {:else}
      <div class="panel" bind:this={panel}>
        <svg class="chips" width="116" height="6" viewBox="0 0 116 6" aria-hidden="true">
          {#each [0, 1, 2, 3, 4, 5] as i (i)}<rect x={i * 20} y="0" width="16" height="6" rx="3" class:on={i === 0} />{/each}
        </svg>
        <div class="blocks"><span></span><span></span></div>
      </div>
    {/if}
  </div>
  <span class="caption tabular">{caption}{mode === "expanded" ? " · the real island is open too" : ""}</span>
</div>

<style>
  .card {
    border-radius: var(--r-card);
    background: var(--c-card);
    box-shadow: 0 0 0 1px var(--c-line);
    padding: var(--sp-16);
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    flex-wrap: wrap;
  }
  .title {
    flex: 1;
    min-width: var(--size-seg);
    font-weight: var(--fw-strong);
  }
  .desk {
    position: relative;
    height: var(--size-preview-h);
    border-radius: var(--r-control);
    background: var(--c-desk);
    overflow: hidden;
  }
  .scene {
    position: absolute;
    top: 0;
  }
  .scene.left {
    left: 0;
  }
  .scene.mid {
    left: 50%;
    translate: -50% 0;
  }
  .win {
    fill: var(--c-desk-win);
  }
  .bar {
    fill: var(--c-desk-bar);
  }
  .blockA {
    fill: var(--c-desk-block-a);
  }
  .blockB {
    fill: var(--c-desk-block-b);
  }
  .pill,
  .panel {
    position: absolute;
    left: 50%;
    translate: -50% 0;
    box-sizing: border-box;
    background: rgba(var(--c-hull-rgb), var(--pv-alpha, var(--hull-alpha)));
    color: var(--c-ink);
  }
  .pill {
    top: var(--sp-16);
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    padding: 0 var(--sp-12) 0 var(--sp-8);
    font-size: var(--fs-small);
    overflow: hidden;
    box-shadow:
      0 0 0 var(--glow-spread) var(--pv-c),
      0 0 0 var(--glow-gap) rgb(var(--c-hull-rgb)),
      0 0 var(--glow-blur) var(--glow-size) color-mix(in srgb, var(--pv-c) var(--glow-strength), transparent);
  }
  .pill.idle {
    box-shadow: var(--shadow-preview);
  }
  .word {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    flex: none;
    color: var(--pv-c);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
  }
  .label {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    flex: none;
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
  .grow {
    flex: 1;
  }
  .dot {
    width: var(--sp-8);
    height: var(--sp-8);
    border-radius: var(--r-pill);
    flex: none;
    background: var(--c-muted);
  }
  .panel {
    top: var(--sp-10);
    padding: var(--sp-10);
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
    box-shadow: var(--shadow-preview);
    overflow: hidden;
  }
  .chips rect {
    fill: var(--c-ctrl);
  }
  .chips rect.on {
    fill: var(--c-accent);
  }
  .blocks {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-6);
  }
  .blocks span {
    border-radius: var(--r-keycap);
    background: var(--c-sel);
  }
  .caption {
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
</style>
