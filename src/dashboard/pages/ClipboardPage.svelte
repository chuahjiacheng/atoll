<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { clipApi, onClipboard, type Clip } from "../../lib/tools.svelte";
  import type { Settings } from "../../lib/settings";
  import { act, change, confirm } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SliderRow from "../ui/SliderRow.svelte";
  import CountUp from "../ui/CountUp.svelte";

  let { s }: { s: Settings } = $props();

  let clips = $state<Clip[]>([]);
  onMount(() => {
    clipApi.list().then((c) => (clips = c));
    const un = onClipboard((c) => (clips = c));
    return () => un.then((f) => f());
  });

  const c = $derived(s.clipboard);
  const pinned = $derived(clips.filter((x) => x.pinned).length);
  const loose = $derived(clips.length - pinned);

  function clearHistory() {
    confirm(
      "Clear clipboard history?",
      `This removes ${loose} stored ${loose === 1 ? "item" : "items"}${pinned ? `. ${pinned} pinned stay` : ""}. This cannot be undone.`,
      "Clear history",
      () => act(clipApi.clear, "Clipboard history cleared"),
    );
  }
</script>

<Section title="Clipboard history">
  <SliderRow
    label="History length"
    help="How many items are kept. Pinned items don't count."
    value={c.maxItems}
    min={10}
    max={200}
    step={10}
    unit="items"
    onchange={(v) => change("clipboard.maxItems", v)}
  />
  <SwitchRow label="Pause history" help="New copies are not saved while paused." checked={c.paused} onchange={(v) => change("clipboard.paused", v)} />
  <SwitchRow
    label="Skip password manager entries"
    help="Copies that apps mark as private (password managers do) are not saved."
    checked={c.skipPrivate}
    onchange={(v) => change("clipboard.skipPrivate", v)}
  />
  <SwitchRow
    label="Encrypt history"
    help="Stored items and images are encrypted for your Windows account only."
    checked={c.encrypt}
    onchange={(v) => change("clipboard.encrypt", v)}
  />
  <SwitchRow
    label="Keep history after restart"
    help="Pinned items are always kept."
    checked={c.persist}
    onchange={(v) => change("clipboard.persist", v)}
  />
  <SwitchRow label="Save images" checked={c.images} onchange={(v) => change("clipboard.images", v)} />
  <SwitchRow label="Save copied files" checked={c.files} onchange={(v) => change("clipboard.files", v)} />
  <SwitchRow
    label="Click to paste"
    help="Clicking an item pastes it into the app you were in. Off only copies it."
    checked={c.clickPaste}
    onchange={(v) => change("clipboard.clickPaste", v)}
  />
  {#if clips.length}
    <Row icon="clipboard" help="The oldest items are removed after {c.maxItems}.">
      {#snippet title()}
        <CountUp value={clips.length} />
        {clips.length === 1 ? "item" : "items"} stored{pinned ? `, ${pinned} pinned` : ""}
      {/snippet}
      <GButton label="Clear history" icon="trash" iconSize={16} destructive disabled={!loose} onclick={clearHistory} />
    </Row>
  {:else}
    <Row line={false}>
      {#snippet below()}
        <div class="empty">
          <span class="tile"><Icon name="clipboard" size={24} /></span>
          <span class="t">No clipboard history</span>
          <span class="h">{c.paused ? "History is paused. Turn off Pause history to start saving again." : `Items you copy will be kept here, up to ${c.maxItems}.`}</span>
        </div>
      {/snippet}
    </Row>
  {/if}
</Section>

<Section title="Whiteboard">
  <SwitchRow
    label="Add copies to the whiteboard"
    help="Everything you copy also lands on the whiteboard as a card."
    checked={s.whiteboard.autoAddCopies}
    onchange={(v) => change("whiteboard.autoAddCopies", v)}
  />
  <SwitchRow
    label="Click to paste"
    help="Clicking a card pastes it into the app you were in. Off only copies it."
    checked={s.whiteboard.clickPaste}
    onchange={(v) => change("whiteboard.clickPaste", v)}
  />
</Section>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-8);
    padding: var(--sp-8) var(--sp-16);
    text-align: center;
  }
  .tile {
    width: var(--size-empty-tile);
    height: var(--size-empty-tile);
    border-radius: var(--r-card);
    background: var(--c-sel);
    color: var(--c-muted);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .t {
    font-weight: var(--fw-strong);
  }
  .h {
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
</style>
