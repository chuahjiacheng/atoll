<!--
  Expanded island (design: Island → expanded). Header plate, module tabs,
  and the modules laid side by side in one strip. Changing tab glides the
  strip to that module with Lenis (driven by GSAP's ticker, only while the
  panel is open); Reduce motion jumps and fades instead.
-->
<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import Lenis from "lenis";
  import "lenis/dist/lenis.css";
  import type { Status } from "../api";
  import { STATUS_LOOK } from "../format";
  import { dur, ease, enter, gsap, reduced } from "../motion";
  import GButton from "../glass/GButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";
  import Tabs, { type TabDef } from "./Tabs.svelte";

  let {
    tabs,
    active,
    status,
    tone,
    onselect,
    onSettings,
    onCollapse,
    onQuit,
    pane,
  }: {
    tabs: TabDef[];
    active: string;
    status: Status;
    tone: Tone;
    onselect: (id: string) => void;
    onSettings: () => void;
    onCollapse: () => void;
    onQuit: () => void;
    pane: Snippet<[string]>;
  } = $props();

  let wrap = $state<HTMLDivElement>();
  // Tabs are built the first time they are opened, then kept.
  let visited = $state<string[]>([]);
  $effect(() => {
    if (!visited.includes(active)) visited = [...visited, active];
  });
  let strip = $state<HTMLDivElement>();
  let lenis: Lenis | null = null;
  const look = $derived(STATUS_LOOK[status]);
  // Icon-only tabs: the header names the open tab instead.
  let compactTabs = $state(false);
  const activeLabel = $derived(tabs.find((t) => t.id === active)?.label ?? "");
  // Quit takes two clicks: the first turns the button into "Quit?" for a moment.
  let quitArmed = $state(false);
  let quitTimer: ReturnType<typeof setTimeout> | undefined;
  function quit() {
    if (quitArmed) return onQuit();
    quitArmed = true;
    clearTimeout(quitTimer);
    quitTimer = setTimeout(() => (quitArmed = false), 3000);
  }

  onMount(() => {
    if (!wrap || !strip) return;
    lenis = new Lenis({
      wrapper: wrap,
      content: strip,
      orientation: "horizontal",
      gestureOrientation: "horizontal",
      smoothWheel: false,
      syncTouch: false,
      autoRaf: false,
      // The strip only moves when a tab is chosen, never from wheel or touch.
      // (lenis.stop() can't be used: its overflow: clip blocks scrollTo.)
      prevent: () => true,
    });
    go(active, true);
    // Other tabs are built on their first visit (one short frame, once).
    // Pre-building them all made the first open stall while every glass
    // control prepared its effects at the same time.
    return () => {
      ticking(false);
      lenis?.destroy();
      lenis = null;
    };
  });

  // Lenis is driven by GSAP's clock only while the strip is gliding. A
  // permanent listener kept a 144 Hz frame loop running even when idle.
  const raf = (time: number) => lenis?.raf(time * 1000);
  let attached = false;
  function ticking(on: boolean) {
    if (on && !attached) gsap.ticker.add(raf);
    if (!on && attached) gsap.ticker.remove(raf);
    attached = on;
  }

  function go(id: string, immediate = false) {
    const target = strip?.querySelector<HTMLElement>(`[data-pane="${id}"]`);
    if (!lenis || !target) return;
    const jump = immediate || reduced();
    if (!jump) ticking(true);
    lenis.scrollTo(target, {
      immediate: jump,
      duration: dur("expand"),
      easing: gsap.parseEase(ease()) as (t: number) => number,
      onComplete: () => ticking(false),
    });
    if (jump && !immediate) enter(target);
  }

  let last: string | null = null;
  $effect(() => {
    const id = active;
    if (last === null) {
      last = id; // first position is set without motion in onMount
      return;
    }
    if (id !== last) {
      last = id;
      go(id);
    }
  });
</script>

<div class="panel" role="region" aria-label="Atoll">
  <header>
    <div class="plate">
      <Pip mood={look.mood} {tone} size={24} pokeable />
      <span class="name">Atoll</span>
      {#if compactTabs && activeLabel}
        <span class="crumb" aria-hidden="true">/</span>
        <span class="tabname">{activeLabel}</span>
      {/if}
      {#if status !== "idle"}
        <span class="status-word {status}"><Icon name={look.icon} size={16} />{look.word}</span>
      {/if}
    </div>
    <span class="flex"></span>
    <GButton icon="settings" ariaLabel="Settings" title="Settings" onclick={onSettings} />
    <GButton icon="chevronUp" ariaLabel="Collapse" title="Collapse" onclick={onCollapse} />
    {#if quitArmed}
      <GButton icon="power" iconSize={16} label="Quit?" destructive ariaLabel="Click again to quit Atoll" onclick={quit} />
    {:else}
      <GButton icon="power" ariaLabel="Quit Atoll" title="Quit Atoll" onclick={quit} />
    {/if}
  </header>

  <Tabs {tabs} {active} {onselect} bind:compact={compactTabs} />

  <div class="viewport" bind:this={wrap}>
    <div class="strip" bind:this={strip}>
      {#each tabs as t (t.id)}
        <section class="pane" data-pane={t.id} aria-label={t.label} inert={t.id !== active} data-lenis-prevent>
          {#if visited.includes(t.id)}{@render pane(t.id)}{/if}
        </section>
      {/each}
    </div>
  </div>
</div>

<style>
  .panel {
    box-sizing: border-box;
    height: 100%;
    padding: var(--sp-16);
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
  }
  header { display: flex; align-items: center; gap: var(--sp-8); height: var(--size-control); flex: none; }
  .plate {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    height: var(--size-control);
    padding: 0 var(--sp-16) 0 var(--sp-8);
    border-radius: var(--r-pill);
    background: var(--c-surface-plate);
  }
  .name { font-size: var(--fs-body); font-weight: var(--fw-strong); }
  .crumb { color: var(--c-muted); font-size: var(--fs-body); }
  .tabname { font-size: var(--fs-body); color: var(--c-muted); white-space: nowrap; }
  .flex { flex: 1; }
  .viewport {
    flex: 1;
    min-height: 0;
    margin-top: var(--sp-8);
    overflow: hidden;
  }
  .strip { display: flex; height: 100%; }
  /* Fixed to the final panel width, so a tab's position is right even while
     the island is still growing open. */
  .pane {
    flex: none;
    /* Only the tab on screen is drawn; the others draw as they slide in. */
    content-visibility: auto;
    contain-intrinsic-size: auto calc(var(--panel-w) - 2 * var(--sp-16)) auto 300px;
    width: calc(var(--panel-w) - 2 * var(--sp-16));
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
