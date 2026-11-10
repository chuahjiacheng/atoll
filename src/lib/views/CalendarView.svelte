<!--
  Calendar tab: a Liquid Glass month grid beside an agenda.
  The grid sits on a glass plate; a glass lens marks the picked day and
  slides between days. Each day shows up to three dots in its calendars'
  colours. With no day picked the right side lists what's coming up
  (Settings → Calendar → Days to show); picking a day lists that day.
  Events carry their calendar's colour, time, title and place; the event on
  now is marked, past ones fade, and a Join button appears from 10 minutes
  before a call until it ends. Clicking an event opens it in Google Calendar.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { Glass } from "glisse";
  import {
    allTimed,
    calendarApi,
    clockTime,
    dayLabel,
    dayStart,
    monthGrid,
    monthStart,
    untilText,
    type Agenda,
    type CalendarStatus,
    type Timed,
  } from "../calendar";
  import { react } from "../glass/bridge";
  import GButton from "../glass/GButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let {
    status,
    agenda,
    daysAhead,
    showDeclined,
    now,
    tone,
  }: { status: CalendarStatus | null; agenda: Agenda | null; daysAhead: number; showDeclined: boolean; now: number; tone: Tone } = $props();

  let error = $state("");

  // Opening the tab fetches fresh events (the background fetch runs every 5 minutes).
  onMount(() => {
    if (status?.connected) calendarApi.refresh().catch(() => {});
  });

  const DAY = 86_400_000;
  const nextDay = (day: number) => dayStart(day + 36 * 3_600_000);
  const today = $derived(dayStart(now));

  const all = $derived(allTimed(agenda?.events ?? [], showDeclined));
  const on = (day: number) => {
    const next = nextDay(day);
    return all.filter((e) => e.startMs < next && e.endMs > day);
  };

  // ---------- month grid ----------
  /** The picked day, or null for "Upcoming". */
  let picked = $state<number | null>(null);
  let shownMonth = $state(monthStart(Date.now()));
  // Events are fetched a month back and two ahead: browse last month to next.
  const minMonth = $derived(monthStart(now, -1));
  const maxMonth = $derived(monthStart(now, 1));
  const cells = $derived(monthGrid(shownMonth));
  const monthName = $derived(new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" }).format(shownMonth));
  const weekdays = Array.from({ length: 7 }, (_, i) => {
    const d = new Date(2024, 0, 1 + i); // a Monday
    return {
      short: new Intl.DateTimeFormat(undefined, { weekday: "narrow" }).format(d),
      long: new Intl.DateTimeFormat(undefined, { weekday: "long" }).format(d),
    };
  });

  /** Up to three distinct calendar colours per day. */
  const dots = $derived.by(() => {
    const map = new Map<number, string[]>();
    for (const day of cells) {
      const colours = [...new Set(on(day).map((e) => e.color))].slice(0, 3);
      if (colours.length) map.set(day, colours);
    }
    return map;
  });

  const focusDay = $derived(picked ?? today);
  const lensIndex = $derived(cells.indexOf(focusDay));
  const inMonth = (day: number) => new Date(day).getMonth() === new Date(shownMonth).getMonth();
  const canMonth = (m: number) => m >= minMonth && m <= maxMonth;

  function pick(day: number) {
    const m = monthStart(day);
    if (!canMonth(m)) return;
    shownMonth = m;
    picked = day === today ? null : day;
  }

  function goMonth(delta: number) {
    const m = monthStart(shownMonth, delta);
    if (canMonth(m)) shownMonth = m;
  }

  let grid: HTMLDivElement | undefined = $state();
  async function gridKey(e: KeyboardEvent) {
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    let target: number | null = null;
    if (step) target = dayStart(focusDay + step * DAY + 12 * 3_600_000);
    else if (e.key === "Home") target = today;
    else if (e.key === "PageUp" || e.key === "PageDown") {
      const d = new Date(focusDay);
      target = new Date(d.getFullYear(), d.getMonth() + (e.key === "PageUp" ? -1 : 1), Math.min(d.getDate(), 28)).getTime();
    }
    if (target === null) return;
    e.preventDefault();
    pick(target);
    await tick();
    grid?.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus();
  }

  const fullDate = (day: number) =>
    new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" }).format(day);
  function cellLabel(day: number): string {
    const n = on(day).length;
    const parts = [fullDate(day)];
    if (day === today) parts.unshift("Today");
    parts.push(n ? `${n} ${n === 1 ? "event" : "events"}` : "no events");
    return parts.join(", ");
  }

  // ---------- agenda ----------
  /** Upcoming (today on, `daysAhead` days) or the picked day. */
  const days = $derived.by(() => {
    if (picked !== null) return [{ day: picked, items: on(picked) }];
    const out: { day: number; items: Timed[] }[] = [];
    const last = today + daysAhead * DAY;
    for (let day = today; day < last; day = nextDay(day)) {
      const items = on(day);
      if (items.length) out.push({ day, items });
    }
    return out;
  });

  const hasCalendarsOn = $derived(!!status?.calendars.some((c) => c.on));
  const paneTitle = $derived(picked === null ? "Upcoming" : dayLabel(picked, now));

  function whenText(e: Timed, day: number): string {
    if (e.allDay) return "All day";
    if (e.startMs < day) return e.endMs < day + DAY ? `until ${clockTime(e.endMs)}` : "All day";
    return clockTime(e.startMs);
  }

  const isNow = (e: Timed) => !e.allDay && e.startMs <= now && e.endMs > now;
  const isPast = (e: Timed) => e.endMs <= now;
  const canJoin = (e: Timed) => !!e.meet && !e.allDay && e.startMs - now <= 10 * 60_000 && e.endMs > now;

  async function open(url: string | null) {
    if (!url) return;
    error = "";
    try {
      await calendarApi.open(url);
    } catch (e) {
      error = String(e);
    }
  }

  function connect() {
    error = "";
    calendarApi.login().catch((e) => (error = String(e)));
  }

  function refresh() {
    error = "";
    calendarApi.refresh().catch((e) => (error = String(e)));
  }
</script>

{#if !status?.connected}
  <div class="empty-state">
    <Pip mood="idle" {tone} size={64} />
    <div class="title">Google Calendar isn't connected</div>
    <div class="hint">
      {status?.signingIn ? "Finish signing in to Google in your browser." : "See today's events and join calls from the island. Connect opens Google in your browser."}
    </div>
    {#if status?.signingIn}
      <GButton label="Cancel" onclick={() => calendarApi.cancelLogin()} />
    {:else}
      <GButton label="Connect Google Calendar" icon="calendar" iconSize={16} variant="filled" onclick={connect} />
    {/if}
    {#if status?.error || error}<p class="error" role="alert">{error || status?.error}</p>{/if}
  </div>
{:else}
  <div class="cal">
    <!-- Month: a glass plate with a sliding glass lens on the picked day. -->
    <div class="month">
      <span class="plate-host" aria-hidden="true" use:react={{ component: Glass, props: { className: "cal-plate" } }}></span>
      <div class="month-head">
        <span class="month-name strong" aria-live="polite">{monthName}</span>
        <GButton icon="chevronLeft" iconSize={16} ariaLabel="Previous month" title="Previous month" disabled={!canMonth(monthStart(shownMonth, -1))} onclick={() => goMonth(-1)} />
        <GButton icon="chevronRight" iconSize={16} ariaLabel="Next month" title="Next month" disabled={!canMonth(monthStart(shownMonth, 1))} onclick={() => goMonth(1)} />
      </div>
      <div class="weekdays" aria-hidden="true">
        {#each weekdays as w (w.long)}<span title={w.long}>{w.short}</span>{/each}
      </div>
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div class="grid" role="group" aria-label={`${monthName}. Arrow keys move between days.`} bind:this={grid} onkeydown={gridKey}>
        {#if lensIndex >= 0}
          <span
            class="lens"
            class:today={focusDay === today}
            style:--col={lensIndex % 7}
            style:--row={Math.floor(lensIndex / 7)}
            aria-hidden="true"
          ></span>
        {/if}
        {#each cells as day (day)}
          <button
            class="day"
            class:out={!inMonth(day)}
            class:past={day < today}
            class:is-today={day === today}
            class:sel={day === focusDay}
            tabindex={day === focusDay ? 0 : -1}
            aria-pressed={day === focusDay}
            aria-label={cellLabel(day)}
            disabled={!canMonth(monthStart(day))}
            onclick={() => pick(day)}
          >
            <span class="num tabular">{new Date(day).getDate()}</span>
            <span class="dots" aria-hidden="true">
              {#each dots.get(day) ?? [] as c (c)}<span class="dotc" style:background={c}></span>{/each}
            </span>
          </button>
        {/each}
      </div>
    </div>

    <!-- Agenda for the picked day, or what's coming up. -->
    <div class="pane">
      <div class="pane-head">
        <h3 class="pane-title strong">{paneTitle}</h3>
        {#if picked !== null}
          <GButton label="Today" iconSize={16} onclick={() => pick(today)} />
        {:else if agenda?.fetchedAt}
          <GButton icon="reset" iconSize={16} ariaLabel="Refresh" title="Refresh" onclick={refresh} />
        {/if}
      </div>
      {#if status.error || error}<p class="error" role="alert">{error || status.error}</p>{/if}

      {#if !agenda?.fetchedAt}
        <div class="pane-empty"><Pip mood="focused" {tone} size={40} /><span>Fetching events from Google…</span></div>
      {:else if !hasCalendarsOn}
        <div class="pane-empty"><Pip mood="idle" {tone} size={40} /><span>Every calendar is switched off. Pick the ones to show in Settings → Calendar.</span></div>
      {:else if !days.length || !days.some((d) => d.items.length)}
        <div class="pane-empty">
          <Pip mood="idle" {tone} size={40} />
          <span>{picked !== null ? "Nothing on this day." : `No events ${daysAhead === 1 ? "today" : `in the next ${daysAhead} days`}.`}</span>
        </div>
      {:else}
        <div class="list" data-lenis-prevent aria-label={picked === null ? "Upcoming events" : `Events on ${fullDate(picked)}`}>
          {#each days as d (d.day)}
            <section aria-label={dayLabel(d.day, now)}>
              {#if picked === null}<h4 class="day-h label strong">{dayLabel(d.day, now)}</h4>{/if}
              {#each d.items as e (e.calendarId + e.id + d.day)}
                <div class="event" class:now={isNow(e)} class:past={isPast(e)} class:declined={e.declined}>
                  <button class="main" onclick={() => open(e.link)} title="Open in Google Calendar" disabled={!e.link}>
                    <span class="bar" style:background={e.color}></span>
                    <span class="when tabular">{whenText(e, d.day)}</span>
                    <span class="what">
                      <span class="name ellipsis">{e.title}</span>
                      {#if e.location}
                        <span class="sub ellipsis"><Icon name="pinmap" size={12} />{e.location}</span>
                      {:else if !e.allDay && e.startMs > now && e.startMs - now < 12 * 3_600_000}
                        <span class="sub">{untilText(e.startMs, now)}</span>
                      {/if}
                    </span>
                    {#if isNow(e)}<span class="tag">Now</span>{/if}
                  </button>
                  {#if canJoin(e)}
                    <GButton label="Join" icon="video" iconSize={16} variant={isNow(e) ? "filled" : "glass"} onclick={() => open(e.meet)} />
                  {/if}
                </div>
              {/each}
            </section>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .cal {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--sp-16);
  }

  /* ---------- month: glass plate ---------- */
  .month {
    --cell: 36px;
    position: relative;
    flex: none;
    align-self: flex-start;
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    padding: var(--sp-8) var(--sp-12) var(--sp-12);
    border-radius: calc(var(--r-card) + var(--sp-4));
    isolation: isolate;
  }
  .plate-host {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    z-index: -1;
    pointer-events: none;
  }
  .plate-host :global(.cal-plate.lg-glass) {
    --lg-glass: var(--c-surface-card);
    position: absolute;
    inset: 0;
    border-radius: inherit;
    /* Nothing but the solid panel behind it: keep the rim, skip the blur pass. */
    -webkit-backdrop-filter: none !important;
    backdrop-filter: none !important;
  }
  .month-head {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-left: var(--sp-4);
    /* glisse icon buttons are 40 px; keep the header compact */
    height: var(--size-control);
  }
  .month-name {
    flex: 1;
    font-size: var(--fs-body);
    font-weight: var(--fw-strong);
    white-space: nowrap;
  }
  .month-head :global(.lg-button[data-icon="only"]) {
    width: 32px;
    height: 32px;
  }
  .weekdays,
  .grid {
    display: grid;
    grid-template-columns: repeat(7, var(--cell));
  }
  .weekdays span {
    text-align: center;
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    color: var(--c-muted);
    line-height: 20px;
  }
  .grid {
    position: relative;
    grid-auto-rows: var(--cell);
  }

  /* The lens: a small piece of glass that slides to the picked day. */
  .lens {
    position: absolute;
    left: 0;
    top: 0;
    width: var(--cell);
    height: var(--cell);
    border-radius: 50%;
    transform: translate(calc(var(--col) * 100%), calc(var(--row) * 100%)) scale(0.92);
    background:
      radial-gradient(120% 90% at 30% 20%, rgba(255, 255, 255, 0.22), transparent 55%),
      color-mix(in srgb, var(--c-ink) 12%, transparent);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.35),
      inset 0 -1px 0 rgba(0, 0, 0, 0.18),
      inset 0 0 0 1px color-mix(in srgb, var(--c-ink) 18%, transparent),
      0 2px 8px rgba(0, 0, 0, 0.25);
    transition: transform var(--dur-slow) var(--ease-settle), background-color var(--dur-fast);
    pointer-events: none;
  }
  .lens.today {
    background:
      radial-gradient(120% 90% at 30% 20%, rgba(255, 255, 255, 0.35), transparent 55%),
      var(--c-accent);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.45),
      inset 0 -1px 0 rgba(0, 0, 0, 0.2),
      0 2px 10px color-mix(in srgb, var(--c-accent) 45%, transparent);
  }
  :global([data-reduce-motion="true"]) .lens { transition: none; }

  .day {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border-radius: 50%;
    color: var(--c-ink);
    font-size: var(--fs-small);
    transition: transform var(--dur-fast) var(--ease-settle);
  }
  .day:hover:not(:disabled):not(.sel) { background: var(--c-hover); }
  .day:active:not(:disabled) { transform: scale(0.94); }
  .day:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: -2px; }
  :global([data-input="pointer"]) .day:focus-visible { outline: none; }
  .day.past .num { color: var(--c-muted); }
  .day.out { opacity: 0.4; }
  .day:disabled { opacity: 0.2; cursor: default; }
  .day.is-today .num { color: var(--c-accent-text); font-weight: var(--fw-strong); }
  .day.is-today.sel .num { color: var(--c-accent-ink); }
  .day.sel .num { font-weight: var(--fw-strong); }
  .num { line-height: 1; }
  .dots { display: flex; gap: 2px; height: 4px; }
  .dotc { width: 4px; height: 4px; border-radius: 50%; }
  .day.is-today.sel .dotc { box-shadow: 0 0 0 1px var(--c-accent-ink); }

  /* ---------- agenda pane ---------- */
  .pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    height: var(--size-control);
    flex: none;
  }
  .pane-title {
    flex: 1;
    margin: 0;
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
  }
  .pane-head :global(.lg-button[data-icon="only"]) { width: 32px; height: 32px; }
  .pane-head :global(.lg-button:not([data-icon="only"])) { height: 32px; }
  .pane-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-8);
    text-align: center;
    font-size: var(--fs-small);
    color: var(--c-muted);
    padding: 0 var(--sp-16);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
    scrollbar-width: none;
    mask-image: linear-gradient(currentColor 88%, transparent);
    padding-bottom: var(--sp-16);
  }
  section { display: flex; flex-direction: column; gap: var(--sp-4); }
  .day-h { margin: 0 0 var(--sp-2); padding-left: var(--sp-4); }
  .event {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    border-radius: var(--r-card);
    background: var(--c-surface-card);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), inset 0 0 0 1px var(--c-hairline);
    padding-right: var(--sp-8);
  }
  .event.now { box-shadow: inset 0 0 0 1px var(--c-accent); }
  .event.past, .event.declined { opacity: 0.5; }
  .event.declined .name { text-decoration: line-through; }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    padding: var(--sp-8) var(--sp-4) var(--sp-8) var(--sp-8);
    text-align: left;
    color: inherit;
    border-radius: var(--r-card);
  }
  .main:hover:not(:disabled) { background: var(--c-hover); }
  .main:focus-visible { outline: var(--focus-width) solid var(--c-accent); }
  .bar { width: 4px; align-self: stretch; border-radius: 2px; flex: none; }
  .when { flex: none; width: 5.5em; white-space: nowrap; font-size: var(--fs-small); color: var(--c-muted); }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .name { font-size: var(--fs-small); font-weight: var(--fw-strong); }
  .sub { display: flex; align-items: center; gap: var(--sp-4); font-size: var(--fs-label); color: var(--c-muted); }
  .tag { flex: none; font-size: var(--fs-label); font-weight: var(--fw-strong); color: var(--c-accent-text); }
  .error { margin: 0; color: var(--c-error); font-size: var(--fs-small); }
</style>
