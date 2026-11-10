<!--
  glisse segmented control. The selected option carries a check (design),
  so it isn't shown by colour alone.

  glisse gives every segment the same width. That width is the size token
  (wide / narrow / default) or, if larger, what the longest label needs with
  the check beside it in the selected weight, so a label is never cut off and
  never crowds the chip's edges.
-->
<script lang="ts">
  import { SegmentedControl } from "glisse";
  import { iconLabel, react } from "../../lib/glass/bridge";
  import { px } from "../../lib/theme";

  let {
    items,
    value,
    label,
    disabled = false,
    wide = false,
    narrow = false,
    fit = false,
    onchange,
  }: {
    items: { id: string; label: string }[];
    value: string;
    label: string;
    disabled?: boolean;
    /** Wider segments for longer labels ("Follow Windows"). */
    wide?: boolean;
    /** Narrower segments for one short word each. */
    narrow?: boolean;
    /** Many options on their own line: when the line is narrower than the
     *  segments want, they share it (the side padding gives way, never the label). */
    fit?: boolean;
    onchange: (v: string) => void;
  } = $props();

  /** The check beside the selected label (px). */
  const CHECK = 14;

  const rItems = $derived(items.map((i) => ({ value: i.id, label: i.id === value ? iconLabel("check", i.label, CHECK) : i.label })));

  let box = $state<HTMLSpanElement>();
  let fontsReady = $state(false);
  document.fonts?.ready.then(() => (fontsReady = true));

  /** The longest label, selected (600, with the check), without side padding. */
  const content = $derived.by(() => {
    void fontsReady;
    if (!box) return 0;
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return 0;
    const cs = getComputedStyle(box);
    ctx.font = `${cs.getPropertyValue("--fw-strong").trim() || 600} ${px("--fs-small")}px ${cs.fontFamily}`;
    const text = Math.max(0, ...items.map((i) => ctx.measureText(i.label).width));
    return Math.ceil(text + CHECK + px("--sp-6"));
  });
  const token = $derived(wide ? "--size-seg-wide" : narrow ? "--size-seg-sm" : "--size-seg");

  // fit: the width of the line the control sits on.
  let line = $state(Infinity);
  $effect(() => {
    const parent = box?.parentElement;
    if (!fit || !parent) return;
    const measure = () => {
      const cs = getComputedStyle(parent);
      line = parent.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(parent);
    measure();
    return () => observer.disconnect();
  });

  const segW = $derived.by(() => {
    if (!content) return `var(${token})`;
    const want = `max(var(${token}), ${content + 2 * px("--sp-12")}px)`;
    if (!fit || !Number.isFinite(line)) return want;
    // Share the line, keeping at least 4 px either side of the label.
    const share = Math.max(content + 2 * px("--sp-4"), Math.floor(line / Math.max(1, items.length)));
    return `min(${want}, ${share}px)`;
  });
</script>

<span class="seg" class:dis={disabled} inert={disabled} aria-disabled={disabled || undefined} style:--seg-w={segW} bind:this={box}>
  <span
    class="host"
    use:react={{
      component: SegmentedControl,
      props: { items: rItems, value, "aria-label": label, onValueChange: (v: string) => v !== value && onchange(v) },
    }}
  ></span>
</span>

<style>
  .seg {
    flex: none;
    display: inline-flex;
  }
  .dis {
    opacity: var(--alpha-disabled);
  }
  .host {
    display: contents;
  }
</style>
