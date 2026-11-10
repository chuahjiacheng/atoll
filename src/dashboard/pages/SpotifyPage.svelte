<script lang="ts">
  import { onMount } from "svelte";
  import { Input } from "glisse";
  import { react } from "../../lib/glass/bridge";
  import GButton from "../../lib/glass/GButton.svelte";
  import { onSpotifyStatus, spotifyApi, type SpotifyStatus } from "../../lib/spotify";
  import type { Settings } from "../../lib/settings";
  import { change, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import Badge from "../ui/Badge.svelte";

  let { s }: { s: Settings } = $props();

  let status = $state<SpotifyStatus | null>(null);
  let linking = $state(false);
  let clientId = $state("");
  let portText = $state("");

  $effect(() => {
    clientId = s.spotify.clientId;
  });
  $effect(() => {
    portText = String(s.spotify.callbackPort);
  });

  onMount(() => {
    spotifyApi.status().then((st) => (status = st));
    const un = onSpotifyStatus((st) => {
      if (linking && st.account) toast("Spotify account linked");
      status = st;
      linking = false;
    });
    return () => un.then((f) => f());
  });

  const connected = $derived(s.spotify.connected);
  const appOpen = $derived(!!status?.appOpen);
  const account = $derived(!!status?.account);
  const accountKind = $derived(account ? "connected" : linking ? "connecting" : "off");
  const idBad = $derived(/\s/.test(clientId.trim()) || (clientId.trim() !== "" && !/^[0-9a-zA-Z]+$/.test(clientId.trim())));
  const portN = $derived(Number(portText));
  const portBad = $derived(!Number.isInteger(portN) || portN < 1024 || portN > 65535);
  const redirect = $derived(`http://127.0.0.1:${s.spotify.callbackPort}/spotify/callback`);

  function connect(on: boolean) {
    change("spotify.connected", on);
    toast(on ? "Spotify connected" : "Spotify disconnected");
  }

  async function openApp() {
    try {
      await spotifyApi.openApp();
    } catch (e) {
      toast(String(e), true);
    }
  }

  function saveId(v: string) {
    clientId = v;
    const t = v.trim();
    if (!idBad && t !== s.spotify.clientId) change("spotify.clientId", t);
  }

  async function link() {
    linking = true;
    toast("Opening Spotify sign-in");
    // Let the client ID save land before Rust reads it.
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    try {
      await spotifyApi.linkAccount();
    } catch (e) {
      linking = false;
      toast(String(e), true);
    }
  }

  async function unlink() {
    await spotifyApi.unlinkAccount();
    toast("Spotify account unlinked");
  }

  async function copy() {
    await navigator.clipboard.writeText(redirect);
    toast("Redirect URI copied");
  }
</script>

<Section title="Connection">
  <Row
    label="Spotify on this PC"
    size="big"
    icon="music"
    help={!connected
      ? "One click. Atoll reads what the Spotify app on this PC is playing, through Windows. No sign-in, no developer app, and nothing leaves your PC."
      : appOpen
        ? "Connected. The island shows and controls the Spotify app on this PC."
        : "Connected. Open the Spotify app on this PC and it shows up in the island."}
  >
    <Badge kind={connected ? "connected" : "off"} />
    {#if connected && !appOpen}<GButton label="Open Spotify" icon="external" iconSize={16} onclick={openApp} />{/if}
    {#if connected}
      <GButton label="Disconnect" onclick={() => connect(false)} />
    {:else}
      <GButton label="Connect" variant="filled" onclick={() => connect(true)} />
    {/if}
  </Row>
  <Row icon="info" label="Play, pause, skip and seek work with Spotify Free and Premium." help="Works with the Spotify app from spotify.com or the Microsoft Store. The Spotify website in a browser isn't picked up." />
</Section>

<Section
  title="Other devices (optional)"
  desc="Only needed to move playback to your phone, a speaker or another computer from the island. Windows can't do that, and Spotify allows it only through a free developer app of your own, which needs Premium."
>
  <Row
    label="Linked account"
    help={account
      ? `Linked${status?.user ? ` as ${status.user}` : ""}. The device menu in the island lists your Spotify devices. The sign-in is kept in Windows Credential Manager.`
      : linking
        ? "Finish signing in to Spotify in your browser."
        : "Not linked. Everything else works without it."}
    note={status?.error && !linking ? status.error : ""}
    noteIcon="alert"
    noteTone="err"
  >
    <Badge kind={accountKind} />
    {#if account}
      <GButton label="Unlink" onclick={unlink} />
    {:else if linking}
      <GButton label="Cancel" onclick={() => ((linking = false), toast("Linking cancelled"))} />
    {:else}
      <GButton label="Link account" disabled={!clientId.trim() || idBad} onclick={link} />
    {/if}
  </Row>
  {#if !account}
    <Row
      label="Client ID"
      help="From your app in the Spotify developer dashboard (steps below)."
      note={idBad ? "A client ID is letters and numbers only, with no spaces." : ""}
      noteIcon="alert"
      noteTone="err"
    >
      <span
        class="field"
        class:bad={idBad}
        use:react={{
          component: Input,
          props: {
            value: clientId,
            placeholder: "Paste your client ID",
            spellCheck: false,
            "aria-label": "Client ID",
            "aria-invalid": idBad,
            onValueChange: saveId,
          },
        }}
      ></span>
    </Row>
    <Row label="Redirect URI" help="Add exactly this under Redirect URIs in your Spotify app, port included.">
      <GButton label={redirect} icon="clipboard" iconSize={16} title="Copy" onclick={copy} />
    </Row>
    <Row
      label="Callback port"
      help="Opened on 127.0.0.1 only while you sign in. Change it only if another program uses it, then update the redirect URI."
      note={portBad ? "Use a whole number from 1024 to 65535." : ""}
      noteIcon="alert"
      noteTone="err"
    >
      <input
        class="d-field port"
        type="number"
        min="1024"
        max="65535"
        value={portText}
        aria-label="Callback port"
        aria-invalid={portBad}
        oninput={(e) => {
          portText = e.currentTarget.value;
          if (!portBad && portN !== s.spotify.callbackPort) change("spotify.callbackPort", portN);
        }}
      />
    </Row>
    <Row line={false}>
      {#snippet below()}
        <ol class="steps">
          <li>Open developer.spotify.com/dashboard, sign in and choose Create app. Any name and description.</li>
          <li>Under Redirect URIs, add the redirect URI above and select Add.</li>
          <li>Under the APIs you plan to use, tick Web API, accept the terms and save.</li>
          <li>Open the app's Settings, copy the Client ID (not the secret) and paste it above.</li>
          <li>Select Link account and approve in the browser tab that opens.</li>
        </ol>
      {/snippet}
    </Row>
  {/if}
</Section>

<Section title="In the island">
  <SwitchRow label="Spotify in the island" help="Same as the Spotify switch on Modules." checked={s.modules.spotify} onchange={(v) => change("modules.spotify", v)} />
  <SwitchRow
    label="Show what's playing"
    help="In the collapsed island while music plays (or is paused for under 5 minutes)."
    checked={s.spotify.showInPill}
    disabled={!s.modules.spotify}
    onchange={(v) => change("spotify.showInPill", v)}
  />
  <SegRow
    label="The pill shows"
    help="Lyrics shows the line being sung, and the song name whenever there's no synced line."
    items={[
      { id: "track", label: "Song · artist" },
      { id: "lyrics", label: "Lyrics" },
    ]}
    value={s.spotify.pillText}
    disabled={!s.modules.spotify || !s.spotify.showInPill}
    note={s.spotify.pillText === "lyrics" && !s.spotify.lyrics ? "Turn on Synced lyrics below, or the pill keeps showing the song name." : ""}
    noteIcon="info"
    noteTone="muted"
    onchange={(v) => change("spotify.pillText", v)}
  />
  <SegRow
    label="While Claude Code works"
    help="Who gets the collapsed island when Claude is busy and music is playing. Needs you and errors always show."
    items={[
      { id: "claude", label: "Claude Code" },
      { id: "spotify", label: "Spotify" },
    ]}
    value={s.spotify.pillPriority}
    disabled={!s.modules.spotify || !s.spotify.showInPill}
    onchange={(v) => change("spotify.pillPriority", v)}
  />
  <SwitchRow
    label="Progress line"
    help="A thin line along the bottom of the island."
    checked={s.spotify.progressLine}
    disabled={!s.modules.spotify}
    onchange={(v) => change("spotify.progressLine", v)}
  />
  <SwitchRow
    label="Synced lyrics"
    help="From LRCLIB, a free lyrics database. It gets the song title, artist, album and length, nothing else."
    checked={s.spotify.lyrics}
    disabled={!s.modules.spotify}
    onchange={(v) => change("spotify.lyrics", v)}
  />
</Section>

<style>
  .field {
    display: contents;
  }
  .field :global(.lg-input) {
    width: var(--size-text-field);
    border-radius: var(--r-control);
    background: var(--c-field);
    box-shadow: inset 0 0 0 1px var(--c-ctrl);
  }
  .field.bad :global(.lg-input) {
    box-shadow: inset 0 0 0 var(--focus-width) var(--c-err);
  }
  .port {
    width: var(--size-hex-field);
  }
  .steps {
    margin: 0;
    padding-left: var(--sp-24);
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
</style>
