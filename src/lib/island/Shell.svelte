<!--
  The island's glass body. Its outline morphs between a floating pill, a peek
  and the expanded panel, with the status glow around it. The pill is as wide
  as its content (docs/contracts/width-contract.md).

  Smoothness: the pill, peek and panel are three layers stacked in the shell,
  each laid out at its final size and centred. Morphing only resizes the
  (otherwise empty) shell on a spring and cross-fades the layers, so nothing
  inside is laid out again on any frame, text never reflows mid-move, and the
  panel stays mounted between opens. The shell grows and shrinks evenly
  around its centre, revealing or covering the content from both sides.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Glass } from "glisse";
  import { dur, ease, gsap, reduced } from "../motion";
  import { px } from "../theme";
  import { react } from "../glass/bridge";
  import { ShapeSpring, type Shape, type SpringFrame } from "./spring";
  import { metrics } from "./metrics";

  export type ShellMode = "pill" | "peek" | "panel";
  export type Signal = "busy" | "done" | "error" | "accent" | null;

  let {
    mode,
    pillW,
    quiet = false,
    signal = null,
    progress = null,
    sizeKey = "",
    pill,
    peek,
    panel,
    el = $bindable(),
  }: {
    mode: ShellMode;
    /** How wide the collapsed pill should be (px), resolved from its width contract. */
    pillW: number;
    /** The pill shows no text (idle): it may be more see-through. */
    quiet?: boolean;
    signal?: Signal;
    /** Changes whenever the island's size settings do, so the outline re-fits
     *  to the new size tokens without waiting for the next open or close. */
    sizeKey?: string;
    /** 0..1: thin progress line along the pill's bottom edge (music). */
    progress?: number | null;
    pill: Snippet;
    peek?: Snippet;
    /** Rendered once, then kept (hidden) so reopening costs nothing. */
    panel?: Snippet;
    el?: HTMLDivElement;
  } = $props();

  let glassBox = $state<HTMLSpanElement>();
  let layers: Record<ShellMode, HTMLDivElement | undefined> = $state({ pill: undefined, peek: undefined, panel: undefined });
  let lastSignal = $state<Exclude<Signal, null>>("busy");

  function target(m: ShellMode): Shape {
    const roundness = px("--roundness");
    if (m === "panel") return { w: px("--panel-w"), h: px("--panel-h"), r: px("--r-panel") * roundness };
    if (m === "peek") return { w: px("--size-peek-w"), h: layers.peek?.offsetHeight ?? 0, r: px("--r-peek") * roundness };
    const h = px("--pill-h");
    return { w: pillW, h, r: (h / 2) * roundness };
  }

  /** Spring stiffness (1/s) for a move, scaled by the user's animation speed. */
  function stiffness(name: "expand" | "collapse" | "grow" | "shrink") {
    return px(`--spring-${name}`) * (px("--speed") || 1);
  }

  /** The glass only re-fits at the start and end of a move: it takes the larger size meanwhile. */
  let glassSize = { w: 0, h: 0 };
  function fitGlass(width: number, height: number, radius: number) {
    if (!glassBox) return;
    gsap.set(glassBox, { width, height, borderRadius: radius });
    glassSize = { w: width, h: height };
  }

  // ---------- the shape ----------
  // Width, height and corner radius ride one spring (spring.ts). A frame loop
  // exists only while the shape is moving; at rest nothing is scheduled.
  let spring: ShapeSpring | null = null;
  function paint(f: SpringFrame) {
    if (!el) return;
    // Corners never exceed half the height, so a pill stays a pill all the way.
    const r = Math.max(0, Math.min(f.pos.r, f.pos.h / 2));
    el.style.width = `${f.pos.w}px`;
    el.style.height = `${f.pos.h}px`;
    el.style.borderRadius = `${r}px`;
    if (f.rest) fitGlass(f.pos.w, f.pos.h, r);
    metrics.onFrame(f, spring?.retargets ?? 0);
  }
  let startFrame = 0;
  onMount(() => () => {
    cancelAnimationFrame(startFrame);
    spring?.stop();
  });

  /**
   * Move the shape. `large` = a peek or the panel: growing the glass to a size
   * that big makes glisse redraw its rim, which can take longer than a frame
   * (measured: 14 ms for a peek, 69 ms for a first panel). That cost is paid
   * BEFORE the shape starts moving (the spring starts two frames later), so it
   * shows as a start a few milliseconds late, never as a hitch mid-move.
   */
  function moveTo(to: Shape, omega: number, large = false) {
    if (!spring) return;
    cancelAnimationFrame(startFrame);
    if (reduced()) {
      // Reduce motion: the shape is simply there; the content cross-fades.
      spring.jump(to);
      return;
    }
    const at = spring.now().pos;
    // Pill-to-pill moves borrow one size the glass has already drawn (the
    // widest pill, warmed at start-up) instead of a new in-between size, so no
    // rim is redrawn mid-move; the exact fit happens once the shape is at rest.
    const w = Math.max(at.w, to.w, large ? 0 : px("--status-w"));
    const h = Math.max(at.h, to.h);
    const glassGrows = w > glassSize.w + 0.5 || h > glassSize.h + 0.5;
    fitGlass(w, h, to.r);
    if (large && glassGrows && !spring.running) {
      // Start once frames are arriving on time again (at least two frames, at
      // most four): the redraw spreads over a frame or three.
      let last = 0;
      let seen = 0;
      const wait = (stamp: number) => {
        const late = last ? stamp - last > metrics.budgetMs * 1.5 : true;
        last = stamp;
        if (++seen >= 5 || (seen >= 3 && !late)) spring?.to(to, omega);
        else startFrame = requestAnimationFrame(wait);
      };
      startFrame = requestAnimationFrame(wait);
    } else {
      spring.to(to, omega);
    }
  }

  let shown: ShellMode = "pill";
  $effect(() => {
    const m = mode;
    void pillW;
    void sizeKey;
    if (!el) return;
    tick().then(() => {
      if (!el) return;
      const to = target(m);
      const prev = shown;
      shown = m;
      document.documentElement.style.setProperty("--island-h", `${to.h}px`);
      if (!spring) {
        spring = new ShapeSpring(to, paint);
        spring.jump(to);
        for (const k of ["pill", "peek", "panel"] as ShellMode[]) if (layers[k]) gsap.set(layers[k]!, { autoAlpha: k === m ? 1 : 0 });
        return;
      }
      // Any layer that is neither leaving nor arriving stays hidden. Its
      // pending fades are cancelled too: a quick open-then-close must not let
      // a delayed fade-in bring the panel back over the pill.
      for (const k of ["pill", "peek", "panel"] as ShellMode[]) {
        if (layers[k] && k !== prev && k !== m) gsap.set(layers[k]!, { autoAlpha: 0, y: 0, overwrite: true });
      }
      const name = m === "panel" ? "expand" : m === "peek" ? "peek" : prev === "pill" ? "slow" : "collapse";
      const d = dur(name);
      const move = m !== "pill" ? "expand" : prev !== "pill" ? "collapse" : to.w >= spring.target.w ? "grow" : "shrink";
      moveTo(to, stiffness(move), m !== "pill");
      if (prev !== m) {
        const out = layers[prev];
        const inn = layers[m];
        // overwrite: true kills every tween on the layer, including one still waiting on its delay.
        if (out) gsap.to(out, { autoAlpha: 0, y: 0, duration: dur("fast") * 0.75, ease: "power1.out", overwrite: true });
        if (inn) {
          gsap.fromTo(
            inn,
            { autoAlpha: 0, y: reduced() || m === "pill" ? 0 : -6 },
            { autoAlpha: 1, y: 0, duration: dur("slow"), delay: m === "pill" ? d * 0.5 : d * 0.3, ease: ease(), overwrite: true },
          );
        }
      }
    });
  });

  // Once the panel exists, let the glass fit the panel size once in the background
  // (and back), so the first real open doesn't pay for it.
  let warmed = false;
  $effect(() => {
    if (warmed || !layers.panel || !glassBox || !el) return;
    warmed = true;
    const quiet = () => shown === "pill" && !spring?.running;
    requestAnimationFrame(() => {
      fitGlass(px("--panel-w"), px("--panel-h"), px("--r-panel") * px("--roundness"));
      setTimeout(() => {
        if (!quiet()) return;
        // The widest pill too: every pill-to-pill move borrows this size.
        const t = target("pill");
        fitGlass(Math.max(t.w, px("--status-w")), t.h, t.r);
        setTimeout(() => quiet() && fitGlass(t.w, t.h, t.r), 250);
      }, 250);
    });
  });

  // Peeks grow with their content (e.g. the typed-reply field opening).
  $effect(() => {
    const layer = layers.peek;
    if (mode !== "peek" || !layer || !el) return;
    const observer = new ResizeObserver(() => {
      if (mode !== "peek" || !el || !layer || shown !== "peek") return;
      const to = target("peek");
      if (spring && Math.abs(to.h - spring.target.h) < 0.5) return;
      document.documentElement.style.setProperty("--island-h", `${to.h}px`);
      moveTo(to, stiffness("grow"), true);
    });
    observer.observe(layer);
    return () => observer.disconnect();
  });

  // Whenever the island comes to rest at a new size, for whatever reason, the
  // glass is fitted to exactly that size. Without this net, a stale fit left
  // the bottom of a peek with no glass at all: see-through.
  // Each change is also sent to Windows, which blurs the desktop under exactly
  // this shape (backdrop.rs); a page's own backdrop-filter can't see the desktop.
  $effect(() => {
    const shell = el;
    if (!shell) return;
    let rest: ReturnType<typeof setTimeout> | undefined;
    const report = () => {
      const r = shell.getBoundingClientRect();
      const radius = parseFloat(getComputedStyle(shell).borderTopLeftRadius) || 0;
      invoke("set_backdrop", { x: r.left, y: r.top, w: r.width, h: r.height, r: radius }).catch(() => {});
    };
    const settle = () => {
      if (spring?.running) return;
      fitGlass(shell.offsetWidth, shell.offsetHeight, parseFloat(getComputedStyle(shell).borderTopLeftRadius) || 0);
      report();
    };
    const observer = new ResizeObserver(() => {
      report();
      clearTimeout(rest);
      // At rest once it has not changed for a short beat.
      rest = setTimeout(settle, dur("fast") * 1000);
    });
    observer.observe(shell);
    report();
    return () => {
      observer.disconnect();
      clearTimeout(rest);
    };
  });

  // Corners can change without the size changing (Corner roundness).
  $effect(() => {
    void sizeKey;
    if (!el) return;
    const shell = el;
    const t = setTimeout(() => {
      const r = shell.getBoundingClientRect();
      invoke("set_backdrop", { x: r.left, y: r.top, w: r.width, h: r.height, r: parseFloat(getComputedStyle(shell).borderTopLeftRadius) || 0 }).catch(() => {});
    }, (dur("slow") + 0.05) * 1000);
    return () => clearTimeout(t);
  });

  // Status glow fades in and out (DESIGN.md: a glow is news).
  $effect(() => {
    if (signal) lastSignal = signal;
    if (!el) return;
    gsap.to(el, { "--glow": signal ? 1 : 0, duration: dur("slow"), ease: ease(), overwrite: "auto" });
  });
</script>

<div class="shell {mode}" class:text={mode !== "pill" || !quiet} data-signal={signal ?? lastSignal} bind:this={el}>
  <!-- glisse Liquid Glass body (rim light), tinted with the hull. -->
  <span class="glass-box" bind:this={glassBox}>
    <span class="glass-host" use:react={{ component: Glass, props: { className: "shell-glass" } }}></span>
  </span>

  <div class="layer pill-layer" style="--w: {pillW}px" bind:this={layers.pill} inert={mode !== "pill"}>
    {@render pill()}
  </div>
  {#if peek}
    <div class="layer peek-layer" bind:this={layers.peek} inert={mode !== "peek"}>
      {@render peek()}
    </div>
  {/if}
  {#if panel}
    <div class="layer panel-layer" bind:this={layers.panel} inert={mode !== "panel"}>
      {@render panel()}
    </div>
  {/if}

  {#if progress !== null && mode === "pill"}
    <span class="progress" aria-hidden="true"><span style="transform: scaleX({Math.min(1, Math.max(0, progress))})"></span></span>
  {/if}
</div>

<style>
  @property --glow {
    syntax: "<number>";
    inherits: false;
    initial-value: 0;
  }
  @property --sig {
    syntax: "<color>";
    inherits: false;
    initial-value: transparent;
  }
  .shell {
    --glow: 0;
    --shell-alpha: var(--hull-alpha);
    position: relative;
    overflow: hidden;
    box-sizing: border-box;
    color: var(--c-ink);
    box-shadow:
      0 0 0 calc(var(--glow-spread) * var(--glow)) var(--sig),
      0 0 0 calc(var(--glow-gap) * var(--glow)) rgba(var(--c-hull-rgb), var(--glow)),
      0 0 calc(var(--glow-blur) * var(--glow)) calc(var(--glow-size) * var(--glow))
        color-mix(in srgb, var(--sig) calc(var(--glow-strength) * var(--glow)), transparent),
      var(--shadow-shell);
    transition: --sig calc(var(--dur-slow) / var(--speed)) var(--ease-settle);
  }
  .shell.text { --shell-alpha: var(--pill-alpha); }
  .shell.panel { --shell-alpha: var(--panel-alpha); }
  .shell.peek { --shell-alpha: 1; }
  .shell[data-signal="busy"] { --sig: var(--c-busy); }
  .shell[data-signal="done"] { --sig: var(--c-done); }
  .shell[data-signal="error"] { --sig: var(--c-error); }
  .shell[data-signal="accent"] { --sig: var(--c-accent); }

  .glass-box {
    position: absolute;
    top: 0;
    left: 50%;
    transform: translateX(-50%);
    overflow: hidden;
    pointer-events: none;
  }
  .glass-host { display: contents; }

  /* Layers at their final size, centred at the top: the shell clips them
     evenly on both sides while it morphs. (Centred with a margin, not a
     transform: GSAP owns the layers' transforms.) */
  .layer {
    position: absolute;
    top: 0;
    left: 50%;
    width: var(--w);
    margin-left: calc(var(--w) / -2);
    box-sizing: border-box;
    z-index: 1;
  }
  /* The pill's content is anchored to the island's left edge, not centred: its
     leading mark (Pip, album art, timer ring) rides that edge as the island
     grows, and the text is uncovered by the right edge. Centred, the leading
     mark hopped sideways and was clipped whenever the content got wider.
     The leaving and arriving contents share one cell while they cross-fade. */
  .pill-layer {
    left: 0;
    margin-left: 0;
    height: var(--pill-h);
    display: grid;
    justify-items: start;
  }
  .pill-layer > :global(*) { grid-area: 1 / 1; }
  .peek-layer { --w: var(--size-peek-w); }
  .panel-layer { --w: var(--panel-w); height: var(--panel-h); }
  /* The panel is built in the background after start-up (and peeks appear
     later too): they start hidden, and only GSAP's fade shows them. */
  .peek-layer,
  .panel-layer {
    visibility: hidden;
    opacity: 0;
  }

  .progress {
    position: absolute;
    left: var(--sp-16);
    right: var(--sp-16);
    bottom: var(--sp-4);
    height: var(--sp-2);
    border-radius: var(--r-pill);
    background: var(--c-track);
    overflow: hidden;
    z-index: 2;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--c-accent);
    transform-origin: left;
  }
</style>
