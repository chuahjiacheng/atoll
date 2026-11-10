<!-- Progress ring (usage, focus timer). Fill animates with GSAP; "dashed" = not available. -->
<script lang="ts">
  import { dur, ease, gsap } from "../motion";

  let {
    value,
    size = 56,
    stroke = 6,
    label,
    dashed = false,
    tone = "accent",
    text = "",
  }: {
    /** 0..1 */
    value: number;
    size?: number;
    stroke?: number;
    label: string;
    dashed?: boolean;
    tone?: "accent" | "busy" | "done" | "error";
    /** Centre text (e.g. "62%"). */
    text?: string;
  } = $props();

  const r = $derived((size - stroke) / 2);
  const c = $derived(2 * Math.PI * r);
  let arc = $state<SVGCircleElement>();

  $effect(() => {
    if (!arc) return;
    const v = Math.min(1, Math.max(0, value));
    gsap.to(arc, { strokeDashoffset: c * (1 - v), duration: dur("slow"), ease: ease(), overwrite: "auto" });
  });
</script>

<svg width={size} height={size} viewBox="0 0 {size} {size}" role="img" aria-label={label} class="ring {tone}">
  <circle cx={size / 2} cy={size / 2} {r} fill="none" class="track" stroke-width={stroke} stroke-dasharray={dashed ? "6 6" : "none"} />
  {#if !dashed}
    <circle
      bind:this={arc}
      cx={size / 2}
      cy={size / 2}
      {r}
      fill="none"
      class="fill"
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={c}
      stroke-dashoffset={c}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  {/if}
  {#if text}<text x="50%" y="50%" dy="0.35em" text-anchor="middle" class="text tabular">{text}</text>{/if}
</svg>

<style>
  .ring { flex: none; display: block; }
  .track { stroke: var(--c-track); }
  .accent .fill { stroke: var(--c-accent); }
  .busy .fill { stroke: var(--c-busy); }
  .done .fill { stroke: var(--c-done); }
  .error .fill { stroke: var(--c-error); }
  .text { fill: var(--c-ink); font-size: var(--fs-title); font-weight: var(--fw-strong); font-family: var(--font); }
</style>
