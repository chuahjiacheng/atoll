<!-- Quick note tab (design: Island → note): one plain-text note, saved as you type. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { noteApi, onNote } from "./tools.svelte";
  import { seconds } from "./motion";

  let text = $state("");
  let saved = $state(true);
  let area = $state<HTMLTextAreaElement>();
  let timer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    noteApi.get().then((t) => (text = t));
    // Changed from elsewhere (not while you are typing here).
    const un = onNote((t) => {
      if (document.activeElement !== area) text = t;
    });
    return () => {
      flush();
      un.then((f) => f());
    };
  });

  export function focus() {
    area?.focus();
  }

  function flush() {
    if (saved) return;
    clearTimeout(timer);
    noteApi.set(text).then(() => (saved = true));
  }

  function input() {
    saved = false;
    clearTimeout(timer);
    timer = setTimeout(flush, seconds("--dur-note-save") * 1000);
  }
</script>

<label for="atoll-note" class="label strong">Quick note</label>
<textarea
  id="atoll-note"
  bind:this={area}
  bind:value={text}
  oninput={input}
  onblur={flush}
  placeholder="Type a note. It is kept until you clear it."
  spellcheck="true"
  data-lenis-prevent
></textarea>
<div class="foot label">
  <span>{!text ? "Empty" : saved ? "Saved" : "Saving…"}</span>
  {#if text}<span class="tabular">{text.length} {text.length === 1 ? "character" : "characters"}</span>{/if}
</div>

<style>
  label { margin-bottom: var(--sp-8); }
  textarea {
    flex: 1;
    resize: none;
    box-sizing: border-box;
    border: none;
    border-radius: var(--r-card);
    padding: var(--sp-16);
    background: var(--c-surface-card);
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-body);
    line-height: var(--lh-loose);
    outline: none;
  }
  textarea:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: 0; }
  .foot { display: flex; justify-content: space-between; margin-top: var(--sp-8); }
</style>
