<!--
  Island lab (dev only, island-lab.html). The real Shell and Pill with made-up
  content, buttons for every state, and "Run checks", which measures:
  text measuring against real layout, retargeting under stress, a full
  pill / peek / panel round trip, reduced motion, and that nothing loops at rest.
  Results land on the page and in window.__lab.
-->
<script lang="ts">
  import { mount, onMount, tick, unmount, untrack } from "svelte";
  import Shell from "../Shell.svelte";
  import Pill from "../Pill.svelte";
  import DevOverlay from "../DevOverlay.svelte";
  import { applyAppearance, px } from "../../theme";
  import type { Appearance } from "../../settings";
  import { focusContract, idleContract, lyricsContract, musicContract, statusContract, withLimits } from "../contracts";
  import { measurer } from "../measure";
  import { exposeMetrics, metrics, type FrameStats } from "../metrics";
  import { resolveWidth, settleWidth, type IslandStateId, type TextStyle, type WidthContract } from "../width";

  const choice = (preset: string) => ({ preset, custom: "", customOn: false });
  let appearance = $state<Appearance>({
    preset: "standard",
    scale: 100,
    positionX: 50,
    positionY: 8,
    collapsedWidth: 384,
    collapsedHeight: 40,
    expandedWidth: 720,
    expandedHeight: 440,
    roundness: 100,
    opacity: 80,
    blur: 24,
    glass: "frosted",
    distortion: 60,
    liquid: 60,
    theme: "dark",
    accent: choice("peri"),
    busy: choice("amber"),
    done: choice("green"),
    error: choice("red"),
    animationSpeed: 100,
    reduceMotion: false,
    monitor: "primary",
  });

  type PillProps = Record<string, unknown> & { kind: "idle" | "status" | "music" | "timer" };
  interface Scenario {
    name: string;
    id: IslandStateId;
    contract: WidthContract;
    props: PillProps;
  }
  const status = (name: string, id: IslandStateId, word: string, text: string, meta = "", tint = "busy", icon = "busy"): Scenario => ({
    name,
    id,
    contract: statusContract(id, { word, text, meta }),
    props: { kind: "status", word, text, meta, tint, icon, mood: id === "busy" ? "focused" : "idle" },
  });
  const music = (name: string, title: string, artist: string): Scenario => ({
    name,
    id: "music",
    contract: musicContract({ title, artist }),
    props: { kind: "music", title, artist, playing: true },
  });
  const timer = (name: string, time: string, word: string, note = "", strong = false): Scenario => ({
    name,
    id: "focus",
    contract: focusContract({ time, word, note, strong }),
    props: { kind: "timer", time, word, text: note, ring: 0.4, tint: strong ? "accent" : "busy" },
  });

  const SCENARIOS: Scenario[] = [
    { name: "Idle", id: "idle", contract: idleContract(), props: { kind: "idle" } },
    status("Busy, short", "busy", "Busy", "Thinking…", "12s"),
    status("Busy, step", "busy", "Busy", "3/7 · Rework Shell, Pill and App", "4m 05s"),
    status("Busy, very long", "busy", "Busy", "2/9 · Read every source file in the repository and summarise what each one is for", "1h 02m"),
    status("Done", "done", "Done", "Task finished", "", "done", "check"),
    status("Error", "error", "Error", "Usage limit reached", "", "error", "alert"),
    status("Needs you", "attention", "Needs you", "Allow Bash: npm run build?", "", "accent", "info"),
    status("Event", "event", "In 5 min", "Design review with Mei", "", "accent", "calendar"),
    music("Music", "Clair de Lune", "Claude Debussy"),
    music("Music, long", "The Girl from Ipanema (Remastered 2014 Stereo Version)", "Stan Getz, João Gilberto, Astrud Gilberto"),
    { name: "Lyrics", id: "lyrics", contract: lyricsContract(), props: { kind: "music", title: "Song", artist: "Singer", lyric: "And the line being sung goes here", lyricAt: 1, roll: true, playing: true } },
    timer("Timer", "24:13", "Focus"),
    timer("Timer, note", "24:13", "Focus", "Write the handoff notes"),
    timer("Timer, nudge", "08:02", "Focus", "Discord · back to focus?", true),
    status("Chinese", "busy", "Busy", "正在重构岛屿的宽度与动画", "8s"),
    status("Japanese", "busy", "Busy", "アイランドの幅を調整しています", "8s"),
    status("Korean", "busy", "Busy", "섬 너비를 조정하는 중입니다", "8s"),
    status("Malay", "busy", "Busy", "Sedang mengemas kini lebar pulau", "8s"),
    status("Arabic", "busy", "Busy", "جارٍ ضبط عرض الجزيرة الآن", "8s"),
    status("Hebrew", "busy", "Busy", "מתאים את רוחב האי כעת", "8s"),
    music("Music, mixed", "夜に駆ける (Racing into the Night)", "YOASOBI"),
  ];

  let index = $state(0);
  let mode = $state<"pill" | "peek" | "panel">("pill");
  let tallPeek = $state(false);
  let measured = $state(0);
  let shellEl = $state<HTMLDivElement>();
  let results = $state<Record<string, unknown> | null>(null);
  let running = $state(false);

  const scenario = $derived(SCENARIOS[index]);
  const contract = $derived.by(() => {
    void measured;
    return withLimits(scenario.contract, measurer);
  });
  const fit = $derived.by(() => {
    void measured;
    return resolveWidth(contract, measurer, appearance.collapsedWidth);
  });
  let pillW = $state(64);
  let widthFor: IslandStateId | null = null;
  $effect(() => {
    const next = fit.width;
    const id = contract.id;
    untrack(() => {
      pillW = id === widthFor ? settleWidth(pillW, next, px("--width-deadband")) : next;
      widthFor = id;
    });
  });
  $effect(() => {
    metrics.setState(mode === "pill" ? scenario.id : mode === "peek" ? "peek:finished" : "panel", `lab: ${scenario.name}`);
    metrics.setLimits(contract.min, Math.max(contract.min, Math.min(contract.max, appearance.collapsedWidth)));
  });
  $effect(() => {
    applyAppearance(appearance);
    untrack(() => measurer.clear());
  });

  onMount(() => {
    exposeMetrics();
    const stop = measurer.onChange(() => measured++);
    measurer.start();
    return stop;
  });

  // ---------- checks ----------
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const frame = () => new Promise<number>((r) => requestAnimationFrame(r));
  /** Wait until the island is at rest AND at the size its current mode asks for. */
  async function settle(limit = 3000) {
    const end = performance.now() + limit;
    await tick();
    await wait(20);
    const there = () => {
      if (metrics.snapshot().loopRunning || !shellEl) return false;
      const r = shellEl.getBoundingClientRect();
      const want = mode === "panel" ? appearance.expandedWidth : mode === "peek" ? px("--size-peek-w") : pillW;
      const tall = mode === "panel" ? appearance.expandedHeight : mode === "peek" ? (tallPeek ? 200 : 120) : appearance.collapsedHeight;
      return Math.abs(r.width - want) < 0.6 && Math.abs(r.height - tall) < 0.6;
    };
    while (!there() && performance.now() < end) await wait(8);
    await wait(60);
  }

  /** Each scenario's contract against the pill's real layout at its natural width. */
  async function checkMeasure() {
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:0;top:0;visibility:hidden;width:max-content;pointer-events:none";
    document.body.append(host);
    const rows = [];
    for (const s of SCENARIOS) {
      if (s.id === "lyrics") continue;
      const app = mount(Pill, { target: host, props: { ...s.props, tone: "peri", label: s.name, onclick: () => {} } as never });
      await tick();
      // Computed width, not the bounding box: the arriving pill is still scaled by its entrance.
      const natural = parseFloat(getComputedStyle(host.querySelector("button")!).width);
      const claimed = s.contract.measure(measurer);
      rows.push({ name: s.name, natural: +natural.toFixed(2), contract: claimed, slack: +(claimed - natural).toFixed(2) });
      await unmount(app);
    }
    host.remove();
    // The two ways of measuring text, for every string the scenarios use.
    const strings: [string, TextStyle][] = SCENARIOS.flatMap((s) => {
      const p = s.props as Record<string, string>;
      return [[p.text, "small"], [p.word, "labelStrong"], [p.title, "smallStrong"], [p.artist, "small"], [p.meta, "tabular"], [p.time, "tabularStrong"]] as [string, TextStyle][];
    }).filter(([v]) => !!v);
    const backends = strings.map(([v, style]) => {
      const canvas = measurer.measure(v, style, "canvas");
      const element = measurer.measure(v, style, "element");
      return { text: v, style, uses: measurer.backendFor(v, style), canvas: +canvas.toFixed(2), element: +element.toFixed(2), diff: +(canvas - element).toFixed(2) };
    });
    const used = backends.filter((b) => b.uses === "canvas");
    return {
      pass: rows.every((r) => r.slack >= 0 && r.slack <= 2),
      worstSlackPx: Math.max(...rows.map((r) => Math.abs(r.slack))),
      rows,
      canvasWorstDiffPx: Math.max(0, ...used.map((b) => Math.abs(b.diff))),
      canvasWouldBeWrongFor: backends.filter((b) => b.uses === "element" && Math.abs(b.diff) > 1).map((b) => ({ text: b.text, style: b.style, diff: b.diff })),
    };
  }

  /** No state's text is cut off at the width the island gives it (unless it hit the ceiling). */
  async function checkNoCutOff() {
    const cut = [];
    for (let i = 0; i < SCENARIOS.length; i++) {
      index = i;
      mode = "pill";
      await settle();
      const line = shellEl?.querySelector<HTMLElement>(".pill-layer .line");
      const overflowing = fit.overflowing;
      if (line && !overflowing && line.scrollWidth > line.clientWidth + 0.5) cut.push({ name: SCENARIOS[i].name, scroll: line.scrollWidth, client: line.clientWidth });
      const shell = shellEl!.getBoundingClientRect();
      if (Math.abs(shell.width - pillW) > 0.6) cut.push({ name: SCENARIOS[i].name, shell: shell.width, pillW });
    }
    return { pass: cut.length === 0, cut };
  }

  /** Change target every few frames for a while; every frame's step must match the spring's speed. */
  async function checkRetarget() {
    mode = "pill";
    index = 0;
    await settle();
    const frames: { w: number; v: number; d: number }[] = [];
    const stop = metrics.onEveryFrame(() => {
      const s = metrics.snapshot();
      frames.push({ w: s.width.current, v: Math.abs(s.width.velocity) / 1000, d: 0 });
    });
    let last = 0;
    let watching = true;
    const stamps: number[] = [];
    const slow: string[] = [];
    const watch = (t: number) => {
      if (last) stamps.push(t - last);
      if (last && t - last > 10.5) slow.push(`${(t - last).toFixed(1)} ms after ${SCENARIOS[index].name}`);
      last = t;
      if (watching) requestAnimationFrame(watch);
    };
    requestAnimationFrame(watch);
    metrics.begin("retarget");
    let retargets = 0;
    const end = performance.now() + 4000;
    let seed = 7;
    const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
    while (performance.now() < end) {
      index = Math.floor(rand() * SCENARIOS.length);
      retargets++;
      await wait(40 + rand() * 90);
    }
    await settle();
    const stats = metrics.end("retarget");
    watching = false;
    stop();
    // A jump is a step far bigger than speed × frame time allows.
    const topSpeed = Math.max(0, ...frames.map((f) => f.v));
    let worstExcess = 0;
    for (let i = 1; i < frames.length; i++) {
      const step = Math.abs(frames[i].w - frames[i - 1].w);
      const allowed = Math.max(frames[i].v, frames[i - 1].v) * stats.max * 1.25 + 0.5;
      worstExcess = Math.max(worstExcess, step - allowed);
    }
    return {
      pass: worstExcess <= 0 && stats.dropped === 0,
      retargets,
      frames: stats,
      topSpeedPxPerSec: Math.round(topSpeed * 1000),
      largestStepPx: stats.longestJumpPx,
      largestStepAllowedPx: +(topSpeed * stats.max).toFixed(1),
      worstExcessPx: +worstExcess.toFixed(2),
      slowFrames: slow,
    };
  }

  /** Pill → panel → pill → peek → taller peek → pill, each timed. */
  async function checkMorph() {
    index = 1;
    mode = "pill";
    await settle();
    const legs: Record<string, FrameStats & { startDelayMs: number }> = {};
    const leg = async (name: string, change: () => void) => {
      metrics.begin(name);
      const t0 = performance.now();
      let started = 0;
      const stop = metrics.onEveryFrame(() => (started ||= performance.now()));
      change();
      await settle();
      stop();
      // startDelayMs: from the request to the first frame of movement.
      legs[name] = { ...metrics.end(name), startDelayMs: Math.round(started - t0) };
    };
    await leg("pill to panel", () => (mode = "panel"));
    await leg("panel to pill", () => (mode = "pill"));
    await leg("pill to peek", () => (mode = "peek"));
    await leg("peek grows", () => (tallPeek = true));
    await leg("peek to pill", () => ((mode = "pill"), (tallPeek = false)));
    await leg("idle to long", () => (index = 3));
    await leg("long to idle", () => (index = 0));
    const all = Object.values(legs);
    return { pass: all.every((l) => l.dropped === 0), dropped: all.reduce((n, l) => n + l.dropped, 0), legs };
  }

  /** At rest, nothing asks for animation frames. */
  async function checkIdle() {
    index = 0;
    mode = "pill";
    await settle();
    await wait(2500); // GSAP keeps its own ticker for about 120 frames after its last call, then sleeps
    const counter = (window as unknown as { __rafCounter: { requests: number } }).__rafCounter;
    const before = counter.requests;
    await wait(2000);
    const requests = counter.requests - before;
    return { pass: requests === 0 && !metrics.snapshot().loopRunning, frameRequestsIn2s: requests, loopRunning: metrics.snapshot().loopRunning };
  }

  /** Reduce motion: the width is simply there on the next frame. */
  async function checkReduced() {
    index = 0;
    mode = "pill";
    await settle();
    appearance.reduceMotion = true;
    await tick();
    await wait(50);
    index = 3;
    await tick();
    await frame();
    await frame();
    const w = shellEl!.getBoundingClientRect().width;
    const moving = metrics.snapshot().loopRunning;
    const result = { pass: Math.abs(w - pillW) < 0.6 && !moving, widthAfterTwoFrames: w, target: pillW, loopRunning: moving };
    appearance.reduceMotion = false;
    await tick();
    index = 0;
    await settle();
    return result;
  }

  async function runChecks() {
    running = true;
    results = null;
    await document.fonts.ready;
    const out: Record<string, unknown> = { visible: document.visibilityState, devicePixelRatio };
    out.measure = await checkMeasure();
    out.noCutOff = await checkNoCutOff();
    out.retarget = await checkRetarget();
    out.morph = await checkMorph();
    out.reducedMotion = await checkReduced();
    out.idle = await checkIdle();
    out.pass = ["measure", "noCutOff", "retarget", "morph", "reducedMotion", "idle"].every((k) => (out[k] as { pass: boolean }).pass);
    results = out;
    (window as unknown as { __lab: unknown }).__lab = out;
    running = false;
    return out;
  }
  (window as unknown as { __runLab: typeof runChecks }).__runLab = runChecks;
</script>

<main>
  <div class="stage">
    <Shell {mode} {pillW} quiet={scenario.id === "idle"} bind:el={shellEl} peek={mode === "peek" ? peek : undefined} {panel}>
      {#snippet pill()}
        {#key `${scenario.id}`}
          <Pill {...scenario.props as never} tone="peri" width={pillW} label={scenario.name} onclick={() => (mode = "panel")} />
        {/key}
      {/snippet}
    </Shell>
    <div class="dev"><DevOverlay /></div>
  </div>

  {#snippet peek()}
    <div class="fake-peek" style:height={tallPeek ? "200px" : "120px"}>A peek ({tallPeek ? 200 : 120} px tall)</div>
  {/snippet}
  {#snippet panel()}
    <div class="fake-panel">The expanded panel</div>
  {/snippet}

  <section class="controls">
    <div class="row">
      {#each SCENARIOS as s, i}
        <button class:on={i === index && mode === "pill"} onclick={() => ((index = i), (mode = "pill"))}>{s.name}</button>
      {/each}
    </div>
    <div class="row">
      <button class:on={mode === "peek"} onclick={() => (mode = mode === "peek" ? "pill" : "peek")}>Peek</button>
      <button onclick={() => (tallPeek = !tallPeek)}>Peek height</button>
      <button class:on={mode === "panel"} onclick={() => (mode = mode === "panel" ? "pill" : "panel")}>Panel</button>
      <button class:on={appearance.reduceMotion} onclick={() => (appearance.reduceMotion = !appearance.reduceMotion)}>Reduce motion</button>
      <button onclick={() => (appearance.animationSpeed = appearance.animationSpeed === 100 ? 50 : 100)}>Speed {appearance.animationSpeed} %</button>
      <button onclick={() => (appearance.theme = appearance.theme === "dark" ? "light" : "dark")}>Theme</button>
      <button class="go" disabled={running} onclick={runChecks}>{running ? "Running…" : "Run checks"}</button>
    </div>
    {#if results}
      <pre class:fail={!results.pass}>{JSON.stringify(results, null, 1)}</pre>
    {/if}
  </section>
</main>

<style>
  :global(html, body, #app) {
    overflow: auto !important;
    user-select: text !important;
  }
  main {
    min-height: 100%;
    background:
      radial-gradient(circle at 20% 10%, #3b6fb5 0, transparent 40%),
      radial-gradient(circle at 80% 0%, #b5523b 0, transparent 35%),
      linear-gradient(#20242c, #0f1114);
  }
  :global([data-theme="light"]) main {
    background:
      radial-gradient(circle at 20% 10%, #9cc4f5 0, transparent 40%),
      radial-gradient(circle at 80% 0%, #f5c09c 0, transparent 35%),
      linear-gradient(#e9edf3, #cfd6e0);
  }
  .stage {
    position: relative;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    height: 560px;
    padding-top: 32px;
    box-sizing: border-box;
  }
  .dev {
    position: absolute;
    left: var(--sp-16);
    top: var(--sp-16);
  }
  .fake-peek,
  .fake-panel {
    display: grid;
    place-items: center;
    color: var(--c-ink);
    box-sizing: border-box;
  }
  .fake-panel { width: 100%; height: 100%; }
  .controls {
    padding: var(--sp-16);
    display: grid;
    gap: var(--sp-8);
    color: var(--c-ink);
  }
  .row { display: flex; flex-wrap: wrap; gap: var(--sp-6); }
  .controls button {
    padding: var(--sp-6) var(--sp-12);
    border-radius: var(--r-nav);
    background: rgba(var(--c-hull-rgb), 0.8);
    font-size: var(--fs-label);
  }
  .controls button.on { outline: 2px solid var(--c-accent); }
  .controls button.go { background: var(--c-accent); color: var(--c-accent-ink); font-weight: var(--fw-strong); }
  pre {
    margin: 0;
    padding: var(--sp-12);
    border-radius: var(--r-control);
    background: rgba(var(--c-hull-rgb), 0.92);
    font-size: 11px;
    line-height: 1.35;
    max-height: 480px;
    overflow: auto;
    white-space: pre-wrap;
  }
  pre.fail { outline: 2px solid var(--c-error); }
</style>
