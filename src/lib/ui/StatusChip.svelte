<!--
  An agent's status as a small tinted tag: icon + word ("Error", "Needs you",
  "Done"...). With `onClear`, the tag ends in a × that clears the message, so
  the way to dismiss it sits right on the thing being dismissed.
-->
<script lang="ts">
  import type { Status } from "../api";
  import { STATUS_LOOK } from "../format";
  import Icon from "./Icon.svelte";

  let { status, onClear, hint = "" }: { status: Status; onClear?: () => void; hint?: string } = $props();
  const look = $derived(STATUS_LOOK[status]);
</script>

<span class="chip {status}" class:clearable={!!onClear}>
  <Icon name={look.icon} size={16} />
  <span>{look.word}</span>
  {#if onClear}
    <button
      class="x"
      aria-label="Clear this {look.word.toLowerCase()} message"
      title={hint ? `Clear (${hint})` : "Clear"}
      onclick={(e) => {
        e.stopPropagation();
        onClear?.();
      }}
    >
      <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3.5 3.5l5 5M8.5 3.5l-5 5" /></svg>
    </button>
  {/if}
</span>

<style>
  .chip {
    --tint: var(--c-muted);
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    height: 24px;
    padding: 0 var(--sp-8) 0 6px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--tint) 14%, transparent);
    color: var(--tint);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    line-height: 1;
    white-space: nowrap;
    flex: none;
  }
  .chip.clearable { padding-right: 2px; }
  .chip.working { --tint: var(--c-busy); }
  .chip.done { --tint: var(--c-done); }
  .chip.error { --tint: var(--c-error); }
  .chip.attention { --tint: var(--c-accent-text); }
  .x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-left: 2px;
    border-radius: var(--r-pill);
    color: inherit;
    opacity: 0.7;
    pointer-events: auto;
    transition: opacity var(--dur-fast), background-color var(--dur-fast);
  }
  .x svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; }
  .x:hover,
  .x:focus-visible {
    opacity: 1;
    background-color: color-mix(in srgb, var(--tint) 22%, transparent);
  }
</style>
