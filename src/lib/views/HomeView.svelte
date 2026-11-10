<!--
  Home tab (design: Island → home): four cards, each opening its module.
  Agent · Now playing · Focus or Usage · Last copy. The third card is the
  design's Focus card while a session or break is on, or when there is no
  usage to show; otherwise it shows your Claude usage.
-->
<script lang="ts">
  import type { Session, Usage } from "../api";
  import { ago, clearable, summary } from "../format";
  import StatusChip from "../ui/StatusChip.svelte";
  import { focusActive, focusClockMs, focusFraction, focusTotal, timerText, type FocusView } from "../focus";
  import { positionMs, type Player } from "../spotify";
  import type { Clip } from "../tools.svelte";
  import Icon from "../ui/Icon.svelte";
  import Ring from "../ui/Ring.svelte";

  let {
    session,
    sessionCount,
    player,
    connected,
    usage,
    lastClip,
    focus = null,
    now,
    open,
    live = true,
    onClear,
    clearKey = "",
  }: {
    /** Clear the session's error / needs you / done message (× on its status tag). */
    onClear?: (id: string) => void;
    /** The Clear hotkey, for the × tooltip. */
    clearKey?: string;
    /** The focus timer (null when its module is off). */
    focus?: FocusView | null;
    /** Show live activity (Settings → AI agents). */
    live?: boolean;
    session: Session | null;
    sessionCount: number;
    player: Player | null;
    connected: boolean;
    usage: Usage | null;
    lastClip: Clip | null;
    now: number;
    open: (tab: string) => void;
  } = $props();

  const track = $derived(player?.track ?? null);
  const fraction = $derived(track && player && track.durationMs ? positionMs(player, now) / track.durationMs : 0);
  const canClear = $derived(!!onClear && clearable(session?.status));
  // A window whose reset time has passed no longer means anything.
  const five = $derived(
    usage?.fiveHour && (usage.fiveHour.resetsAt === 0 || usage.fiveHour.resetsAt * 1000 > now) ? usage.fiveHour : null,
  );

  const showFocus = $derived(!!focus && (focusActive(focus) || !five));
  const focusWord = $derived.by(() => {
    if (!focus || focus.phase === "ready") return "Ready";
    if (focus.phase === "break") return focus.running ? (focus.longBreak ? "Long break" : "Break") : "Break paused";
    return focus.running ? `Focus ${focus.round} of ${focus.rounds}` : "Paused";
  });

  function resets(at: number) {
    const mins = Math.max(0, Math.round((at * 1000 - now) / 60000));
    return mins >= 60 ? `Resets in ${Math.floor(mins / 60)} h ${mins % 60} m` : `Resets in ${mins} m`;
  }

  function clipTitle(c: Clip) {
    if (c.kind === "image") return `Image ${c.width} × ${c.height}`;
    if (c.kind === "files") return c.files.map((f) => f.split("\\").pop()).join(", ");
    return c.text ?? "";
  }
</script>

<div class="grid">
  <div class="cell">
  <button class="card" onclick={() => open("claude")}>
    <div class="row label"><span>Agent</span>{#if session && !canClear}<StatusChip status={session.status} />{/if}</div>
    <div class="headline ellipsis">{session ? summary(session, live) : "No session yet"}</div>
    <span class="flex"></span>
    <div class="label">
      {#if session}Started {ago(session.startedAt, now)} · {session.toolCount} {session.toolCount === 1 ? "action" : "actions"}{sessionCount > 1 ? ` · ${sessionCount} sessions` : ""}{:else}Start Claude Code to see it here{/if}
    </div>
  </button>
  {#if session && canClear}
    <!-- Over the card (a button can't hold another button); only the × takes clicks. -->
    <span class="chip-over"><StatusChip status={session.status} onClear={() => onClear?.(session.id)} hint={clearKey} /></span>
  {/if}
  </div>

  <button class="card" onclick={() => open("spotify")}>
    <div class="label">Now playing · Spotify</div>
    {#if track}
      <div class="np">
        {#if track.image}<img class="art" src={track.image} alt="" />{:else}<span class="art blank"><Icon name="music" size={20} /></span>{/if}
        <div class="meta"><div class="headline ellipsis">{track.name}</div><div class="sub ellipsis">{track.artists}</div></div>
      </div>
      <span class="flex"></span>
      <span class="track" aria-hidden="true"><span style="transform: scaleX({Math.min(1, fraction)})"></span></span>
    {:else}
      <div class="headline">{connected ? "Nothing playing" : "Not connected"}</div>
      <span class="flex"></span>
      <div class="label">{connected ? "Play something in the Spotify app" : "Open this card to connect"}</div>
    {/if}
  </button>

  {#if showFocus && focus}
    {@const timed = focus.lengthMs > 0}
    <button class="card usage" onclick={() => open("focus")}>
      <Ring
        value={focus.phase === "ready" ? 0 : timed ? 1 - focusFraction(focus, now) : 0}
        size={56}
        stroke={6}
        tone={focus.phase === "break" ? "done" : "accent"}
        label="Focus timer, {focusWord}"
      />
      <div>
        <div class="big tabular">{timerText(focusClockMs(focus, now), !timed)}</div>
        <div class="label">{focusWord}{focus.stats.todayMs ? ` · Today ${focusTotal(focus.stats.todayMs)}` : ""}</div>
      </div>
    </button>
  {:else}
    <button class="card usage" onclick={() => open("claude")}>
      <Ring value={five ? five.used / 100 : 0} size={56} stroke={6} label={five ? `${Math.round(five.used)} percent of the 5-hour limit used` : "Usage not available"} dashed={!five} />
      <div>
        <div class="big tabular">{five ? `${Math.round(five.used)}%` : "Usage"}</div>
        <div class="label">{five ? `5-hour limit · ${resets(five.resetsAt)}` : "Appears once Claude Code runs"}</div>
      </div>
    </button>
  {/if}

  <button class="card" onclick={() => open("clipboard")}>
    <div class="label">Last copy</div>
    {#if lastClip}
      <div class="copy"><span class="label"><Icon name={lastClip.kind === "image" ? "image" : lastClip.kind === "files" ? "file" : "text"} size={16} /></span><span class="ellipsis">{clipTitle(lastClip)}</span></div>
      <span class="flex"></span>
      <div class="label">{lastClip.kind === "image" ? "Image" : lastClip.kind === "files" ? "File" : "Text"} · {ago(lastClip.at, now)}</div>
    {:else}
      <div class="headline">Nothing copied yet</div>
      <span class="flex"></span>
      <div class="label">Copies appear here</div>
    {/if}
  </button>
</div>

<style>
  .grid {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-template-rows: repeat(2, minmax(0, 1fr));
    gap: var(--sp-8);
  }
  .cell { position: relative; display: flex; min-width: 0; min-height: 0; }
  .cell > .card { flex: 1; }
  /* Sits where the card's status would be: top right, centred on the "Agent" line. */
  .chip-over {
    position: absolute;
    top: calc(var(--sp-16) + var(--fs-label) * 0.6);
    right: var(--sp-16);
    transform: translateY(-50%);
    pointer-events: none;
  }
  .card { display: flex; flex-direction: column; gap: var(--sp-8); text-align: left; }
  .card:hover { background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .row { display: flex; align-items: center; justify-content: space-between; }
  .headline { font-size: var(--fs-body); font-weight: var(--fw-strong); }
  .sub { font-size: var(--fs-small); color: var(--c-muted); }
  .flex { flex: 1; }
  .np { display: flex; gap: var(--sp-16); align-items: center; min-width: 0; }
  .meta { min-width: 0; }
  .art { width: var(--size-art-sm); height: var(--size-art-sm); border-radius: var(--r-control); flex: none; object-fit: cover; }
  .art.blank { display: grid; place-items: center; background: var(--c-raised); color: var(--c-muted); }
  .track { height: var(--sp-4); border-radius: var(--r-pill); background: var(--c-track); overflow: hidden; }
  .track span { display: block; height: 100%; background: var(--c-accent); transform-origin: left; }
  .usage { flex-direction: row; align-items: center; gap: var(--sp-16); }
  .big { font-size: var(--fs-headline); font-weight: var(--fw-strong); }
  .copy { display: flex; align-items: center; gap: var(--sp-8); font-size: var(--fs-small); min-width: 0; }
  .copy .label { display: flex; }
</style>
