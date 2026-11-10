<!--
  Spotify tab. Track and controls on top, the seek bar, then synced lyrics
  filling the rest: the current line is highlighted and kept in the middle,
  lines already sung fade back. Devices live in a menu next to the controls.
  Playback comes from the Spotify app on this PC (through Windows, no
  sign-in); the device list needs the optional linked account.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { Menu, Slider } from "glisse";
  import { settingsApi } from "../settings";
  import { clock, positionMs, spotifyApi, type Device, type Lyrics, type Player, type SpotifyStatus } from "../spotify";
  import { dur, ease, gsap, reduced, seconds } from "../motion";
  import GButton from "../glass/GButton.svelte";
  import { icon as ricon, react } from "../glass/bridge";
  import type { IconName } from "../ui/Icon.svelte";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let {
    player,
    status,
    lyrics,
    lyricsOn,
    now,
    tone,
  }: { player: Player | null; status: SpotifyStatus | null; lyrics: Lyrics | null; lyricsOn: boolean; now: number; tone: Tone } = $props();

  let error = $state("");
  let devices = $state<Device[]>([]);
  let lyricsBox = $state<HTMLDivElement>();
  let dragging = $state<number | null>(null);

  const track = $derived(player?.track ?? null);
  const pos = $derived(dragging ?? (player ? positionMs(player, now) : 0));
  const trackLyrics = $derived(lyrics && track && lyrics.trackId === track.id ? lyrics : null);
  const current = $derived.by(() => {
    const lines = trackLyrics?.synced ?? [];
    let i = -1;
    for (let k = 0; k < lines.length && lines[k].t <= pos; k++) i = k;
    return i;
  });

  const account = $derived(!!status?.account);
  const canSeek = $derived(!!player?.canSeek || account);

  async function loadDevices() {
    try {
      devices = await spotifyApi.devices();
    } catch {
      devices = [];
    }
  }
  // The device list is the only thing that asks Spotify's servers, so it is
  // fetched once per visit to this tab, and only with a linked account.
  onMount(() => {
    if (status?.account) loadDevices();
  });

  async function attempt(job: () => Promise<unknown>) {
    error = "";
    try {
      await job();
    } catch (e) {
      error = String(e);
    }
  }

  // Keep the current line centred.
  $effect(() => {
    if (current < 0 || !lyricsBox) return;
    const el = lyricsBox.querySelector<HTMLElement>(`[data-i="${current}"]`);
    if (!el) return;
    const top = el.offsetTop - lyricsBox.clientHeight / 2 + el.clientHeight / 2;
    if (reduced()) lyricsBox.scrollTop = top;
    else gsap.to(lyricsBox, { scrollTop: top, duration: dur("slow") * 1.5, ease: ease(), overwrite: "auto" });
  });

  async function run(action: Parameters<typeof spotifyApi.action>[0], value?: number | string) {
    error = "";
    try {
      await spotifyApi.action(action, value);
    } catch (e) {
      error = String(e);
    }
  }

  // Seek: dragging previews the time; Spotify is told once the slider rests.
  let seekTimer: ReturnType<typeof setTimeout> | undefined;
  function seekChange(ms: number) {
    dragging = ms;
    clearTimeout(seekTimer);
    seekTimer = setTimeout(() => {
      if (dragging !== null) run("seek", dragging);
      dragging = null;
    }, seconds("--dur-seek-commit") * 1000);
  }

  function deviceIcon(d: Device): IconName {
    const k = d.kind.toLowerCase();
    return k.includes("smartphone") || k.includes("phone") ? "phone" : k.includes("computer") ? "laptop" : "speaker";
  }

  async function pick(d: Device) {
    if (d.id && !d.isActive && !d.isRestricted) {
      await run("transfer", d.id);
      setTimeout(loadDevices, 800);
    }
  }

  const deviceItems = $derived(
    !account
      ? [
          { label: "Playing on this PC", icon: ricon("check", 16), disabled: true },
          { label: "Switch devices… (link account)", icon: ricon("settings", 16), onSelect: () => settingsApi.openDashboard() },
        ]
      : devices.length
        ? devices.map((d) => ({
            label: d.isActive ? `${d.name} · playing` : d.name,
            icon: ricon(d.isActive ? "check" : deviceIcon(d), 16),
            disabled: d.isRestricted || !d.id,
            onSelect: () => pick(d),
          }))
        : [{ label: "No devices found", disabled: true }],
  );
</script>

{#snippet deviceMenu()}
  <div
    class="menu"
    use:react={{
      component: Menu,
      props: {
        "aria-label": "Play on device",
        icon: ricon(player?.device ? deviceIcon(player.device) : "speaker", 20),
        align: "end",
        items: deviceItems,
      },
    }}
  ></div>
{/snippet}

{#if !status?.connected}
  <div class="empty-state">
    <Pip mood="idle" {tone} size={64} />
    <div class="title">Spotify isn't connected</div>
    <div class="hint">Show and control what Spotify plays on this PC. No sign-in needed.</div>
    <GButton label="Connect Spotify" variant="filled" onclick={() => attempt(() => spotifyApi.connect(true))} />
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
{:else if !status.appOpen}
  <div class="empty-state">
    <Pip mood="idle" {tone} size={64} />
    <div class="title">Spotify isn't open</div>
    <div class="hint">Open the Spotify app on this PC and play something.</div>
    <GButton label="Open Spotify" icon="external" iconSize={16} onclick={() => attempt(spotifyApi.openApp)} />
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
{:else if !track}
  <div class="empty-state">
    <Pip mood="idle" {tone} size={64} />
    <div class="title">Nothing playing on Spotify</div>
    <div class="hint">Play something in the Spotify app{account ? ", or pick a device here" : ""}.</div>
    {@render deviceMenu()}
  </div>
{:else}
  <div class="music">
    <div class="now">
      {#if track.image}
        <img class="art" src={track.image} alt="Album art for {track.album}" />
      {:else}
        <span class="art blank"><Icon name="music" size={24} /></span>
      {/if}
      <div class="meta">
        <div class="title ellipsis" title={track.name}>{track.name}</div>
        <div class="artist ellipsis" title={track.artists}>{track.artists}</div>
        {#if player?.device}<div class="device ellipsis">on {player.device.name}</div>{/if}
      </div>
      <div class="controls">
        <GButton icon="prev" ariaLabel="Previous" onclick={() => run("previous")} />
        <span class="play">
          <GButton
            icon={player?.isPlaying ? "pause" : "play"}
            variant="filled"
            ariaLabel={player?.isPlaying ? "Pause" : "Play"}
            onclick={() => run(player?.isPlaying ? "pause" : "play")}
          />
        </span>
        <GButton icon="next" ariaLabel="Next" onclick={() => run("next")} />
        {@render deviceMenu()}
      </div>
    </div>

    <div class="seek">
      <span class="time label tabular">{clock(pos)}</span>
      <div
        class="slider"
        use:react={{
          component: Slider,
          props: {
            "aria-label": "Playback position",
            "aria-valuetext": `${clock(pos)} of ${clock(track.durationMs)}`,
            min: 0,
            max: track.durationMs,
            step: 1000,
            disabled: !canSeek,
            value: Math.min(pos, track.durationMs),
            onValueChange: seekChange,
          },
        }}
      ></div>
      <span class="time label tabular">{clock(track.durationMs)}</span>
    </div>

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if lyricsOn}
      <div class="lyrics" bind:this={lyricsBox} data-lenis-prevent aria-label="Lyrics">
        {#if !trackLyrics}
          <p class="hint">Looking for lyrics…</p>
        {:else if trackLyrics.instrumental}
          <p class="hint">Instrumental</p>
        {:else if trackLyrics.synced.length}
          {#each trackLyrics.synced as line, i}
            <p data-i={i} class:current={i === current} class:past={i < current}>{line.text || "♪"}</p>
          {/each}
        {:else if trackLyrics.plain}
          <p class="plain">{trackLyrics.plain}</p>
        {:else}
          <p class="hint">{trackLyrics.state === "error" ? "Lyrics service unavailable right now." : "No lyrics for this track."}</p>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .music { display: flex; flex-direction: column; gap: var(--sp-8); flex: 1; min-height: 0; }
  .now { display: flex; align-items: center; gap: var(--sp-16); min-width: 0; }
  .art {
    width: var(--size-art-sm);
    height: var(--size-art-sm);
    border-radius: var(--r-control);
    flex: none;
    object-fit: cover;
  }
  .art.blank { display: grid; place-items: center; background: var(--c-surface-card); color: var(--c-muted); }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .title { font-size: var(--fs-title); font-weight: var(--fw-strong); }
  .artist { font-size: var(--fs-small); color: var(--c-muted); }
  .device { font-size: var(--fs-label); color: var(--c-muted); }
  .controls { display: flex; align-items: center; gap: var(--sp-8); flex: none; }
  .menu { display: flex; }
  .seek { display: flex; align-items: center; gap: var(--sp-12); }
  .slider { flex: 1; min-width: 0; }
  .time { min-width: var(--sp-32); text-align: center; }
  .error { margin: 0; color: var(--c-error); font-size: var(--fs-small); }
  .lyrics {
    position: relative; /* line offsets are measured from here */
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--sp-24) var(--sp-8);
    mask-image: linear-gradient(transparent, currentColor 16%, currentColor 84%, transparent);
    scrollbar-width: none;
  }
  .lyrics p {
    margin: 0 0 var(--sp-12);
    color: var(--c-muted);
    font-size: var(--fs-title);
    line-height: var(--lh-tight);
    transform-origin: left center;
    transition:
      color calc(var(--dur-slow) / var(--speed)) var(--ease-settle),
      opacity calc(var(--dur-slow) / var(--speed)) var(--ease-settle),
      transform calc(var(--dur-slow) / var(--speed)) var(--ease-settle);
  }
  .lyrics p.past { opacity: 0.5; }
  .lyrics p.current { color: var(--c-ink); font-weight: var(--fw-strong); transform: scale(1.06); }
  .lyrics .plain { white-space: pre-wrap; font-size: var(--fs-body); }
  .lyrics .hint { text-align: center; font-size: var(--fs-small); }
  :global([data-reduce-motion="true"]) .lyrics p { transition: none; }
</style>
