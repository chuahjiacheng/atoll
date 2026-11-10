<script lang="ts">
  import { onMount } from "svelte";
  import Pip from "../../lib/ui/Pip.svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { api } from "../../lib/api";
  import { px } from "../../lib/theme";
  import { settingsApi, type Settings } from "../../lib/settings";
  import { act } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";

  let { s }: { s: Settings } = $props();

  let info = $state<{ version: string; exe: string } | null>(null);
  onMount(async () => (info = await settingsApi.appInfo()));
  const build = $derived(info && /target[\\/]debug/i.test(info.exe) ? "development build" : "release build");
</script>

<Section aria="About Atoll">
  <Row size="huge" label="Atoll" help={info ? `Version ${info.version} · ${build} of ${__BUILD_DATE__}` : ""}>
    {#snippet lead()}
      <span class="pip"><Pip mood="happy" tone={s.mascot.tone} size={px("--size-pip-peek")} /></span>
    {/snippet}
  </Row>
  <Row icon="file" label="Log folder" help="Logs help when something goes wrong. They stay on this PC.">
    <GButton label="Open log folder" icon="external" iconSize={16} onclick={() => act(() => settingsApi.openFolder("logs"), "")} />
  </Row>
  <Row icon="power" label="Quit Atoll" help="Closes the island, the tray icon and this window.">
    <GButton label="Quit" destructive onclick={() => api.quit()} />
  </Row>
</Section>

<style>
  .pip {
    width: var(--size-pip-tile);
    height: var(--size-pip-tile);
    flex: none;
    border-radius: var(--r-card);
    background: var(--c-desk);
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
