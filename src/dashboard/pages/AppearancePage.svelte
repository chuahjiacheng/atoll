<!--
  Appearance: the sticky live preview, then size, shape, colours, theme and
  motion. Every change reaches the real island at once too.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { ScrollTrigger } from "gsap/ScrollTrigger";
  import { gsap } from "../../lib/motion";
  import { px, resolveTheme, watchSystem, type ColorKey } from "../../lib/theme";
  import type { ColorChoice, Settings } from "../../lib/settings";
  import { change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SliderRow from "../ui/SliderRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import ColorRow from "../ui/ColorRow.svelte";
  import Preview from "../ui/Preview.svelte";

  gsap.registerPlugin(ScrollTrigger);

  let { s, scroller }: { s: Settings; scroller: HTMLElement } = $props();

  const a = $derived(s.appearance);
  let systemTick = $state(0);
  const theme = $derived((systemTick, resolveTheme(a)));
  const osReduce = $derived((systemTick, window.matchMedia("(prefers-reduced-motion: reduce)").matches));
  let picker = $state<ColorKey | "">("");

  const PRESETS: Record<string, [string, string]> = {
    compact: ["--size-compact-w", "--size-compact-h"],
    standard: ["--size-status-w", "--size-pill-h"],
    large: ["--size-large-w", "--size-large-h"],
  };
  const presetHelp = $derived(
    Object.entries(PRESETS)
      .map(([id, [w, h]]) => `${id[0].toUpperCase()}${id.slice(1)} ${px(w)} × ${px(h)}`)
      .join(", ") + ".",
  );
  function pickPreset(id: string) {
    const [w, h] = PRESETS[id];
    change("appearance.preset", id);
    change("appearance.collapsedWidth", px(w));
    change("appearance.collapsedHeight", px(h));
  }
  const custom = (path: string) => (v: number) => {
    change(path, v);
    change("appearance.preset", "custom");
  };
  const color = (key: ColorKey) => (c: ColorChoice) => change(`appearance.${key}`, c);
  const expandMs = $derived(Math.round((px("--dur-expand") * 100) / a.animationSpeed));

  // The preview card lifts off the page once it sticks (ScrollTrigger on the settings scroller).
  // Pages open scrolled to the top, so the card's resting place is measured there.
  let sticky = $state<HTMLElement>();
  let stuck = $state(false);
  onMount(() => {
    const un = watchSystem(() => systemTick++);
    const st = sticky
      ? ScrollTrigger.create({ trigger: sticky, scroller, start: "top top", end: "max", onToggle: (self) => (stuck = self.isActive) })
      : null;
    return () => {
      un();
      st?.kill();
    };
  });
</script>

<div class="sticky" class:stuck bind:this={sticky}>
  <Preview {a} tone={s.mascot.tone} />
</div>

<Section title="Island size">
  <SegRow
    label="Preset"
    help={presetHelp}
    items={[
      { id: "compact", label: "Compact" },
      { id: "standard", label: "Standard" },
      { id: "large", label: "Large" },
    ]}
    value={a.preset}
    onchange={pickPreset}
  />
  <SliderRow
    label="Scale"
    help="Makes the whole island, its text and its panel smaller or larger."
    value={a.scale}
    min={25}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    onchange={(v) => change("appearance.scale", v)}
  />
  <SliderRow
    label="Width"
    help="Width with a status showing. Idle is one third of this."
    value={a.collapsedWidth}
    min={160}
    max={560}
    step={8}
    unit="px"
    unitWord="pixels"
    onchange={custom("appearance.collapsedWidth")}
  />
  <SliderRow
    label="Height"
    help="Height of the collapsed island."
    value={a.collapsedHeight}
    min={28}
    max={56}
    step={2}
    unit="px"
    unitWord="pixels"
    onchange={custom("appearance.collapsedHeight")}
  />
</Section>

<Section title="Expanded size">
  <SliderRow
    label="Width"
    help="Width of the expanded panel."
    value={a.expandedWidth}
    min={480}
    max={960}
    step={8}
    unit="px"
    unitWord="pixels"
    onchange={(v) => change("appearance.expandedWidth", v)}
  />
  <SliderRow
    label="Height"
    help="Height of the expanded panel."
    value={a.expandedHeight}
    min={280}
    max={640}
    step={8}
    unit="px"
    unitWord="pixels"
    onchange={(v) => change("appearance.expandedHeight", v)}
  />
</Section>

<Section title="Shape and surface">
  <SliderRow
    label="Corner roundness"
    help="100% is a full pill."
    value={a.roundness}
    min={0}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    onchange={(v) => change("appearance.roundness", v)}
  />
  <SliderRow
    label="Opacity"
    help="Hovering raises it to at least 92%. Text always sits on at least 85%."
    value={a.opacity}
    min={50}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    onchange={(v) => change("appearance.opacity", v)}
  />
  <SegRow
    label="Glass"
    help={a.glass === "liquid"
      ? "Liquid bends and blurs the desktop behind the island. While it's on, the island doesn't show in screenshots or recordings."
      : "Frosted uses Windows' own blur: one fixed strength, no bending."}
    items={[
      { id: "liquid", label: "Liquid" },
      { id: "frosted", label: "Frosted" },
    ]}
    value={a.glass}
    onchange={(v) => change("appearance.glass", v)}
  />
  <SliderRow
    label="Blur"
    help={a.glass === "liquid"
      ? "How much the desktop behind the glass is blurred. 0 is clear glass."
      : "Turns Windows' blur on or off (it has one strength). 0 turns it off."}
    value={a.blur}
    min={0}
    max={48}
    step={2}
    unit="px"
    unitWord="pixels"
    onchange={(v) => change("appearance.blur", v)}
  />
  <SliderRow
    label="Distortion"
    help={a.glass === "liquid" ? "How strongly the rim bends what lies behind and around it, like a thick lens." : "Liquid glass only."}
    value={a.distortion}
    min={0}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    disabled={a.glass !== "liquid"}
    onchange={(v) => change("appearance.distortion", v)}
  />
  <SliderRow
    label="Liquid glass"
    help={a.glass === "liquid" ? "Light on the rim, a soft inner glow, a hint of colour split and richer colour." : "Liquid glass only."}
    value={a.liquid}
    min={0}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    disabled={a.glass !== "liquid"}
    onchange={(v) => change("appearance.liquid", v)}
  />
</Section>

<Section title="Colours">
  {#each [["accent", "Accent", "Highlights, progress and the selected tab."], ["busy", "Busy", "Glow, icon and word while an agent works."], ["done", "Done", "Glow, icon and word when a task finishes."], ["error", "Error", "Glow, icon and word when something fails."]] as [key, label, help] (key)}
    <ColorRow
      key={key as ColorKey}
      {label}
      {help}
      choice={a[key as ColorKey]}
      {theme}
      open={picker === key}
      ontoggle={() => (picker = picker === key ? "" : (key as ColorKey))}
      onchange={color(key as ColorKey)}
    />
  {/each}
</Section>

<Section title="Theme">
  <SegRow
    label="Theme"
    help="Applies to the island and to this window."
    items={[
      { id: "dark", label: "Dark" },
      { id: "light", label: "Light" },
      { id: "follow", label: "Follow Windows" },
    ]}
    value={a.theme}
    wide
    onchange={(v) => change("appearance.theme", v)}
  />
</Section>

<Section title="Motion">
  <SliderRow
    label="Animation speed"
    help={a.reduceMotion || osReduce ? "Turned off while Reduce motion is on." : `At this speed, expanding takes ${expandMs} ms.`}
    value={a.animationSpeed}
    min={50}
    max={200}
    step={25}
    unit="%"
    unitWord="percent"
    disabled={a.reduceMotion || osReduce}
    onchange={(v) => change("appearance.animationSpeed", v)}
  />
  <SwitchRow
    label="Reduce motion"
    help="Replaces movement with short fades."
    checked={a.reduceMotion}
    onchange={(v) => change("appearance.reduceMotion", v)}
    note={osReduce && !a.reduceMotion ? "Windows is set to reduce motion, so Atoll already does." : ""}
  />
</Section>

<style>
  /* design: sticky, bleeding to the page edges, on the window colour */
  .sticky {
    position: sticky;
    top: 0;
    z-index: 2;
    margin: calc(-1 * var(--sp-8)) calc(-1 * var(--sp-32)) 0;
    padding: var(--sp-8) var(--sp-32);
    background: var(--c-win);
    transition: box-shadow var(--dur-fast) var(--ease-settle);
  }
  .sticky.stuck {
    box-shadow: 0 1px 0 var(--c-line);
  }
</style>
