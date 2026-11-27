<!--
  Routines (part of the Presets page): switch preset at a time of day, or
  when screens are plugged in or unplugged. The rules run in Rust
  (src-tauri/src/routines.rs); this only edits them.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { settingsApi } from "../../lib/settings";
  import { routinesApi, type PresetInfo, type Routine, type RoutineList } from "../../lib/presets";
  import { ago } from "../../lib/format";
  import { confirm, store, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import Switch from "../ui/Switch.svelte";
  import Seg from "../ui/Seg.svelte";
  import Select from "../ui/Select.svelte";

  let { presets, onswitched }: { presets: PresetInfo[]; onswitched: () => void } = $props();

  let list = $state<RoutineList | null>(null);
  let now = $state(Date.now());
  let busy = $state(false);

  /** The routine being edited ("" id = new), or null. */
  let draft = $state<Routine | null>(null);
  let draftError = $state("");

  export async function refresh() {
    try {
      list = await routinesApi.list();
    } catch (e) {
      toast(String(e), true);
    }
  }

  onMount(() => {
    refresh();
    const t = setInterval(() => {
      now = Date.now();
      refresh(); // screen count and a routine that just fired
    }, 15_000);
    return () => clearInterval(t);
  });

  // A preset deleted: its routines went with it.
  $effect(() => {
    presets.map((p) => p.id).join();
    refresh();
  });

  async function run(op: () => Promise<RoutineList>, done = ""): Promise<boolean> {
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

  const name = (id: string) => presets.find((p) => p.id === id)?.name ?? "a deleted preset";

  // ---------- editor ----------
  const DAYS = ["S", "M", "T", "W", "T", "F", "S"];
  const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
  const SCREENS = [
    { v: "1", label: "Only 1 (nothing plugged in)" },
    { v: "2", label: "Exactly 2" },
    { v: "2+", label: "2 or more" },
    { v: "3", label: "Exactly 3" },
    { v: "3+", label: "3 or more" },
  ];

  function nextHour(): string {
    const h = (new Date().getHours() + 1) % 24;
    return `${String(h).padStart(2, "0")}:00`;
  }

  function startNew(kind: "time" | "displays") {
    draftError = "";
    // Sensible first guess: the next full hour on weekdays, or the opposite of
    // the screens connected now (so it fires the next time you plug or unplug).
    const n = list?.displays ?? 1;
    draft = {
      id: "",
      enabled: true,
      presetId: presets.find((p) => !list?.routines.some((r) => r.presetId === p.id))?.id ?? presets[0]?.id ?? "",
      when: kind === "time" ? { kind: "time", at: nextHour(), days: [1, 2, 3, 4, 5] } : n > 1 ? { kind: "displays", count: 1, orMore: false } : { kind: "displays", count: 2, orMore: true },
    };
  }

  function edit(r: Routine) {
    draftError = "";
    draft = JSON.parse(JSON.stringify({ id: r.id, enabled: r.enabled, presetId: r.presetId, when: r.when }));
  }

  function setKind(kind: string) {
    if (!draft || draft.when.kind === kind) return;
    const keep = { id: draft.id, enabled: draft.enabled, presetId: draft.presetId };
    startNew(kind as "time" | "displays");
    draft = { ...draft!, ...keep };
  }

  function toggleDay(d: number) {
    if (draft?.when.kind !== "time") return;
    const days = draft.when.days;
    draft.when.days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort();
    draftError = "";
  }

  function setDays(days: number[]) {
    if (draft?.when.kind === "time") draft.when.days = days;
    draftError = "";
  }

  const screensValue = $derived(draft?.when.kind === "displays" ? `${draft.when.count}${draft.when.orMore ? "+" : ""}` : "1");

  async function saveDraft(e?: Event) {
    e?.preventDefault();
    if (!draft) return;
    busy = true;
    try {
      const isNew = !draft.id;
      list = await routinesApi.save($state.snapshot(draft));
      draft = null;
      toast(isNew ? "Routine added" : "Routine saved");
    } catch (err) {
      draftError = String(err);
    } finally {
      busy = false;
    }
  }

  function remove(r: Routine & { label: string }) {
    confirm(`Delete this routine?`, `“${r.label}, switch to ${name(r.presetId)}” will stop. The preset itself stays.`, "Delete routine", () =>
      run(() => routinesApi.remove(r.id), "Routine deleted").then(() => {}),
    );
  }

  async function undo() {
    if (await run(routinesApi.undo, "Switch undone")) {
      store.settings = await settingsApi.get();
      onswitched();
    }
  }

  const screensNow = $derived(list ? `${list.displays} screen${list.displays === 1 ? "" : "s"} connected now.` : "");
</script>

<Section
  title="Routines"
  desc="Switch presets on their own: at a time of day, or when you plug in or unplug screens. A routine acts when its moment comes, so a preset you pick by hand stays until the next one."
>
  {#if presets.length === 0}
    <Row icon="info" label="Save a preset first" help="Routines switch between your presets, so make at least one above." />
  {:else if list}
    {#if list.lastSwitch}
      <Row
        icon="reset"
        label="Switched to {name(list.lastSwitch.presetId)} automatically"
        help="{list.lastSwitch.reason}, {ago(list.lastSwitch.at, now)}. Undo puts your settings back as they were just before."
      >
        <GButton label="Undo" disabled={busy} onclick={undo} />
      </Row>
    {/if}

    <SwitchRow
      label="Run routines"
      help={list.options.paused ? "Paused. Nothing switches until you turn this back on." : "On. Routines switch presets for you."}
      checked={!list.options.paused}
      onchange={(v) => run(() => routinesApi.options({ paused: !v }))}
    />
    <SwitchRow
      label="Tell me when a routine switches"
      help="A Windows notification names the preset and why."
      checked={list.options.notify}
      disabled={list.options.paused}
      onchange={(v) => run(() => routinesApi.options({ notify: v }))}
    />

    {#each list.routines as r (r.id)}
      {#if draft && draft.id === r.id}
        {@render editor()}
      {:else}
        <Row
          icon={r.when.kind === "time" ? "clock" : "monitor"}
          label={r.label}
          help="Switches to {name(r.presetId)}"
          disabled={list.options.paused}
          note={list.overlaps.includes(r.id) ? "Another screen routine can fire on the same change. The one higher in this list wins." : ""}
          noteIcon="alert"
          noteTone="ink"
          noteIndent
        >
          <Switch checked={r.enabled} label="Routine on" disabled={busy} onchange={(v) => run(() => routinesApi.save({ ...$state.snapshot(r), enabled: v }))} />
          <GButton icon="edit" iconSize={16} ariaLabel="Edit routine" title="Edit" disabled={busy} onclick={() => edit(r)} />
          <GButton icon="trash" iconSize={16} destructive ariaLabel="Delete routine" title="Delete" disabled={busy} onclick={() => remove(r)} />
        </Row>
      {/if}
    {/each}

    {#if draft && !draft.id}
      {@render editor()}
    {:else if !draft}
      <Row icon="plus" label="Add a routine" help={screensNow}>
        <GButton label="At a time" icon="clock" iconSize={16} onclick={() => startNew("time")} />
        <GButton label="When screens change" icon="monitor" iconSize={16} onclick={() => startNew("displays")} />
      </Row>
    {/if}
  {/if}
</Section>

{#snippet editor()}
  {#if draft}
    <Row line={false} note={draftError} noteIcon="alert" noteTone="err">
      {#snippet below()}
        {#if draft}
        <form class="r-edit" onsubmit={saveDraft} aria-label={draft?.id ? "Edit routine" : "New routine"}>
          <div class="r-line">
            <span class="r-key">When</span>
            <Seg
              label="Routine type"
              wide
              items={[
                { id: "time", label: "At a time" },
                { id: "displays", label: "Screens change" },
              ]}
              value={draft.when.kind}
              onchange={setKind}
            />
          </div>

          {#if draft.when.kind === "time"}
            <div class="r-line">
              <span class="r-key">Time</span>
              <input
                class="d-field r-time"
                type="time"
                aria-label="Time"
                bind:value={draft.when.at}
                oninput={() => (draftError = "")}
              />
            </div>
            <div class="r-line">
              <span class="r-key">Days</span>
              <div class="r-days" role="group" aria-label="Days">
                {#each DAYS as d, i}
                  <button
                    type="button"
                    class="r-day"
                    aria-pressed={draft.when.days.includes(i)}
                    aria-label={DAY_NAMES[i]}
                    title={DAY_NAMES[i]}
                    onclick={() => toggleDay(i)}>{d}</button
                  >
                {/each}
              </div>
              <div class="r-quick">
                <button type="button" class="r-link" onclick={() => setDays([1, 2, 3, 4, 5])}>Weekdays</button>
                <button type="button" class="r-link" onclick={() => setDays([0, 6])}>Weekends</button>
                <button type="button" class="r-link" onclick={() => setDays([0, 1, 2, 3, 4, 5, 6])}>Every day</button>
              </div>
            </div>
          {:else}
            <div class="r-line">
              <span class="r-key">Screens</span>
              <Select
                label="Screens connected"
                value={screensValue}
                options={SCREENS}
                onchange={(v) => {
                  if (draft) draft.when = { kind: "displays", count: parseInt(v), orMore: v.endsWith("+") };
                  draftError = "";
                }}
              />
              <span class="r-hint">{screensNow}</span>
            </div>
          {/if}

          <div class="r-line">
            <span class="r-key">Switch to</span>
            <Select
              label="Preset"
              value={draft.presetId}
              options={presets.map((p) => ({ v: p.id, label: p.name }))}
              onchange={(v) => {
                if (draft) draft.presetId = v;
                draftError = "";
              }}
            />
          </div>

          <div class="r-line r-actions">
            <GButton label={draft.id ? "Save routine" : "Add routine"} variant="filled" type="submit" disabled={busy} />
            <GButton label="Cancel" onclick={() => (draft = null)} />
          </div>
        </form>
        {/if}
      {/snippet}
    </Row>
  {/if}
{/snippet}

<style>
  .r-edit {
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
    padding: var(--sp-4) 0;
  }
  .r-line {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-12);
  }
  .r-key {
    width: 72px;
    color: var(--c-muted);
    font-size: var(--fs-body);
  }
  .r-time {
    width: 120px;
  }
  .r-days {
    display: flex;
    gap: var(--sp-4);
  }
  .r-day {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 1px solid var(--c-ctrl);
    background: var(--c-field);
    color: var(--c-ink);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .r-day[aria-pressed="true"] {
    background: var(--c-accent);
    border-color: var(--c-accent);
    color: var(--c-accent-ink);
  }
  .r-day:focus-visible,
  .r-link:focus-visible {
    outline: var(--focus-width) solid var(--c-accent);
    outline-offset: 2px;
  }
  .r-quick {
    display: flex;
    gap: var(--sp-8);
  }
  .r-link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--c-accent);
    font: inherit;
    font-size: var(--fs-small);
    cursor: pointer;
  }
  .r-hint {
    color: var(--c-muted);
    font-size: var(--fs-small);
  }
  .r-actions {
    padding-left: calc(72px + var(--sp-12));
  }
</style>
