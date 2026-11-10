<script lang="ts">
  import { onMount } from "svelte";
  import { settingsApi, type MonitorInfo, type Settings } from "../../lib/settings";
  import { change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SelectRow from "../ui/SelectRow.svelte";
  import ScreenPicker from "../ui/ScreenPicker.svelte";
  import { pretty } from "../ui/KeyRow.svelte";

  let { s }: { s: Settings } = $props();

  let monitors = $state<MonitorInfo[]>([]);
  let exe = $state("");
  onMount(async () => {
    monitors = await settingsApi.monitors();
    exe = (await settingsApi.appInfo()).exe;
  });

  const a = $derived(s.appearance);
  const monitor = $derived(
    monitors.find((m) => (a.monitor === "primary" ? m.primary : m.name === a.monitor)) ?? monitors.find((m) => m.primary) ?? null,
  );
  const monitorOptions = $derived([
    { v: "primary", label: "Main display" },
    ...monitors.map((m, i) => ({
      v: m.name,
      label: `Display ${i + 1}${m.primary ? " (main)" : ""}, ${Math.round(m.width / m.scale)} × ${Math.round(m.height / m.scale)}`,
    })),
  ]);
  const devBuild = $derived(/target[\\/]debug/i.test(exe));
  const settingsKey = $derived(s.hotkeys.openSettings ? pretty(s.hotkeys.openSettings) : "");
</script>

<Section title="Startup">
  <SwitchRow
    label="Start with Windows"
    help={devBuild && s.general.startWithWindows
      ? "This is a development build, so Windows would start it from target\\debug. Turn this on from the release build."
      : "Opens the island when you sign in."}
    checked={s.general.startWithWindows}
    onchange={(v) => change("general.startWithWindows", v)}
  />
  <SwitchRow
    label="Show tray icon"
    help={settingsKey ? `When off, open settings with ${settingsKey}.` : "When off, set an Open settings hotkey first, or you can't get back here."}
    checked={s.general.showTray}
    onchange={(v) => change("general.showTray", v)}
  />
</Section>

<Section title="Language">
  <SelectRow
    label="Language"
    help="Used in the island and in this window."
    value={s.general.language}
    options={[{ v: "en-GB", label: "English (United Kingdom)" }]}
    onchange={(v) => change("general.language", v)}
  />
</Section>

<Section title="Position">
  <Row line={false}>
    {#snippet below()}
      <ScreenPicker
        {a}
        {monitor}
        onmove={(x, y) => {
          if (x !== a.positionX) change("appearance.positionX", x);
          if (y !== a.positionY) change("appearance.positionY", y);
        }}
      />
    {/snippet}
  </Row>
  <SelectRow
    label="Monitor"
    help="Which display shows the island."
    value={a.monitor}
    options={monitorOptions}
    onchange={(v) => change("appearance.monitor", v)}
  />
  <SwitchRow
    label="Hide during fullscreen apps"
    help="The tray icon shows status while the island is hidden."
    checked={s.general.hideFullscreen}
    onchange={(v) => change("general.hideFullscreen", v)}
    note={!s.general.showTray && s.general.hideFullscreen
      ? "The tray icon is off, so status will not show anywhere while a fullscreen app is open."
      : ""}
    noteIcon="alert"
    noteTone="ink"
  />
</Section>
