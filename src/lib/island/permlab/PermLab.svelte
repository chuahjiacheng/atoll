<!--
  Permission lab (dev only). Every sample request in a 440 px peek on the island's
  opaque hull, in dark or light. ?only=<n> shows one sample; ?theme=light switches theme.
-->
<script lang="ts">
  import PermissionPeek from "../PermissionPeek.svelte";
  import type { Permission } from "../../api";

  let { fixtures }: { fixtures: { name: string; request: Permission }[] } = $props();
  const params = new URLSearchParams(location.search);
  let theme = $state(params.get("theme") === "light" ? "light" : "dark");
  const only = params.get("only");
  const list = $derived(fixtures.filter((_, i) => only === null || String(i + 1) === only));

  $effect(() => {
    document.documentElement.dataset.theme = theme;
  });
</script>

<main>
  <header>
    <h1>Permission peek</h1>
    <button onclick={() => (theme = theme === "dark" ? "light" : "dark")}>Theme: {theme}</button>
  </header>
  <div class="grid">
    {#each list as f, i (f.name)}
      <section>
        <h2>{i + 1}. {f.name}</h2>
        <div class="hull" data-sample={i + 1}>
          <PermissionPeek
            request={f.request}
            folder="C:\Users\me\proj"
            queued={f.name.startsWith("Risky delete") ? 3 : 1}
            now={0}
            tone="peri"
            focused={i % 2 === 0}
            keys={{ allow: "Ctrl+Alt+Y", deny: "Ctrl+Alt+X" }}
          />
        </div>
      </section>
    {/each}
  </div>
</main>

<style>
  :global(body) { margin: 0; background: var(--c-desk); color: var(--c-ink); font-family: var(--font); }
  main { padding: 24px; }
  header { display: flex; gap: 16px; align-items: center; margin-bottom: 24px; }
  h1 { font-size: 20px; margin: 0; }
  h2 { font-size: 13px; font-weight: 600; margin: 0 0 8px; color: var(--c-mist); }
  .grid { display: flex; flex-wrap: wrap; gap: 32px; align-items: flex-start; }
  .hull {
    width: var(--size-peek-w);
    border-radius: var(--r-peek);
    background: rgb(var(--c-hull-rgb));
    box-shadow: 0 0 0 2px var(--c-accent), 0 0 0 3.5px rgb(var(--c-hull-rgb)), var(--shadow-shell);
  }
</style>
