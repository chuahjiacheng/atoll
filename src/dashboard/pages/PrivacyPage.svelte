<!--
  Privacy and data. The design marked several items "To confirm"; the app is
  built now, so each says what actually happens.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { settingsApi, type Settings } from "../../lib/settings";
  import { act, confirm, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import Badge from "../ui/Badge.svelte";

  let { s }: { s: Settings } = $props();

  let dataDir = $state("");
  onMount(async () => (dataDir = (await settingsApi.appInfo()).dataDir));

  // Both open a file dialog; nothing happens if it is cancelled.
  async function viaFile(run: () => Promise<boolean>, done: string) {
    try {
      if (await run()) toast(done);
    } catch (e) {
      toast(String(e), true);
    }
  }

  function clearAll() {
    confirm(
      "Clear all data?",
      "This removes your settings, clipboard history, whiteboard, file tray, note, focus history and Spotify sign-in. Your Claude Code hooks stay. This cannot be undone.",
      "Clear all data",
      () => act(settingsApi.clearAllData, "All data cleared"),
    );
  }
</script>

<Section title="Storage">
  <Row icon="folder" label="Data folder" help={dataDir || "…"}>
    <GButton label="Open folder" icon="external" iconSize={16} onclick={() => act(() => settingsApi.openFolder("data"), "")} />
  </Row>
</Section>

<Section title="What leaves this PC">
  <Row icon="music" label="Spotify" help="Atoll reads and controls the Spotify app on this PC through Windows, so nothing is sent for that.">
    <Badge kind="local" />
  </Row>
  <Row icon="speaker" label="Spotify device switching" help="Only with a linked account: the device list and your device choice go to Spotify. The sign-in is kept in Windows Credential Manager.">
    <Badge kind="when" />
  </Row>
  <Row icon="lyrics" label="Lyrics" help="The song title, artist, album and length go to LRCLIB to find synced lyrics.">
    <Badge kind={s.spotify.lyrics ? "whenOn" : "local"} />
  </Row>
  <Row
    icon="calendar"
    label="Google Calendar"
    help="Only once you connect it: Atoll reads your calendar list and the next days' events from Google (read-only) every 5 minutes. The sign-in and client secret are kept in Windows Credential Manager."
  >
    <Badge kind="when" />
  </Row>
  <Row icon="agent" label="AI agents" help="Claude Code's hooks talk to Atoll on 127.0.0.1 only, with a random token. Nothing is sent on.">
    <Badge kind="local" />
  </Row>
  <Row
    icon="clock"
    label="Claude usage"
    help="With the usage switch on, Atoll asks Anthropic for your usage every 3 minutes using Claude Code's sign-in, and renews that sign-in with Anthropic when it runs out. Nothing else is sent."
  >
    <Badge kind={s.usage.fromSignIn && s.modules.usageRing ? "whenOn" : "local"} />
  </Row>
  <Row icon="download" label="Update checks" help="Atoll never checks online for new versions and sends no usage data.">
    <Badge kind="local" />
  </Row>
  <Row icon="clipboard" label="Clipboard, notes and whiteboard" help={s.clipboard.encrypt ? "Kept on this PC, clipboard history encrypted for your Windows account." : "Kept on this PC only."}>
    <Badge kind="local" />
  </Row>
  <Row
    icon="focus"
    label="Focus timer"
    help="Your focus history stays on this PC. The distraction nudge only checks which app is in front, here on this PC."
  >
    <Badge kind="local" />
  </Row>
</Section>

<Section title="Your data">
  <Row icon="download" label="Export settings" help="Saves your settings to a file. No sign-ins or clipboard items are included.">
    <GButton label="Export" onclick={() => viaFile(settingsApi.exportSettings, "Settings exported")} />
  </Row>
  <Row icon="upload" label="Import settings" help="Replaces your current settings with the ones in a file.">
    <GButton label="Import" onclick={() => viaFile(settingsApi.importSettings, "Settings imported")} />
  </Row>
  <Row icon="trash" label="Clear all data" help="Removes settings, clipboard history, notes and connections.">
    <GButton label="Clear all data" icon="trash" iconSize={16} destructive onclick={clearAll} />
  </Row>
</Section>
