<!--
  Module tabs: glisse's Liquid Glass tab bar (the selection springs between
  tabs, follows a drag and lifts into a lens while held). The width is shared
  out so every tab fits the panel. When the names no longer fit (many modules
  on), the bar turns into an icon-only dock: each tab keeps its name as a
  tooltip and for screen readers, and the panel header names the open tab.
-->
<script lang="ts">
  import { TabBar } from "glisse";
  import { iconLabel, iconOnlyLabel, react } from "../glass/bridge";
  import { px } from "../theme";
  import type { IconName } from "../ui/Icon.svelte";

  export interface TabDef {
    id: string;
    label: string;
    icon: IconName;
  }

  let {
    tabs,
    active,
    onselect,
    compact = $bindable(false),
  }: { tabs: TabDef[]; active: string; onselect: (id: string) => void; compact?: boolean } = $props();

  // Panel width minus its padding and the bar's own inset.
  const room = $derived(px("--panel-w") - 2 * px("--sp-16") - 2 * px("--sp-4"));
  const shared = $derived(Math.floor(room / Math.max(1, tabs.length)));

  // Width a labelled tab needs: icon, gap, the longest name, side padding.
  let measure: CanvasRenderingContext2D | null = null;
  function textWidth(text: string): number {
    measure ??= document.createElement("canvas").getContext("2d");
    if (!measure) return text.length * 8;
    const cs = getComputedStyle(document.documentElement);
    measure.font = `${cs.getPropertyValue("--fw-strong").trim() || 600} ${px("--fs-small")}px ${cs.fontFamily}`;
    return measure.measureText(text).width;
  }
  const needed = $derived(16 + 6 + Math.max(0, ...tabs.map((t) => textWidth(t.label))) + 2 * px("--sp-12"));

  $effect(() => {
    compact = shared < needed;
  });

  const width = $derived(compact ? Math.min(shared, 64) : Math.min(px("--size-tab-max"), shared));
</script>

<div
  class="tabs"
  class:compact
  use:react={{
    component: TabBar,
    props: {
      "aria-label": "Modules",
      items: tabs.map((t) => ({
        value: t.id,
        label: compact ? iconOnlyLabel(t.icon, t.label) : iconLabel(t.icon, t.label),
      })),
      value: active,
      onValueChange: (v: string) => onselect(v),
      style: { "--lg-tab-width": `${width}px` },
    },
  }}
></div>

<style>
  .tabs { display: flex; flex: none; align-self: flex-start; }
  .tabs.compact { align-self: center; }
  .tabs :global(.lg-tab) { font-size: var(--fs-small); font-weight: var(--fw-strong); }
  /* Optically centre icon + label in every tab (glisse trims text to cap height). */
  .tabs :global(.lg-tab-text) { text-box: none; line-height: 1; display: flex; align-items: center; justify-content: center; }
  .tabs :global(.atoll-lg-label) { line-height: 1; }
  .tabs :global(.atoll-sr-only) {
    position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap;
  }
</style>
