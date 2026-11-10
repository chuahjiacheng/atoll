<!--
  The island's glass body. Its outline morphs between a floating pill, a peek
  and the expanded panel (GSAP, motion tokens), with the status glow around it.

  Smoothness: the pill, peek and panel are three fixed-size layers stacked in
  the shell. Morphing only resizes the (otherwise empty) shell and cross-fades
  the layers, so nothing inside is laid out again on any frame, and the panel
  stays mounted between opens.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Glass } from "glisse";
  import { dur, ease, gsap, reduced } from "../motion";
  import { px } from "../theme";
  import { react } from "../glass/bridge";

  export type ShellMode = "pill" | "peek" | "panel";
  export type PillWidth = "idle" | "status" | "timer";
  export type Signal = "busy" | "done" | "error" | "accent" | null;

  let {
    mode,
    pillWidth = "idle",
    signal = null,
    progress = null,
    sizeKey = "",
    pill,
    peek,
    panel,
    el = $bindable(),
  }: {
    mode: ShellMode;
    pillWidth?: PillWidth;
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

  function target(m: ShellMode) {
    const roundness = px("--roundness");
    if (m === "panel") return { width: px("--panel-w"), height: px("--panel-h"), radius: px("--r-panel") * roundness };
    if (m === "peek") return { width: px("--size-peek-w"), height: layers.peek?.offsetHeight ?? 0, radius: px("--r-peek") * roundness };
    const h = px("--pill-h");
    const w = pillWidth === "status" ? px("--status-w") : pillWidth === "timer" ? px("--timer-w") : px("--idle-w");
    return { width: w, height: h, radius: (h / 2) * roundness };
  }

  /** The glass only re-fits once per morph: it takes the larger size during the move. */
  function fitGlass(width: number, height: number, radius: number) {
    if (glassBox) gsap.set(glassBox, { width, height, borderRadius: radius });
  }

  let first = true;
  let shown: ShellMode = "pill";
  $effect(() => {
    const m = mode;
    void pillWidth;
    void sizeKey;
    if (!el) return;
    tick().then(() => {
      if (!el) return;
      const from = { width: el.offsetWidth, height: el.offsetHeight };
      const to = target(m);
      const prev = shown;
      shown = m;
      if (first) {
        gsap.set(el, { width: to.width, height: to.height, borderRadius: to.radius });
        fitGlass(to.width, to.height, to.radius);
        for (const k of ["pill", "peek", "panel"] as ShellMode[]) if (layers[k]) gsap.set(layers[k]!, { autoAlpha: k === m ? 1 : 0 });
        first = false;
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
      fitGlass(Math.max(from.width, to.width), Math.max(from.height, to.height), to.radius);
      gsap.to(el, {
        width: to.width,
        height: to.height,
        borderRadius: to.radius,
        duration: d,
        ease: ease(),
        overwrite: "auto",
        // Measured again at the end, not reused from the start: a peek's
        // buttons render a frame late, so it ends taller than it began.
        onComplete: () => {
          const end = target(shown);
          fitGlass(end.width, Math.max(end.height, el?.offsetHeight ?? 0), end.radius);
        },
      });
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
    const t = target(mode);
    requestAnimationFrame(() => {
      fitGlass(px("--panel-w"), px("--panel-h"), px("--r-panel") * px("--roundness"));
      setTimeout(() => {
        if (shown === "pill") fitGlass(t.width, t.height, t.radius);
      }, 250);
    });
  });

  // Peeks grow with their content (e.g. the typed-reply field opening).
  $effect(() => {
    const layer = layers.peek;
    if (mode !== "peek" || !layer || !el) return;
    const observer = new ResizeObserver(() => {
      if (mode !== "peek" || !el || !layer) return;
      const h = layer.offsetHeight;
      fitGlass(px("--size-peek-w"), Math.max(h, el.offsetHeight), px("--r-peek") * px("--roundness"));
      gsap.to(el, { height: h, duration: dur("fast"), ease: ease(), overwrite: "auto" });
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

<div class="shell {mode}" class:text={mode !== "pill" || pillWidth !== "idle"} data-signal={signal ?? lastSignal} bind:this={el}>
  <!-- glisse Liquid Glass body (rim light), tinted with the hull. -->
  <span class="glass-box" bind:this={glassBox}>
    <span class="glass-host" use:react={{ component: Glass, props: { className: "shell-glass" } }}></span>
  </span>

  <div class="layer pill-layer" style="width: var(--{pillWidth}-w)" bind:this={layers.pill} inert={mode !== "pill"}>
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

  /* Fixed-size layers, centred at the top: the shell clips them while it morphs. */
  .layer {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    margin: 0 auto;
    box-sizing: border-box;
    z-index: 1;
  }
  .pill-layer { height: var(--pill-h); }
  .peek-layer { width: var(--size-peek-w); }
  .panel-layer { width: var(--panel-w); height: var(--panel-h); }
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
