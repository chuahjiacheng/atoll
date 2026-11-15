<!--
  Agent tab → History (flight recorder). Owner: recorder-forecast.
  Left: past sessions, or search results while searching.
  Right: where the selected session's time went, its subagents, a timeline
  you can scrub (drag, or arrow keys), and the event under the handle,
  with a line diff for file edits.
  Reads only when shown; refreshes on `recorder:changed` (no polling).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../ui/Icon.svelte";
  import {
    recorder,
    span,
    bytes,
    dayTime,
    clockSeconds,
    lineDiff,
    markKind,
    kindName,
    type SessionRow,
    type EventRow,
    type EventDetail,
    type Breakdown,
    type AgentNode,
    type StoreStats,
  } from "../recorder";

  let { now, onBack }: { now: number; onBack: () => void } = $props();

  let sessions = $state<SessionRow[]>([]);
  let stats = $state<StoreStats | null>(null);
  let loading = $state(true);
  let error = $state("");
  let selected = $state<string | null>(null);
  let events = $state<EventRow[]>([]);
  let index = $state(0);
  let detail = $state<EventDetail | null>(null);
  let brk = $state<Breakdown | null>(null);
  let agents = $state<AgentNode[]>([]);
  let query = $state("");
  let results = $state<EventRow[] | null>(null);

  const current = $derived(sessions.find((s) => s.sessionId === selected) ?? null);
  const ev = $derived(events[index] ?? null);
  const projectOf = $derived(new Map(sessions.map((s) => [s.sessionId, s.project || "Claude Code"])));

  async function loadSessions() {
    try {
      const [s, st] = await Promise.all([recorder.sessions(undefined, 100), recorder.stats()]);
      sessions = s;
      stats = st;
      error = "";
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
    if (!selected && sessions[0]) select(sessions[0].sessionId);
  }

  async function loadSession(id: string) {
    return Promise.all([recorder.events(id), recorder.breakdown(id), recorder.agentTree(id)]);
  }

  async function select(id: string, eventId?: number) {
    selected = id;
    try {
      const [evs, b, a] = await loadSession(id);
      if (selected !== id) return;
      events = evs;
      brk = b;
      agents = a;
      const at = eventId != null ? evs.findIndex((e) => e.id === eventId) : -1;
      index = at >= 0 ? at : Math.max(0, evs.length - 1);
    } catch (e) {
      error = String(e);
    }
  }

  // The event under the handle: fetched shortly after scrubbing settles.
  $effect(() => {
    const e = events[index];
    if (!e) {
      detail = null;
      return;
    }
    const id = e.id;
    const t = setTimeout(async () => {
      const d = await recorder.event(id).catch(() => null);
      if (events[index]?.id === id) detail = d;
    }, 60);
    return () => clearTimeout(t);
  });

  // Search as you type (quoted words, prefix match; see event-store.md).
  $effect(() => {
    const q = query.trim();
    if (!q) {
      results = null;
      return;
    }
    const t = setTimeout(async () => {
      try {
        results = await recorder.search(q, 100);
      } catch (e) {
        error = String(e);
      }
    }, 200);
    return () => clearTimeout(t);
  });

  onMount(() => {
    loadSessions();
    let gone = false;
    let unlisten: (() => void) | undefined;
    recorder
      .onChanged(async (sid) => {
        loadSessions();
        if (sid !== selected) return;
        const atEnd = index >= events.length - 1;
        try {
          const [evs, b, a] = await loadSession(sid);
          if (selected !== sid) return;
          events = evs;
          brk = b;
          agents = a;
          if (atEnd) index = Math.max(0, evs.length - 1);
        } catch {
          /* keep what is shown */
        }
      })
      .then((u) => (gone ? u() : (unlisten = u)));
    return () => {
      gone = true;
      unlisten?.();
    };
  });

  // ---- breakdown bar ----
  const segments = $derived.by(() => {
    if (!brk || brk.total <= 0) return [];
    const tools = brk.tools.slice(0, 3).map(([name, ms], i) => ({ key: `t-${name}`, name, ms, cls: `tool t${i}` }));
    const rest = brk.tools.slice(3).reduce((sum, [, ms]) => sum + ms, 0);
    return [
      { key: "think", name: "Thinking", ms: brk.thinking, cls: "think" },
      ...tools,
      ...(rest > 0 ? [{ key: "t-rest", name: "Other tools", ms: rest, cls: "tool t3" }] : []),
      { key: "wait", name: "Waiting for you", ms: brk.waitingForYou, cls: "wait" },
      { key: "idle", name: "Idle", ms: brk.idle, cls: "idle" },
    ].filter((s) => s.ms > 0);
  });

  // ---- timeline marks: one path per kind, one unit per event ----
  const marks = $derived.by(() => {
    const paths: Record<string, string> = {};
    events.forEach((e, i) => {
      const k = markKind(e.kind);
      paths[k] = (paths[k] ?? "") + `M${i} 0h1v1h-1z`;
    });
    return Object.entries(paths);
  });

  function jumpToAgent(agentId: string) {
    const at = events.findIndex((e) => e.agentId === agentId);
    if (at >= 0) index = at;
  }

  function origin(s: SessionRow) {
    return s.entrypoint === "desktop" ? "Desktop app" : s.entrypoint === "cli" ? "Terminal" : "";
  }
</script>

<div class="history clay">
  <div class="bar">
    <button class="btn secondary small back" onclick={onBack} aria-label="Back to live activity">
      <Icon name="agent" size={16} />Live
    </button>
    <label class="search">
      <Icon name="search" size={16} />
      <input class="field" type="search" placeholder="Search history" aria-label="Search history" bind:value={query} spellcheck="false" />
    </label>
    {#if stats}
      <span class="label tabular meta" title="Saved on this PC only, with secrets removed">
        {stats.events.toLocaleString("en-GB")} events · {bytes(stats.fileBytes)}
      </span>
    {/if}
  </div>

  {#if error}
    <div class="notice" role="alert"><Icon name="alert" size={16} /><span>History couldn't be read. {error}</span></div>
  {/if}

  {#if !loading && sessions.length === 0 && !error}
    <div class="empty-state">
      <Icon name="clock" size={32} />
      <div class="title">No history yet</div>
      <div class="hint">Claude Code events are saved here, on this PC, as they arrive. Turn history on or off in Settings → AI agents.</div>
    </div>
  {:else}
    <div class="body">
      <div class="list" data-lenis-prevent aria-label={results ? "Search results" : "Sessions"}>
        {#if results}
          <div class="label list-head tabular">{results.length === 100 ? "100+ matches" : `${results.length} ${results.length === 1 ? "match" : "matches"}`}</div>
          {#each results as r (r.id)}
            <button class="row" aria-current={ev?.id === r.id} onclick={() => select(r.sessionId, r.id)}>
              <span class="ellipsis">{r.summary}</span>
              <span class="label ellipsis tabular">{projectOf.get(r.sessionId) ?? "Claude Code"} · {dayTime(r.ts, now)}</span>
            </button>
          {:else}
            <div class="label none">Nothing matches “{query.trim()}”.</div>
          {/each}
        {:else}
          {#each sessions as s (s.sessionId)}
            <button class="row" aria-current={s.sessionId === selected} onclick={() => select(s.sessionId)}>
              <span class="ellipsis strong">{s.project || "Claude Code"}{#if origin(s)}<span class="label"> · {origin(s)}</span>{/if}</span>
              <span class="label ellipsis tabular">{dayTime(s.startedAt, now)} · {span(s.lastEventAt - s.startedAt)} · {s.eventCount} events</span>
            </button>
          {/each}
        {/if}
      </div>

      {#if current}
        <div class="detail">
          {#if segments.length}
            <div class="split" role="img" aria-label={`Time: ${segments.map((s) => `${s.name} ${span(s.ms)}`).join(", ")}`}>
              {#each segments as s (s.key)}<span class="seg {s.cls}" style:flex-grow={s.ms}></span>{/each}
            </div>
            <div class="legend label tabular" aria-hidden="true">
              {#each segments as s (s.key)}<span><i class="seg {s.cls}"></i>{s.name} {span(s.ms)}</span>{/each}
            </div>
          {/if}

          {#if agents.length}
            <div class="agents">
              <span class="label strong">Subagents</span>
              {#each agents as a (a.agentId)}
                <button class="chip" onclick={() => jumpToAgent(a.agentId)} title="Jump to its first event">
                  {a.agentType ?? "Subagent"} <span class="label tabular">{a.eventCount}{a.endedAt ? ` · ${span(a.endedAt - a.startedAt)}` : " · running"}</span>
                </button>
              {/each}
            </div>
          {/if}

          {#if events.length}
            <div class="timeline">
              <svg class="marks" viewBox="0 0 {events.length} 1" preserveAspectRatio="none" aria-hidden="true">
                {#each marks as [k, d] (k)}<path class="m-{k}" {d} />{/each}
              </svg>
              <input
                type="range"
                min="0"
                max={events.length - 1}
                step="1"
                bind:value={index}
                aria-label="Timeline"
                aria-valuetext={ev ? `${kindName(ev.kind)} at ${clockSeconds(ev.ts)}, event ${index + 1} of ${events.length}` : ""}
              />
            </div>
            <div class="where label tabular">
              <span>{ev ? clockSeconds(ev.ts) : ""}</span>
              <span>Event {index + 1} of {events.length}</span>
            </div>
          {/if}

          <div class="event" data-lenis-prevent>
            {#if ev}
              <div class="label">
                <span class="strong kind k-{markKind(ev.kind)}">{kindName(ev.kind)}</span>{ev.tool ? ` · ${ev.tool}` : ""}{#if detail?.agentType || ev.agentId} · subagent {detail?.agentType ?? ""}{/if}
              </div>
              <p class="summary">{ev.summary}</p>
              {#if detail?.id === ev.id && detail.diff}
                {#each detail.diff as h, hi (hi)}
                  <div class="label path ellipsis" title={h.path}>{h.path}</div>
                  <div class="diff" role="group" aria-label="Changes to {h.path}">
                    {#each lineDiff(h.old, h.new) as l, li (li)}
                      <div class="dl op{l.op === '+' ? 'add' : l.op === '-' ? 'del' : 'same'}">
                        <span class="sign" aria-label={l.op === "+" ? "added" : l.op === "-" ? "removed" : undefined}>{l.op === "+" ? "+" : l.op === "-" ? "−" : ""}</span><span class="txt">{l.text}</span>
                      </div>
                    {/each}
                  </div>
                  {#if h.truncated}<div class="label">Cut short: Claude Code passes at most 4,000 characters per field.</div>{/if}
                {/each}
              {:else if detail?.id === ev.id && detail.detail && detail.detail !== ev.summary}
                <p class="text">{detail.detail}</p>
              {/if}
            {:else}
              <span class="label">This session has no events.</span>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .history {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
    --hv-ask: var(--preset-accent-peri);
    --hv-tool: color-mix(in srgb, var(--c-ink) 45%, transparent);
  }
  .clay {
    --c-accent: var(--clay);
    --c-accent-text: var(--clay);
    --c-accent-ink: var(--clay-ink);
  }
  .bar { display: flex; align-items: center; gap: var(--sp-8); flex: none; }
  .back { display: inline-flex; align-items: center; gap: var(--sp-4); }
  .search { position: relative; flex: 0 1 280px; display: flex; align-items: center; color: var(--c-muted); }
  .search :global(svg) { position: absolute; left: var(--sp-12); pointer-events: none; }
  .search .field { width: 100%; padding-left: calc(var(--sp-12) + 16px + var(--sp-8)); }
  .search .field::placeholder { color: var(--c-muted); }
  .search .field::-webkit-search-cancel-button { filter: grayscale(1); }
  .meta { margin-left: auto; white-space: nowrap; }
  .notice {
    display: flex; align-items: center; gap: var(--sp-8); flex: none;
    padding: var(--sp-8) var(--sp-12); border-radius: var(--r-control);
    background: var(--c-surface-card); color: var(--c-error); font-size: var(--fs-small);
  }
  .empty-state :global(svg) { color: var(--clay); }

  .body { flex: 1; min-height: 0; display: flex; gap: var(--sp-16); }
  .list {
    width: var(--size-agent-side); flex: none;
    display: flex; flex-direction: column; gap: var(--sp-2);
    padding: var(--sp-4); border-radius: var(--r-control);
    background: var(--c-surface-card);
    background-image: linear-gradient(var(--clay-soft), var(--clay-soft));
    overflow-y: auto; min-height: 0;
  }
  .list-head, .none { padding: var(--sp-8) var(--sp-8) var(--sp-4); }
  .row {
    display: flex; flex-direction: column; align-items: stretch; gap: var(--sp-2);
    padding: var(--sp-8); border-radius: var(--r-tile, 10px);
    text-align: left; font-size: var(--fs-small); color: var(--c-ink); min-width: 0;
    transition: background-color var(--dur-fast);
  }
  .row:hover { background: var(--c-hover); }
  .row[aria-current="true"] { background: var(--c-tab); }
  .row:focus-visible, .chip:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: -2px; }
  .strong { font-weight: var(--fw-strong); }

  .detail { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-8); min-height: 0; }
  .split { display: flex; gap: var(--sp-2); height: 8px; flex: none; border-radius: var(--r-pill); overflow: hidden; }
  .seg { display: block; min-width: 2px; }
  .seg.think { background: var(--clay); }
  .seg.tool.t0 { background: color-mix(in srgb, var(--c-ink) 72%, transparent); }
  .seg.tool.t1 { background: color-mix(in srgb, var(--c-ink) 52%, transparent); }
  .seg.tool.t2 { background: color-mix(in srgb, var(--c-ink) 36%, transparent); }
  .seg.tool.t3 { background: color-mix(in srgb, var(--c-ink) 24%, transparent); }
  .seg.wait { background: var(--hv-ask); }
  .seg.idle { background: var(--c-track); }
  .legend { display: flex; flex-wrap: wrap; gap: var(--sp-4) var(--sp-12); flex: none; }
  .legend span { display: inline-flex; align-items: center; gap: var(--sp-4); white-space: nowrap; }
  .legend i { width: 8px; height: 8px; border-radius: var(--r-pill); min-width: 0; }

  .agents { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-4) var(--sp-8); flex: none; }
  .chip {
    height: 28px; padding: 0 var(--sp-12); border-radius: var(--r-pill);
    background: var(--c-surface-card); color: var(--c-ink);
    font-size: var(--fs-small); font-weight: var(--fw-strong);
    transition: background-color var(--dur-fast);
  }
  .chip:hover { background: var(--c-hover); }
  .chip .label { font-weight: var(--fw-regular, 400); }

  .timeline { position: relative; height: 28px; flex: none; }
  .marks {
    position: absolute; inset: 6px 0; width: 100%; height: 16px;
    border-radius: var(--r-keycap, 6px); background: var(--c-surface-card);
  }
  .m-prompt { fill: var(--clay); }
  .m-tool { fill: var(--hv-tool); }
  .m-ask { fill: var(--hv-ask); }
  .m-stop { fill: var(--c-ink); }
  .m-fail { fill: var(--c-error); }
  .m-other { fill: var(--c-track); }
  .timeline input {
    position: absolute; inset: 0; width: 100%; height: 100%; margin: 0;
    appearance: none; background: transparent; cursor: ew-resize;
  }
  .timeline input::-webkit-slider-runnable-track { height: 100%; background: transparent; }
  .timeline input::-webkit-slider-thumb {
    appearance: none; width: 4px; height: 28px; border-radius: var(--r-pill);
    background: var(--c-ink); box-shadow: 0 0 0 2px var(--c-hull), 0 2px 6px rgba(0, 0, 0, 0.35);
  }
  .timeline input:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: 2px; border-radius: var(--r-keycap, 6px); }
  .where { display: flex; justify-content: space-between; flex: none; margin-top: calc(-1 * var(--sp-4)); }

  .event {
    flex: 1; min-height: 0; overflow-y: auto;
    padding: var(--sp-12) var(--sp-16); border-radius: var(--r-control);
    background: var(--c-surface-card);
    background-image: linear-gradient(var(--clay-soft), var(--clay-soft));
    font-size: var(--fs-small); user-select: text;
  }
  .kind.k-prompt { color: var(--clay); }
  .kind.k-ask { color: var(--hv-ask); }
  .kind.k-fail { color: var(--c-error); }
  .kind.k-tool, .kind.k-stop, .kind.k-other { color: var(--c-ink); }
  .summary { margin: var(--sp-4) 0 var(--sp-8); font-weight: var(--fw-strong); overflow-wrap: anywhere; }
  .text { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--c-muted); line-height: var(--lh-loose); }
  .path { margin-top: var(--sp-8); }
  .diff {
    margin: var(--sp-4) 0; border-radius: var(--r-tile, 10px); overflow: hidden;
    font-size: var(--fs-label); font-variant-numeric: tabular-nums; line-height: 1.5;
  }
  .dl { display: flex; min-width: 0; }
  .sign { flex: none; width: var(--sp-24); text-align: center; color: var(--c-muted); user-select: none; }
  .txt { flex: 1; white-space: pre-wrap; overflow-wrap: anywhere; padding-right: var(--sp-8); }
  .dl.opadd { background: color-mix(in srgb, var(--hv-ask) 16%, transparent); }
  .dl.opadd .sign { color: var(--c-ink); font-weight: var(--fw-strong); }
  .dl.opdel { background: var(--c-surface-card); color: var(--c-muted); }
  .dl.opdel .txt { text-decoration: line-through; text-decoration-color: color-mix(in srgb, var(--c-muted) 60%, transparent); }
</style>
