<!--
  Presets: named snapshots of your settings to switch between. Kept on this
  PC (presets.json next to settings.json); nothing leaves it.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { settingsApi, type Settings } from "../../lib/settings";
  import { MAX_NAME, presetsApi, type PresetList } from "../../lib/presets";
  import { ago } from "../../lib/format";
  import { confirm, store, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import Badge from "../ui/Badge.svelte";
  import PresetRoutines from "./PresetRoutines.svelte";
  import { listen } from "@tauri-apps/api/event";

  let { s }: { s: Settings } = $props();

  let list = $state<PresetList>({ active: null, modified: false, presets: [] });
  let loaded = $state(false);
  let newName = $state("");
  let newError = $state("");
  let editing = $state<string | null>(null);
  let editName = $state("");
  let editError = $state("");
  let busy = $state(false);
  let now = $state(Date.now());

  async function refresh() {
    try {
      list = await presetsApi.list();
    } catch (e) {
      toast(String(e), true);
    }
    loaded = true;
  }

  let routines = $state<PresetRoutines>();

  onMount(() => {
    refresh();
    const t = setInterval(() => (now = Date.now()), 30_000);
    // A routine switched preset while this page is open.
    const off = listen("presets-changed", () => {
      refresh();
      routines?.refresh();
    });
    return () => {
      clearInterval(t);
      off.then((f) => f());
    };
  });

  // Settings changed (here or on another page): recheck "changed since saved".
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    JSON.stringify(s);
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 700);
    return () => clearTimeout(refreshTimer);
  });

  /** Run a preset command; returns false (and shows why) when it fails. */
  async function run(op: () => Promise<PresetList>, done: string): Promise<boolean> {
    busy = true;
    try {
      list = await op();
      if (done) toast(done);
      return true;
    } catch (e) {
      toast(String(e), true);
      return false;
    } finally {
      busy = false;
    }
  }

  async function saveNew(e?: Event) {
    e?.preventDefault();
    newError = "";
    const name = newName.trim();
    if (!name) {
      newError = "Give the preset a name.";
      return;
    }
    busy = true;
    try {
      list = await presetsApi.save(name);
      toast(`Saved “${name}”`);
      newName = "";
    } catch (err) {
      newError = String(err);
    } finally {
      busy = false;
    }
  }

  async function switchTo(id: string, name: string) {
    const go = async () => {
      if (await run(() => presetsApi.apply(id), `Switched to “${name}”`)) {
        store.settings = await settingsApi.get();
      }
    };
    if (list.active && list.modified) {
      const cur = list.presets.find((p) => p.id === list.active)?.name ?? "";
      confirm(
        `Switch to “${name}”?`,
        `You've changed settings since you saved “${cur}”. Those changes will be lost unless you update that preset first.`,
        "Switch",
        go,
      );
    } else {
      await go();
    }
  }

  function update(id: string, name: string) {
    confirm(
      `Update “${name}”?`,
      "The preset is replaced with your current settings.",
      "Update preset",
      () => run(() => presetsApi.overwrite(id), `Updated “${name}”`).then(() => {}),
    );
  }

  function remove(id: string, name: string) {
    confirm(
      `Delete “${name}”?`,
      "The preset is removed from this PC, with any routines that switch to it. Your current settings stay as they are. This cannot be undone.",
      "Delete preset",
      () => run(() => presetsApi.remove(id), `Deleted “${name}”`).then(() => {}),
    );
  }

  async function startRename(id: string, name: string) {
    editing = id;
    editName = name;
    editError = "";
    await tick();
    document.querySelector<HTMLInputElement>(`[data-rename="${id}"]`)?.select();
  }

  async function finishRename(e?: Event) {
    e?.preventDefault();
    if (!editing) return;
    const id = editing;
    const old = list.presets.find((p) => p.id === id)?.name;
    if (editName.trim() === old) {
      editing = null;
      return;
    }
    busy = true;
    try {
      list = await presetsApi.rename(id, editName);
      editing = null;
      toast("Renamed");
    } catch (err) {
      editError = String(err);
    } finally {
      busy = false;
    }
  }

  function renameKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      editing = null;
    }
  }
</script>

<Section title="Save your settings">
  <Row
    icon="plus"
    label="New preset"
    help="Saves everything as it is now: appearance, modules, hotkeys, alerts, focus timer and the rest."
    note={newError}
    noteIcon="alert"
    noteTone="err"
    noteIndent
  >
    <form class="p-form" onsubmit={saveNew}>
      <input
        class="d-field p-name"
        type="text"
        placeholder="e.g. Work, Gaming, Late night"
        maxlength={MAX_NAME}
        aria-label="Preset name"
        aria-invalid={newError ? "true" : undefined}
        bind:value={newName}
        oninput={() => (newError = "")}
      />
      <GButton label="Save" variant="filled" type="submit" disabled={busy || !newName.trim()} />
    </form>
  </Row>
</Section>

<Section
  title="Your presets"
  desc={list.presets.length ? "Switching replaces your current settings with the preset's. Save or update a preset first if you want to keep what you have now." : ""}
>
  {#if !loaded}
    <Row label="Loading…" />
  {:else if list.presets.length === 0}
    <Row icon="info" label="No presets yet" help="Set things up the way you like, then save them above. Make as many as you want and switch whenever." />
  {:else}
    {#each list.presets as p (p.id)}
      {@const active = list.active === p.id}
      <Row
        icon={active ? "check" : "copy"}
        label={p.name}
        help={active
          ? list.modified
            ? `In use, changed since saved ${ago(p.savedAt, now)}`
            : `In use, saved ${ago(p.savedAt, now)}`
          : `Saved ${ago(p.savedAt, now)}`}
        note={editing === p.id ? editError : ""}
        noteIcon="alert"
        noteTone="err"
        noteIndent
      >
        {#if editing === p.id}
          <form class="p-form" onsubmit={finishRename}>
            <input
              class="d-field p-name"
              type="text"
              maxlength={MAX_NAME}
              aria-label="New name for {p.name}"
              aria-invalid={editError ? "true" : undefined}
              data-rename={p.id}
              bind:value={editName}
              oninput={() => (editError = "")}
              onkeydown={renameKey}
            />
            <GButton label="Rename" variant="filled" type="submit" disabled={busy || !editName.trim()} />
            <GButton label="Cancel" onclick={() => (editing = null)} />
          </form>
        {:else}
          {#if active && list.modified}
            <GButton label="Update" title="Save your current settings into this preset" disabled={busy} onclick={() => update(p.id, p.name)} />
          {/if}
          {#if active && !list.modified}
            <span class="p-inuse">In use</span>
          {:else}
            <GButton label={active ? "Revert" : "Switch"} variant={active ? "glass" : "filled"} title={active ? "Go back to this preset as saved" : `Use ${p.name}`} disabled={busy} onclick={() => switchTo(p.id, p.name)} />
          {/if}
          {#if !active}
            <GButton icon="reset" iconSize={16} ariaLabel="Update {p.name} with current settings" title="Update with current settings" disabled={busy} onclick={() => update(p.id, p.name)} />
          {/if}
          <GButton icon="edit" iconSize={16} ariaLabel="Rename {p.name}" title="Rename" disabled={busy} onclick={() => startRename(p.id, p.name)} />
          <GButton icon="trash" iconSize={16} destructive ariaLabel="Delete {p.name}" title="Delete" disabled={busy} onclick={() => remove(p.id, p.name)} />
        {/if}
      </Row>
    {/each}
  {/if}
</Section>

<PresetRoutines bind:this={routines} presets={list.presets} onswitched={refresh} />

<Section title="What a preset keeps">
  <Row
    icon="lock"
    label="Saved on this PC"
    help="Presets live in presets.json in your data folder. Sign-ins, API keys and Start with Windows are never part of a preset, so switching leaves them alone. Routines are kept in the same file."
  >
    <Badge kind="local" />
  </Row>
</Section>

<style>
  .p-form {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
  }
  .p-name {
    width: 220px;
  }
  .p-inuse {
    color: var(--c-ok);
    font-size: var(--fs-body);
    font-weight: 600;
    padding: 0 var(--sp-8);
  }
</style>
