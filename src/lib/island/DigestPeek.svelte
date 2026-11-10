<!--
  "While you focused" peek: the alerts focus mode held back, shown when the
  break starts. Dismiss clears them; Open shows the Focus tab.
-->
<script lang="ts">
  import type { Held } from "../focus";
  import GButton from "../glass/GButton.svelte";
  import Icon, { type IconName } from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let {
    held,
    breakLabel,
    tone,
    onDismiss,
    onView,
  }: { held: Held[]; breakLabel: string; tone: Tone; onDismiss: () => void; onView: () => void } = $props();

  const KIND_ICON: Record<string, IconName> = { done: "check", error: "alert", attention: "agent", usage: "clock", chatReply: "send", timer: "focus" };
  const shown = $derived(held.slice(-3).reverse());
</script>

<div class="peek" role="dialog" aria-label="While you focused: {held.length} {held.length === 1 ? 'update' : 'updates'}">
  <div class="top">
    <Pip mood="happy" {tone} size={40} />
    <div class="body">
      <div class="kicker"><Icon name="coffee" size={16} /><span>{breakLabel}</span></div>
      <div class="title">While you focused</div>
      <ul>
        {#each shown as h (h.at + h.title)}
          <li>
            <span class="ic {h.kind}"><Icon name={KIND_ICON[h.kind] ?? "dot"} size={16} /></span>
            <span class="text"><span class="ellipsis strong">{h.title}</span><span class="ellipsis muted">{h.body}</span></span>
          </li>
        {/each}
      </ul>
      {#if held.length > shown.length}<div class="more">and {held.length - shown.length} more in the Focus tab</div>{/if}
    </div>
  </div>
  <div class="actions">
    <GButton label="Dismiss" grow onclick={onDismiss} />
    <GButton label="Open Focus" variant="filled" grow onclick={onView} />
  </div>
</div>

<style>
  .peek { padding: var(--sp-16); display: flex; flex-direction: column; gap: var(--sp-16); }
  .top { display: flex; gap: var(--sp-16); align-items: flex-start; }
  .body { display: flex; flex-direction: column; gap: var(--sp-8); min-width: 0; flex: 1; }
  .kicker { display: flex; align-items: center; gap: var(--sp-4); font-size: var(--fs-label); font-weight: var(--fw-strong); color: var(--c-done); }
  .title { font-size: var(--fs-title); font-weight: var(--fw-strong); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-8); }
  li { display: flex; gap: var(--sp-8); align-items: flex-start; min-width: 0; }
  .text { display: flex; flex-direction: column; min-width: 0; font-size: var(--fs-small); }
  .strong { font-weight: var(--fw-strong); }
  .muted, .more { color: var(--c-muted); }
  .more { font-size: var(--fs-label); }
  .ic { display: flex; flex: none; color: var(--c-muted); }
  .ic.done { color: var(--c-done); }
  .ic.error { color: var(--c-error); }
  .ic.attention { color: var(--c-accent-text); }
  .actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-8); }
</style>
