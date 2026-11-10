<!--
  Claude asks you something, and you answer right in the island.

  Two kinds:
  - "ask": Claude ended its turn with a question and reply choices (the
    Claude app's "next step" box). Picking one sends it to Claude as your
    reply, through the Claude app.
  - "question": Claude's multiple-choice question tool (AskUserQuestion) is
    waiting. Picking answers it directly; several questions go one at a time.
    "Other" lets you type your own answer.

  Each choice is a full-width row: number, label, and Claude's description.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { api, type Ask, type Permission, type Question } from "../api";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let {
    kind,
    project,
    tone,
    ask = null,
    request = null,
    now = Date.now(),
    onPick,
    onLater,
  }: {
    kind: "ask" | "question";
    project: string;
    tone: Tone;
    /** kind "ask": the question and its reply choices. */
    ask?: Ask | null;
    /** kind "question": the waiting AskUserQuestion request. */
    request?: Permission | null;
    now?: number;
    /** kind "ask": send this reply (rejects with a message when it can't). */
    onPick?: (label: string) => Promise<void>;
    /** Hide the peek; the question stays in the Agent tab. */
    onLater: () => void;
  } = $props();

  const questions = $derived<Question[]>(
    kind === "question"
      ? (request?.questions ?? [])
      : ask
        ? [{ question: ask.question, options: ask.choices.map((label) => ({ label })) }]
        : [],
  );
  let step = $state(0);
  const q = $derived(questions[Math.min(step, questions.length - 1)]);
  /** Answers so far, by question text. */
  let answers = $state<Record<string, string>>({});
  /** Multi-select: the labels ticked on this question. */
  let ticked = $state<string[]>([]);
  let typing = $state(false);
  let typed = $state("");
  let busy = $state(false);
  let error = $state("");
  let field = $state<HTMLInputElement>();

  const secondsLeft = $derived(request ? Math.max(0, Math.ceil((request.expiresAt - now) / 1000)) : 0);
  const last = $derived(step >= questions.length - 1);

  async function finish(answer: string) {
    if (!q || busy) return;
    error = "";
    if (kind === "ask") {
      busy = true;
      try {
        await onPick?.(answer);
      } catch (e) {
        error = String(e);
        busy = false;
      }
      return;
    }
    answers = { ...answers, [q.question]: answer };
    if (!last) {
      step += 1;
      ticked = [];
      typing = false;
      typed = "";
      return;
    }
    busy = true;
    try {
      await api.decidePermission(request!.id, "answer", undefined, answers);
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }

  function pick(label: string) {
    if (q?.multiSelect) ticked = ticked.includes(label) ? ticked.filter((l) => l !== label) : [...ticked, label];
    else finish(label);
  }

  async function openTyping() {
    typing = true;
    await tick();
    field?.focus();
  }

  async function handBack() {
    if (busy || !request) return;
    busy = true;
    try {
      await api.decidePermission(request.id, "pass");
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }
</script>

<div class="peek" role="dialog" aria-label="Question from {project || 'Claude Code'}">
  <div class="top">
    <Pip mood="curious" {tone} size={40} />
    <div class="body">
      <div class="kicker">
        <Icon name="info" size={16} /><span>{project ? `${project} asks` : "Claude asks"}</span>
        <span class="flex"></span>
        {#if questions.length > 1}<span class="meta tabular">{step + 1} of {questions.length}</span>{/if}
        {#if request}<span class="meta tabular" title="When this runs out, Claude Code asks in its own window instead">{secondsLeft} s</span>{/if}
      </div>
      {#if q?.header && q.header !== q.question}<div class="header">{q.header}</div>{/if}
      <div class="title">{q?.question}</div>
    </div>
  </div>

  {#if q}
    {#key step}
      <div class="options" role={q.multiSelect ? "group" : "list"}>
        {#each q.options as o, i (o.label)}
          {@const on = ticked.includes(o.label)}
          <button
            class="opt"
            class:on
            class:first={kind === "ask" && i === 0}
            disabled={busy}
            role={q.multiSelect ? "checkbox" : undefined}
            aria-checked={q.multiSelect ? on : undefined}
            onclick={() => pick(o.label)}
          >
            <span class="num" aria-hidden="true">{#if q.multiSelect && on}<Icon name="check" size={16} />{:else}{i + 1}{/if}</span>
            <span class="text">
              <span class="label">{o.label}</span>
              {#if o.description}<span class="desc">{o.description}</span>{/if}
            </span>
            {#if kind === "ask" && i === 0 && q.options.length > 1}<span class="tag">Suggested</span>{/if}
            {#if !q.multiSelect}<span class="go" aria-hidden="true"><Icon name="chevronRight" size={16} /></span>{/if}
          </button>
        {/each}

        {#if typing}
          <form
            class="opt typing"
            onsubmit={(e) => {
              e.preventDefault();
              if (typed.trim()) finish(typed.trim());
            }}
          >
            <span class="num" aria-hidden="true"><Icon name="edit" size={16} /></span>
            <input bind:this={field} bind:value={typed} maxlength="500" placeholder="Type your answer" aria-label="Your answer" />
            <button class="send" type="submit" disabled={busy || !typed.trim()} aria-label="Send answer"><Icon name="send" size={16} /></button>
          </form>
        {/if}
      </div>
    {/key}
  {/if}

  {#if q?.multiSelect}
    <button class="primary" disabled={busy || !ticked.length} onclick={() => finish(ticked.join(", "))}>
      {last ? "Send answer" : "Next"}
    </button>
  {/if}

  <div class="extras">
    {#if !typing}<button class="link" onclick={openTyping}>Type my own</button>{/if}
    {#if request}
      <button class="link" disabled={busy} onclick={handBack} title="Let Claude Code ask you in its own window">Answer in Claude</button>
    {:else}
      <button class="link" onclick={onLater} title="Hide this. The question stays in the Agent tab.">Later</button>
    {/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .peek { padding: var(--sp-16); display: flex; flex-direction: column; gap: var(--sp-12); }
  .top { display: flex; gap: var(--sp-16); align-items: flex-start; }
  .body { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; flex: 1; }
  .kicker {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    color: var(--c-accent-text);
    min-width: 0;
  }
  .kicker > span:nth-child(2) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .flex { flex: 1; }
  .meta { color: var(--c-muted); font-weight: var(--fw-regular); margin-left: var(--sp-8); }
  .header { font-size: var(--fs-label); color: var(--c-muted); text-transform: uppercase; letter-spacing: 0.04em; margin-top: var(--sp-4); }
  .title {
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .options { display: flex; flex-direction: column; gap: var(--sp-4); }
  .opt {
    display: flex;
    align-items: center;
    gap: var(--sp-12);
    width: 100%;
    min-height: 44px;
    padding: var(--sp-8) var(--sp-12) var(--sp-8) var(--sp-8);
    box-sizing: border-box;
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    text-align: left;
    transition: background-color var(--dur-fast), box-shadow var(--dur-fast);
  }
  .opt:hover:not(:disabled),
  .opt:focus-visible {
    background-image: linear-gradient(var(--c-hover), var(--c-hover));
  }
  .opt.first { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c-accent) 55%, transparent); }
  .opt.on { box-shadow: inset 0 0 0 1.5px var(--c-accent); }
  .opt:disabled { opacity: 0.6; cursor: default; }
  .num {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--c-accent) 16%, transparent);
    color: var(--c-accent-text);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
  }
  .opt.on .num { background: var(--c-accent); color: var(--c-accent-ink); }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .label { font-size: var(--fs-small); font-weight: var(--fw-strong); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .desc {
    font-size: var(--fs-label);
    color: var(--c-muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .tag {
    flex: none;
    font-size: var(--fs-label);
    color: var(--c-accent-text);
    padding: 2px var(--sp-8);
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--c-accent) 14%, transparent);
  }
  .go { flex: none; display: flex; color: var(--c-muted); opacity: 0; transition: opacity var(--dur-fast); }
  .opt:hover .go { opacity: 1; }

  .typing { cursor: text; }
  .typing input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    font: inherit;
    font-size: var(--fs-small);
    color: var(--c-ink);
  }
  .typing input::placeholder { color: var(--c-muted); }
  .send {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--r-pill);
    background: var(--c-accent);
    color: var(--c-accent-ink);
    flex: none;
  }
  .send:disabled { opacity: 0.4; cursor: default; }

  .primary {
    height: var(--size-control);
    border-radius: var(--r-pill);
    background: var(--c-accent);
    color: var(--c-accent-ink);
    font-weight: var(--fw-strong);
    font-size: var(--fs-small);
  }
  .primary:disabled { opacity: 0.45; cursor: default; }

  .extras { display: flex; justify-content: center; gap: var(--sp-24); }
  .link {
    font-size: var(--fs-small);
    color: var(--c-muted);
    text-decoration: underline;
    text-underline-offset: var(--sp-4);
    text-decoration-color: var(--c-dash);
  }
  .link:hover { color: var(--c-ink); }
  .error { margin: 0; color: var(--c-error); font-size: var(--fs-small); }
</style>
