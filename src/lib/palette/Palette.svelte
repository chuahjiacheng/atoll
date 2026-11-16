<!--
  Command palette (search-clipboard branch): one keyboard-first search over
  clipboard, whiteboard text, the quick note, file tray, calendar and agent
  history. Drawn as a peek under the island in its own window.

  Keys: type to search · ↑/↓ choose · Enter = first action · Ctrl+Enter copy ·
  Alt+Enter open / show in folder · Tab / Shift+Tab change source · Esc close.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import Icon from "../ui/Icon.svelte";
  import { settingsApi, onSettings } from "../settings";
  import { applyAppearance, watchSystem } from "../theme";
  import { ago } from "../tools.svelte";
  import { ACTION_LABEL, SOURCES, onPaletteOpen, searchApi, type Action, type SearchResult, type Source } from "./api";

  let query = $state("");
  let source = $state<Source | null>(null);
  let results = $state<SearchResult[]>([]);
  let selected = $state(0);
  let error = $state("");
  let opened = $state(0);
  let now = $state(Date.now());
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLDivElement>();
  let seq = 0;

  const ICON: Record<Source, string> = {
    clipboard: "clipboard",
    whiteboard: "whiteboard",
    note: "note",
    fileTray: "folder",
    calendar: "calendar",
    agent: "agent",
  };

  async function run() {
    const mine = ++seq;
    try {
      const r = await searchApi.query(query, source, 30);
      if (mine !== seq) return; // a newer keystroke already answered
      results = r;
      selected = 0;
      error = "";
      list?.scrollTo({ top: 0 });
    } catch (e) {
      if (mine === seq) error = String(e);
    }
  }

  $effect(() => {
    void query;
    void source;
    void opened;
    run();
  });

  onMount(() => {
    let settings: Awaited<ReturnType<typeof settingsApi.get>> | null = null;
    const theme = () => settings && applyAppearance(settings.appearance);
    settingsApi.get().then((s) => {
      settings = s;
      theme();
      // Show the window only once it has drawn, so the first open never flashes.
      requestAnimationFrame(() => searchApi.ready());
    });
    const unSettings = onSettings((s) => {
      settings = s;
      theme();
    });
    const unSystem = watchSystem(theme);
    const unOpen = onPaletteOpen(async () => {
      query = "";
      source = null;
      now = Date.now();
      opened++;
      await tick();
      input?.focus();
    });
    input?.focus();
    return () => {
      unSettings.then((f) => f());
      unOpen.then((f) => f());
      unSystem();
    };
  });

  const scope = $derived(SOURCES.find((s) => s.key === source)!);

  function cycle(step: number) {
    const i = SOURCES.findIndex((s) => s.key === source);
    source = SOURCES[(i + step + SOURCES.length) % SOURCES.length].key;
  }

  /** The action Alt+Enter runs: open or reveal, whichever isn't already Enter's. */
  function altAction(r: SearchResult): Action | undefined {
    return r.actions.slice(1).find((a) => a === "open" || a === "reveal");
  }

  async function act(r: SearchResult | undefined, action: Action | undefined) {
    if (!r || !action) return;
    try {
      await searchApi.act(r.key, action);
    } catch (e) {
      error = String(e);
    }
  }

  function keydown(e: KeyboardEvent) {
    const r = results[selected];
    if (e.key === "ArrowDown") selected = Math.min(results.length - 1, selected + 1);
    else if (e.key === "ArrowUp") selected = Math.max(0, selected - 1);
    else if (e.key === "PageDown") selected = Math.min(results.length - 1, selected + 6);
    else if (e.key === "PageUp") selected = Math.max(0, selected - 6);
    else if (e.key === "Tab") cycle(e.shiftKey ? -1 : 1);
    else if (e.key === "Escape") searchApi.hide();
    else if (e.key === "Enter" && e.ctrlKey) act(r, r?.actions.includes("copy") ? "copy" : undefined);
    else if (e.key === "Enter" && e.altKey) act(r, r && altAction(r));
    else if (e.key === "Enter") act(r, r?.actions[0]);
    else return;
    e.preventDefault();
    tick().then(() => list?.querySelector<HTMLElement>(`[data-i="${selected}"]`)?.scrollIntoView({ block: "nearest" }));
  }

  function when(r: SearchResult) {
    if (!r.at) return "";
    if (r.source === "calendar") {
      const d = new Date(r.at);
      return d.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" });
    }
    return ago(r.at, now);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="wrap" onkeydown={keydown} onpointerdown={(e) => e.target === e.currentTarget && searchApi.hide()}>
  {#key opened}
    <div class="peek" role="dialog" aria-label="Search Atoll">
      <div class="field">
        <Icon name="search" size={18} />
        <input
          bind:this={input}
          bind:value={query}
          placeholder={source ? `Search ${scope.label.toLowerCase()}` : "Search copies, notes, files, events, agent history"}
          aria-label="Search"
          aria-controls="palette-results"
          aria-activedescendant={results[selected] ? `r-${selected}` : undefined}
          spellcheck="false"
          autocomplete="off"
        />
      </div>

      <div class="scopes" role="tablist" aria-label="Search in">
        {#each SOURCES as s (s.label)}
          <button
            class="scope"
            class:on={s.key === source}
            role="tab"
            aria-selected={s.key === source}
            tabindex="-1"
            onclick={() => {
              source = s.key;
              input?.focus();
            }}
          >
            {#if s.key === source}<Icon name="check" size={14} />{/if}{s.label}
          </button>
        {/each}
      </div>

      <div class="results" id="palette-results" role="listbox" aria-label="Results" bind:this={list}>
        {#if !query.trim() && results.length}
          <p class="label group">Recent</p>
        {/if}
        {#each results as r, i (r.key)}
          <div
            class="row"
            class:sel={i === selected}
            id="r-{i}"
            data-i={i}
            role="option"
            tabindex="-1"
            aria-selected={i === selected}
            onpointermove={() => (selected = i)}
            onclick={() => act(r, r.actions[0])}
          >
            <span class="tile"><Icon name={ICON[r.source]} size={18} /></span>
            <span class="text">
              <span class="ellipsis title" dir="auto">{r.title || "(empty)"}</span>
              <span class="ellipsis label">{r.sub}{when(r) ? ` · ${when(r)}` : ""}</span>
            </span>
            {#if r.pinned}<span class="pin" title="Pinned"><Icon name="pin" size={14} /></span>{/if}
            {#if i === selected}
              <span class="hint"><kbd>↵</kbd>{ACTION_LABEL[r.actions[0]]}</span>
            {/if}
          </div>
        {:else}
          <div class="empty">
            {#if query.trim()}
              <p class="title">Nothing matches “{query.trim()}”</p>
              <p class="label">{source ? "Press Tab to search everything, or try fewer letters." : "Try fewer letters or another word."}</p>
            {:else}
              <p class="title">Nothing to search yet</p>
              <p class="label">Copies, notes, tray files, events and agent history show up here.</p>
            {/if}
          </div>
        {/each}
      </div>

      <p class="sr" role="status">{error}</p>
      <div class="foot label" aria-hidden="true">
        {#if error}
          <span class="error">{error}</span>
        {:else}
          {@const r = results[selected]}
          {#if r?.actions.includes("copy") && r.actions[0] !== "copy"}<span><kbd>Ctrl</kbd><kbd>↵</kbd>Copy</span>{/if}
          {#if r && altAction(r)}<span><kbd>Alt</kbd><kbd>↵</kbd>{ACTION_LABEL[altAction(r)!]}</span>{/if}
          <span><kbd>Tab</kbd>Source</span>
          <span><kbd>Esc</kbd>Close</span>
        {/if}
      </div>
    </div>
  {/key}
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent;
  }
  .wrap {
    box-sizing: border-box;
    height: 100%;
    padding: 32px;
  }
  .peek {
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
    padding: var(--sp-16);
    border-radius: var(--r-peek);
    background: var(--c-hull);
    box-shadow: var(--shadow-shell);
    color: var(--c-ink);
    animation: drop var(--dur-peek) var(--ease-settle);
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.985);
    }
  }
  /* Atoll's Reduce motion setting (or Windows'), set by applyAppearance: a short fade only. */
  :global([data-reduce-motion="true"]) .peek {
    animation: fade var(--dur-reduced) linear;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    height: var(--size-large-h);
    padding: 0 var(--sp-16);
    border-radius: var(--r-control);
    background: var(--c-raised);
    color: var(--c-muted);
  }
  .field:focus-within {
    outline: var(--focus-width) solid var(--c-accent);
    outline-offset: calc(var(--focus-width) * -1);
  }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-title);
    caret-color: var(--c-accent);
  }
  input::placeholder {
    color: var(--c-muted);
  }
  input::selection {
    background: var(--c-selection);
  }
  .scopes {
    display: flex;
    gap: var(--sp-4);
    overflow: hidden;
  }
  .scope {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    height: var(--size-tab);
    padding: 0 var(--sp-12);
    border-radius: var(--r-pill);
    font-size: var(--fs-small);
    color: var(--c-muted);
    background: transparent;
    white-space: nowrap;
  }
  .scope:hover {
    color: var(--c-ink);
    background: var(--c-hover);
  }
  .scope.on {
    color: var(--c-accent-ink);
    background: var(--c-accent);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    scrollbar-width: thin;
    scrollbar-color: var(--c-hover) transparent;
  }
  .group {
    margin: var(--sp-4) var(--sp-8) 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    min-height: 52px;
    padding: var(--sp-8);
    box-sizing: border-box;
    border-radius: var(--r-control);
    cursor: pointer;
  }
  .row.sel {
    background: var(--c-sel);
  }
  .tile {
    width: var(--size-row-tile);
    height: var(--size-row-tile);
    flex: none;
    display: grid;
    place-items: center;
    border-radius: var(--r-tile);
    background: var(--c-raised);
    color: var(--c-muted);
  }
  .row.sel .tile {
    color: var(--c-accent-text);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: var(--fs-small);
  }
  .label {
    font-size: var(--fs-label);
    color: var(--c-muted);
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pin {
    color: var(--c-accent-text);
    display: grid;
  }
  .hint {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    color: var(--c-ink);
  }
  kbd {
    display: inline-grid;
    place-items: center;
    min-width: var(--size-keycap-w);
    height: var(--size-keycap-h);
    padding: 0 var(--sp-4);
    box-sizing: border-box;
    border-radius: var(--r-keycap);
    background: var(--c-button);
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
  }
  .empty {
    margin: auto;
    text-align: center;
  }
  .empty p {
    margin: var(--sp-4) 0;
  }
  .empty .title {
    font-size: var(--fs-body);
    font-weight: var(--fw-strong);
  }
  .foot {
    display: flex;
    gap: var(--sp-16);
    justify-content: flex-end;
    padding: 0 var(--sp-8);
  }
  .foot span {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
  }
  .error {
    color: var(--c-error);
    margin-right: auto;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: 0;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
