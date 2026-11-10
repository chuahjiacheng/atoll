<!--
  Focus tab (design: Island → focus / focus-break). The big ring counts down
  the session or break; presets set the next session's length; the primary
  button starts, pauses, resumes or skips the break. During a break, alerts
  that were held back while you focused are listed here too.
-->
<script lang="ts">
  import Icon, { type IconName } from "../ui/Icon.svelte";
  import Ring from "../ui/Ring.svelte";
  import GButton from "../glass/GButton.svelte";
  import {
    FOCUS_PRESETS,
    focusApi,
    focusClockMs,
    focusFraction,
    focusTotal,
    presetOf,
    spokenTime,
    timerText,
    type FocusAction,
    type FocusView,
  } from "../focus";
  import { settingsApi, type FocusSettings } from "../settings";
  import { px } from "../theme";

  let { view, settings, now }: { view: FocusView; settings: FocusSettings; now: number } = $props();

  const RING = px("--size-ring-focus") || 200;
  const STROKE = px("--stroke-ring-focus") || 12;

  let error = $state("");
  let draft = $state("");

  const isBreak = $derived(view.phase === "break");
  const ready = $derived(view.phase === "ready");
  const openEnded = $derived(view.phase === "focus" ? view.lengthMs === 0 : ready && settings.openEnded);
  const clock = $derived(timerText(focusClockMs(view, now), openEnded));
  const fraction = $derived(ready ? 1 : openEnded ? 0 : 1 - focusFraction(view, now));

  const look = $derived.by<{ word: string; icon: IconName; tone: "ink" | "accent" | "done"; sub: string }>(() => {
    if (isBreak) {
      const word = view.longBreak ? "Long break" : "Break";
      return { word: view.running ? word : `${word} paused`, icon: "coffee", tone: "done", sub: view.running ? word : "Paused" };
    }
    if (view.phase === "focus") {
      const sub = `Focus ${view.round} of ${view.rounds}`;
      return view.running
        ? { word: openEnded ? "Focusing, no time limit" : "Focus running", icon: "focus", tone: "accent", sub }
        : { word: "Paused", icon: "pause", tone: "ink", sub };
    }
    return { word: view.round > 1 ? `Next: focus ${view.round} of ${view.rounds}` : "Not started", icon: "focus", tone: "ink", sub: "Ready" };
  });

  const primary = $derived.by<{ label: string; icon: IconName; action: FocusAction }>(() => {
    if (ready) return { label: "Start", icon: "play", action: "start" };
    if (isBreak && view.running) return { label: "Skip break", icon: "next", action: "skip" };
    if (view.running) return { label: "Pause", icon: "pause", action: "pause" };
    return { label: "Resume", icon: "play", action: "resume" };
  });

  const ringLabel = $derived.by(() => {
    const t = spokenTime(focusClockMs(view, now));
    if (ready) return openEnded ? "Timer ready, no time limit" : `Timer ready, ${t}`;
    if (openEnded) return `Focus, ${t} so far`;
    return `${isBreak ? "Break" : "Focus"}, ${t} left`;
  });

  const preset = $derived(presetOf(settings));
  const presets = $derived([
    ...FOCUS_PRESETS,
    ...(preset === "custom" ? [{ id: "custom", focus: settings.focusMin, brk: settings.breakMin, label: `${settings.focusMin} / ${settings.breakMin}` }] : []),
    { id: "open", focus: 0, brk: settings.breakMin, label: "No limit" },
  ]);

  async function run(action: FocusAction, text?: string) {
    error = "";
    try {
      await focusApi.act(action, text);
    } catch (e) {
      error = String(e);
    }
  }

  function start() {
    run("start", draft.trim() || undefined);
    draft = "";
  }

  function pick(id: string) {
    if (id === "custom") return;
    const p = FOCUS_PRESETS.find((x) => x.id === id);
    settingsApi
      .update({ focus: id === "open" ? { openEnded: true } : { openEnded: false, focusMin: p!.focus, breakMin: p!.brk } })
      .catch((e) => (error = String(e)));
  }

  /** Arrow keys move between the preset pills (radio group). */
  function presetKey(e: KeyboardEvent) {
    if (!["ArrowLeft", "ArrowRight"].includes(e.key)) return;
    const pills = Array.from((e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button"));
    const i = pills.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    const next = pills[(i + (e.key === "ArrowRight" ? 1 : -1) + pills.length) % pills.length];
    next.focus();
    next.click();
  }

  const statsLine = $derived.by(() => {
    const s = view.stats;
    const parts = [`Today ${focusTotal(s.todayMs)}`];
    if (s.todaySessions) parts.push(`${s.todaySessions} ${s.todaySessions === 1 ? "session" : "sessions"}`);
    if (s.streak > 1) parts.push(`${s.streak}-day streak`);
    if (view.phase === "focus" && view.distractions) parts.push(`${view.distractions} ${view.distractions === 1 ? "distraction" : "distractions"}`);
    return parts.join(" · ");
  });

  const KIND_ICON: Record<string, IconName> = { done: "check", error: "alert", attention: "agent", usage: "clock", chatReply: "send", timer: "focus" };
</script>

<div class="focus">
  <div class="dial">
    <Ring value={fraction} size={RING} stroke={STROKE} tone={isBreak ? "done" : "accent"} label={ringLabel} />
    <div class="readout" aria-hidden="true">
      <span class="time tabular">{clock}</span>
      <span class="sub">{look.sub}</span>
    </div>
  </div>

  <div class="side">
    <div class="word {look.tone}"><Icon name={look.icon} size={20} />{look.word}</div>

    {#if ready}
      <input
        class="intent"
        type="text"
        maxlength="80"
        bind:value={draft}
        placeholder={view.intention || "What are you focusing on? (optional)"}
        aria-label="What you are focusing on (optional)"
        onkeydown={(e) => e.key === "Enter" && start()}
      />
    {:else if view.intention}
      <div class="intention ellipsis" title={view.intention}>{view.intention}</div>
    {/if}

    {#if isBreak && view.held.length}
      <div class="digest">
        <div class="label strong">While you focused</div>
        <ul>
          {#each view.held.slice(-3).reverse() as h (h.at + h.title)}
            <li><span class="ic {h.kind}"><Icon name={KIND_ICON[h.kind] ?? "dot"} size={14} /></span><span class="ellipsis">{h.title}</span></li>
          {/each}
        </ul>
        {#if view.held.length > 3}<div class="label">and {view.held.length - 3} more</div>{/if}
      </div>
    {:else}
      <div>
        <div class="label strong" id="focus-presets">Presets (focus / break minutes)</div>
        <!-- svelte-ignore a11y_interactive_supports_focus -->
        <div class="presets" role="radiogroup" aria-labelledby="focus-presets" tabindex="-1" onkeydown={presetKey}>
          {#each presets as p (p.id)}
            <button
              class="preset tabular"
              role="radio"
              aria-checked={preset === p.id}
              tabindex={preset === p.id ? 0 : -1}
              onclick={() => pick(p.id)}
            >{p.label}</button>
          {/each}
        </div>
      </div>
    {/if}

    <div class="actions">
      <GButton label={primary.label} icon={primary.icon} iconSize={16} variant="filled" onclick={() => (primary.action === "start" ? start() : run(primary.action))} />
      {#if view.phase === "focus" && openEnded}
        <GButton label="Take a break" icon="coffee" iconSize={16} onclick={() => run("skip")} />
      {/if}
      {#if isBreak && view.held.length}
        <GButton label="Dismiss" iconSize={16} onclick={() => run("dismiss")} />
      {/if}
      <GButton label="Reset" icon="reset" iconSize={16} disabled={ready && view.round === 1} onclick={() => run("reset")} />
    </div>

    <div class="label foot" role={error ? "alert" : undefined}>{error || statsLine}</div>
  </div>
</div>

<style>
  .focus {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--sp-32);
    padding: 0 var(--sp-16);
    min-height: 0;
  }
  .dial {
    position: relative;
    flex: none;
    display: grid;
    place-items: center;
  }
  .readout {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-4);
  }
  .time {
    font-size: var(--fs-display);
    font-weight: var(--fw-strong);
    line-height: 1;
  }
  .sub { font-size: var(--fs-small); color: var(--c-muted); }
  .side {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-16);
  }
  .word {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    font-size: var(--fs-body);
    font-weight: var(--fw-strong);
  }
  .word.accent { color: var(--c-accent-text); }
  .word.done { color: var(--c-done); }
  .intent {
    box-sizing: border-box;
    width: 100%;
    height: var(--size-control);
    padding: 0 var(--sp-12);
    border: none;
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-small);
    outline: none;
  }
  .intent::placeholder { color: var(--c-muted); }
  .intent:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: 0; }
  .intention { font-size: var(--fs-small); color: var(--c-muted); }
  .label.strong { font-weight: var(--fw-strong); margin-bottom: var(--sp-8); }
  .presets { display: flex; flex-wrap: wrap; gap: var(--sp-8); }
  .preset {
    height: var(--size-preset-pill, 36px);
    padding: 0 var(--sp-16);
    border-radius: var(--r-pill);
    border: 2px solid transparent;
    background: var(--c-surface-card);
    color: var(--c-ink);
    font-size: var(--fs-small);
    font-weight: var(--fw-strong);
    box-sizing: border-box;
    transition: border-color var(--dur-fast), background-color var(--dur-fast);
  }
  .preset:hover { background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .preset[aria-checked="true"] {
    border-color: var(--c-accent);
    background: color-mix(in srgb, var(--c-accent) 14%, var(--c-surface-card));
  }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-8); }
  .foot { min-height: 1lh; }
  .digest ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-6); }
  .digest li { display: flex; align-items: center; gap: var(--sp-8); font-size: var(--fs-small); min-width: 0; }
  .ic { display: flex; flex: none; color: var(--c-muted); }
  .ic.done { color: var(--c-done); }
  .ic.error { color: var(--c-error); }
  .ic.attention { color: var(--c-accent-text); }
</style>
