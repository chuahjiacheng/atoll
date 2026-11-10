<!--
  Focus timer (design: Settings → Focus timer): session and break lengths, the
  end sound, what alerts do during focus, the distraction nudge, music, and the
  history kept on this PC.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { settingsApi, type Settings, type SoundName } from "../../lib/settings";
  import { focusApi, focusTotal, onFocus, type FocusView } from "../../lib/focus";
  import { act, change, confirm, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SliderRow from "../ui/SliderRow.svelte";
  import SelectRow from "../ui/SelectRow.svelte";
  import SegRow from "../ui/SegRow.svelte";

  let { s }: { s: Settings } = $props();

  let view = $state<FocusView | null>(null);
  onMount(() => {
    focusApi.get().then((v) => (view = v));
    const un = onFocus((v) => (view = v));
    return () => un.then((f) => f());
  });

  const f = $derived(s.focus);
  const off = $derived(!s.modules.focus);
  const quietOff = $derived(off || !f.dnd);
  const musicReady = $derived(s.modules.spotify && s.spotify.connected);

  const SOUNDS: { v: SoundName; label: string }[] = [
    { v: "chime", label: "Soft chime" },
    { v: "pop", label: "Pop" },
    { v: "bell", label: "Bell" },
    { v: "wood", label: "Wood block" },
    { v: "none", label: "None" },
  ];
  const HOLD = [
    { id: "hold", label: "Hold for the break" },
    { id: "show", label: "Show" },
    { id: "silent", label: "Silent" },
  ];

  function saveApps(text: string) {
    change(
      "focus.nudgeApps",
      text.split(/[,\n]/).map((a) => a.trim()).filter(Boolean),
    );
  }

  async function openWindows() {
    try {
      await focusApi.openWindowsDnd();
    } catch (e) {
      toast(String(e), true);
    }
  }

  function clearHistory() {
    confirm(
      "Clear focus history?",
      "Today's total, your streak and the last 7 days are removed from this PC. A session that is running now carries on.",
      "Clear history",
      () => act(() => focusApi.act("clearHistory"), "Focus history cleared"),
    );
  }

  const stats = $derived(view?.stats ?? null);
  /** Bars are scaled to the busiest day, but never below an hour, so short days look short. */
  const peak = $derived(Math.max(60 * 60000, ...(stats?.week.map((d) => d.focusMs) ?? [0])));
  const weekTotal = $derived(stats?.week.reduce((n, d) => n + d.focusMs, 0) ?? 0);
</script>

<Section title="Focus timer">
  <SwitchRow
    label="Focus timer"
    help="The Focus tab in the island, the timer in the collapsed island, and the hotkey."
    checked={s.modules.focus}
    onchange={(v) => change("modules.focus", v)}
  />
</Section>

<Section title="Sessions">
  <SwitchRow
    label="No time limit"
    help="Sessions run until you take a break, and the timer counts up."
    checked={f.openEnded}
    disabled={off}
    onchange={(v) => change("focus.openEnded", v)}
  />
  <SliderRow
    label="Focus length"
    help="Length of one focus session."
    value={f.focusMin}
    min={5}
    max={90}
    step={5}
    unit="min"
    unitWord="minutes"
    disabled={off || f.openEnded}
    onchange={(v) => change("focus.focusMin", v)}
  />
  <SliderRow
    label="Break length"
    help="Short break after each session."
    value={f.breakMin}
    min={1}
    max={30}
    unit="min"
    unitWord="minutes"
    disabled={off}
    onchange={(v) => change("focus.breakMin", v)}
  />
  <SliderRow
    label="Long break"
    help="A longer break after several sessions."
    value={f.longMin}
    min={5}
    max={60}
    step={5}
    unit="min"
    unitWord="minutes"
    disabled={off}
    onchange={(v) => change("focus.longMin", v)}
  />
  <SliderRow
    label="Sessions before a long break"
    help="The long break comes after this many focus sessions."
    value={f.longEvery}
    min={2}
    max={8}
    unit=""
    unitWord="sessions"
    disabled={off}
    onchange={(v) => change("focus.longEvery", v)}
  />
</Section>

<Section title="Sound and alerts">
  <SelectRow
    label="Sound"
    help="Plays when a session or a break ends."
    value={s.alerts.timer.sound}
    options={SOUNDS}
    disabled={off}
    onchange={(v) => change("alerts.timer.sound", v)}
  >
    {#snippet after()}
      <GButton
        icon="play"
        iconSize={14}
        ariaLabel="Preview the sound"
        disabled={off || s.alerts.timer.sound === "none"}
        onclick={() => act(() => settingsApi.previewSound(s.alerts.timer.sound, s.alerts.timer.volume), "")}
      />
    {/snippet}
  </SelectRow>
  <SwitchRow
    label="Do not disturb"
    help="During focus, Atoll's own alerts follow the choices below instead of interrupting you."
    checked={f.dnd}
    disabled={off}
    onchange={(v) => change("focus.dnd", v)}
  />
  <SegRow
    label="Finished tasks"
    help="Also chat replies and usage warnings. Held ones appear together when the break starts."
    items={HOLD}
    value={f.finished}
    disabled={quietOff}
    onchange={(v) => change("focus.finished", v)}
  />
  <SegRow
    label="Errors"
    help="A session that stopped with an error."
    items={HOLD}
    value={f.errors}
    disabled={quietOff}
    onchange={(v) => change("focus.errors", v)}
  />
  <SegRow
    label="Permission requests"
    help="Claude can't go on without you, so these always appear. Quietly means no sound or notification."
    items={[
      { id: "quiet", label: "Quietly" },
      { id: "show", label: "With sound" },
    ]}
    value={f.permissions}
    disabled={quietOff}
    onchange={(v) => change("focus.permissions", v)}
  />
  <Row
    label="Other apps' notifications"
    help="Apps can't turn on Windows' own Do not disturb. Open its settings to silence other apps too."
    disabled={off}
  >
    <GButton label="Open Windows settings" icon="external" iconSize={16} disabled={off} onclick={openWindows} />
  </Row>
</Section>

<Section title="Distraction nudge" desc="Checked on this PC only. Nothing is ever blocked.">
  <SwitchRow
    label="Nudge me"
    help="When a listed app comes to the front during focus, the island nudges you and counts it."
    checked={f.nudge}
    disabled={off}
    onchange={(v) => change("focus.nudge", v)}
  />
  <Row label="Apps and sites" help="Program names or words in window titles, separated by commas." disabled={off || !f.nudge}>
    {#snippet below()}
      <input
        class="d-field apps"
        type="text"
        value={f.nudgeApps.join(", ")}
        placeholder="YouTube, Discord"
        spellcheck="false"
        aria-label="Apps and sites that count as distractions"
        disabled={off || !f.nudge}
        onchange={(e) => saveApps(e.currentTarget.value)}
      />
    {/snippet}
  </Row>
</Section>

<Section title="Music">
  <SwitchRow
    label="Play music with focus"
    help="Plays Spotify on this PC when a session starts, and pauses it at the break."
    checked={f.music}
    disabled={off || !musicReady}
    note={!musicReady ? "Connect Spotify first (Settings → Spotify)." : ""}
    noteIcon="info"
    noteTone="muted"
    onchange={(v) => change("focus.music", v)}
  />
</Section>

<Section title="History" desc="Kept on this PC only.">
  <Row line={false}>
    {#snippet below()}
      {#if stats}
        <div class="summary">
          <div><span class="big tabular">{focusTotal(stats.todayMs)}</span><span class="cap">Today</span></div>
          <div><span class="big tabular">{stats.todaySessions}</span><span class="cap">{stats.todaySessions === 1 ? "Session today" : "Sessions today"}</span></div>
          <div><span class="big tabular">{stats.streak}</span><span class="cap">{stats.streak === 1 ? "Day streak" : "Days streak"}</span></div>
          <div><span class="big tabular">{focusTotal(weekTotal)}</span><span class="cap">Last 7 days</span></div>
        </div>
        <ul class="chart" aria-label="Focus time, last 7 days">
          {#each stats.week as d, i (d.date)}
            <li aria-label="{d.label}: {focusTotal(d.focusMs)}" title="{d.date}: {focusTotal(d.focusMs)}">
              <span class="bar-box"><span class="bar" class:today={i === stats.week.length - 1} style="height: {(d.focusMs / peak) * 100}%"></span></span>
              <span class="day" aria-hidden="true">{d.label}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <div class="cap">Loading…</div>
      {/if}
    {/snippet}
  </Row>
  <Row label="Clear history" help="Removes the totals above. Your settings stay.">
    <GButton label="Clear history" icon="trash" iconSize={16} destructive disabled={!stats || (!weekTotal && !stats.streak)} onclick={clearHistory} />
  </Row>
</Section>

<style>
  .apps { width: 100%; box-sizing: border-box; }
  .summary {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--sp-16);
    margin-bottom: var(--sp-16);
  }
  .summary > div { display: flex; flex-direction: column; gap: var(--sp-4); }
  .big { font-size: var(--fs-headline); font-weight: var(--fw-strong); }
  .cap { font-size: var(--fs-label); color: var(--c-muted); }
  .chart {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: var(--sp-8);
    height: var(--size-focus-chart, 120px);
  }
  .chart li { display: flex; flex-direction: column; align-items: center; gap: var(--sp-4); min-height: 0; }
  .bar-box {
    flex: 1;
    width: 100%;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    border-radius: var(--r-control);
    background: var(--c-track);
    overflow: hidden;
  }
  .bar {
    width: 100%;
    min-height: 0;
    background: color-mix(in srgb, var(--c-accent) 55%, transparent);
    border-radius: var(--r-control) var(--r-control) 0 0;
  }
  .bar.today { background: var(--c-accent); }
  .day { font-size: var(--fs-label); color: var(--c-muted); }
</style>
