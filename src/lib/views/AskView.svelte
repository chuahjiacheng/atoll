<!--
  Ask tab (ask-assistant): a chat with Gemini inside the island.
  Replies stream in and render as Markdown; code blocks have a Copy button
  and links open in the browser. Enter sends, Shift+Enter adds a line, and
  the button turns into Stop while an answer is being written.
  With no API key yet, the tab explains where to add one.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { askApi, onAsk, onAskDelta, ASK_MAX_INPUT, PROVIDER_LABEL, type AskMsg, type AskView } from "../ask";
  import { renderMarkdown } from "../markdown";
  import { settingsApi } from "../settings";
  import GButton from "../glass/GButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let { tone }: { tone: Tone } = $props();

  let view = $state<AskView | null>(null);
  let messages = $state<AskMsg[]>([]);
  let draft = $state("");
  let error = $state("");
  let copied = $state<number | null>(null);
  let input = $state<HTMLTextAreaElement>();
  let log = $state<HTMLDivElement>();
  /** Keep the newest text in view unless you scrolled up to read. */
  let stick = true;

  const busy = $derived(view?.busy ?? null);
  /** Header chip: who answers, and with which model. */
  const chip = $derived(
    !view ? "" : view.provider === "gemini" ? view.model : `${PROVIDER_LABEL[view.provider]} · ${view.model || "auto"}`,
  );
  /** "via …" when a backup provider or stand-in model answered. */
  function via(m: AskMsg): string {
    if (!view || !m.provider || !m.model) return "";
    if (m.provider !== view.provider) return `via ${PROVIDER_LABEL[m.provider] ?? m.provider} · ${m.model}`;
    return view.model && m.model !== view.model ? `via ${m.model}` : "";
  }
  const SUGGESTIONS = ["Explain a regex", "Write a commit message", "Summarise a paragraph"];

  function apply(v: AskView) {
    view = v;
    messages = v.messages;
    follow();
  }

  onMount(() => {
    askApi.get().then(apply).catch((e) => (error = String(e)));
    const un1 = onAsk(apply);
    const un2 = onAskDelta(({ id, text }) => {
      const m = messages.find((x) => x.id === id);
      if (m) m.text = text;
      follow();
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
    };
  });

  export function focus() {
    input?.focus();
  }

  async function follow() {
    if (!stick) return;
    await tick();
    if (log) log.scrollTop = log.scrollHeight;
  }

  function scrolled() {
    if (!log) return;
    stick = log.scrollHeight - log.scrollTop - log.clientHeight < 24;
  }

  async function send(text = draft) {
    const t = text.trim();
    if (!t || busy) return;
    error = "";
    stick = true;
    try {
      await askApi.send(t);
      draft = "";
      await tick();
      grow();
    } catch (e) {
      error = String(e);
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  }

  /** The box grows with its text, up to five lines. */
  function grow() {
    if (!input) return;
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, 5 * 20 + 16)}px`;
  }

  async function copy(text: string, id: number) {
    try {
      await navigator.clipboard.writeText(text);
      copied = id;
      setTimeout(() => copied === id && (copied = null), 1500);
    } catch {
      error = "Couldn't copy.";
    }
  }

  // Clicks inside rendered Markdown: code-block Copy buttons and links.
  function logClick(e: MouseEvent) {
    const el = e.target as HTMLElement;
    const btn = el.closest<HTMLButtonElement>("[data-copy]");
    if (btn) {
      const code = btn.closest(".md-code")?.querySelector("code")?.textContent ?? "";
      navigator.clipboard.writeText(code).then(() => {
        btn.textContent = "Copied";
        setTimeout(() => (btn.textContent = "Copy"), 1500);
      });
      return;
    }
    const link = el.closest<HTMLAnchorElement>("a[data-href]");
    if (link) {
      e.preventDefault();
      const url = link.dataset.href ?? "";
      if (url.startsWith("https://")) askApi.openLink(url).catch((err) => (error = String(err)));
      else error = "Only https links can be opened.";
    }
  }
</script>

{#if view && !view.hasKey}
  <div class="empty-state">
    <Pip mood="curious" {tone} size={64} />
    {#if view.provider === "groq"}
      <div class="title">Add a free Groq key to start</div>
      <div class="hint">Ask is set to use Groq. A key from the Groq console is free and takes a minute to make.</div>
    {:else}
      <div class="title">Add a free Gemini key to start</div>
      <div class="hint">Ask uses Google's Gemini. A key from Google AI Studio is free and takes a minute to make.</div>
    {/if}
    <GButton label="Open Settings → Ask" icon="sparkle" iconSize={16} variant="filled" onclick={() => settingsApi.openDashboard("ask")} />
  </div>
{:else}
  <div class="ask">
    <div class="head">
      <span class="label strong">Ask</span>
      {#if chip}<span class="model label" title="Change this in Settings → Ask">{chip}</span>{/if}
      <span class="spacer"></span>
      {#if messages.length}
        <GButton label="New chat" icon="plus" iconSize={16} onclick={() => askApi.clear()} />
      {/if}
    </div>

    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="log" bind:this={log} onscroll={scrolled} onclick={logClick} data-lenis-prevent aria-live="polite">
      {#if !messages.length}
        <div class="pane-empty">
          <Pip mood="idle" {tone} size={40} />
          <span>Ask anything. Enter sends, Shift+Enter adds a line.</span>
          <div class="chips">
            {#each SUGGESTIONS as s}
              <button type="button" class="chip" onclick={() => { draft = `${s}: `; focus(); }}>{s}</button>
            {/each}
          </div>
        </div>
      {/if}
      {#each messages as m (m.id)}
        {#if m.role === "user"}
          <div class="msg user"><div class="bubble">{m.text}</div></div>
        {:else}
          <div class="msg assistant">
            {#if m.text}
              <div class="md">{@html renderMarkdown(m.text)}</div>
            {:else if busy === m.id}
              <div class="thinking" role="status"><span class="word">Thinking</span><span class="dot"></span><span class="dot"></span><span class="dot"></span></div>
            {/if}
            {#if m.error}<p class="msg-error" class:stopped={m.error === "Stopped."} role={m.error === "Stopped." ? undefined : "alert"}>{m.error}</p>{/if}
            {#if m.text && busy !== m.id}
              <div class="tools">
                <button type="button" class="tool" onclick={() => copy(m.text, m.id)} title="Copy the answer">
                  <Icon name={copied === m.id ? "check" : "copy"} size={14} />{copied === m.id ? "Copied" : "Copy"}
                </button>
                {#if via(m)}
                  <span class="via" title="The main choice couldn't answer, so this one did">{via(m)}</span>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      {/each}
    </div>

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <form class="composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
      <textarea
        bind:this={input}
        bind:value={draft}
        oninput={grow}
        onkeydown={keydown}
        rows="1"
        maxlength={ASK_MAX_INPUT}
        placeholder="Ask anything"
        aria-label="Ask a question"
        spellcheck="true"
        data-lenis-prevent
      ></textarea>
      {#if busy}
        <GButton icon="stop" iconSize={16} ariaLabel="Stop answering" title="Stop" onclick={() => askApi.stop()} />
      {:else}
        <GButton icon="send" iconSize={16} ariaLabel="Send" title="Send (Enter)" variant="filled" type="submit" disabled={!draft.trim()} />
      {/if}
    </form>
  </div>
{/if}

<style>
  .ask { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: var(--sp-8); }
  .head { display: flex; align-items: center; gap: var(--sp-8); min-height: 32px; }
  .model {
    padding: 2px var(--sp-8);
    border-radius: var(--r-pill);
    background: var(--c-surface-card);
    color: var(--c-muted);
    font-family: var(--font-code);
    font-size: var(--fs-label);
  }
  .spacer { flex: 1; }

  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
    padding-right: var(--sp-4);
    overscroll-behavior: contain;
  }
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
  }
  .chips { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--sp-6); margin-top: var(--sp-4); }
  .chip, .tool {
    border: none;
    border-radius: var(--r-pill);
    background: var(--c-surface-card);
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-label);
    padding: var(--sp-4) var(--sp-10);
    cursor: pointer;
  }
  .chip:hover, .tool:hover { background: var(--c-hover); }
  .chip:focus-visible, .tool:focus-visible { outline: var(--focus-width) solid var(--c-accent); }

  .msg { display: flex; flex-direction: column; max-width: 100%; }
  .msg.user { align-items: flex-end; }
  .bubble {
    max-width: 85%;
    padding: var(--sp-8) var(--sp-12);
    border-radius: var(--r-card);
    background: var(--c-accent);
    color: var(--c-accent-ink);
    font-size: var(--fs-body);
    line-height: var(--lh-loose);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .msg.assistant { align-items: stretch; }
  .tools { display: flex; gap: var(--sp-4); margin-top: var(--sp-4); }
  .tool { display: inline-flex; align-items: center; gap: var(--sp-4); background: transparent; color: var(--c-muted); padding: 2px var(--sp-6); }
  .tools { align-items: center; }
  .via { font-size: var(--fs-label); color: var(--c-muted); font-family: var(--font-code); }
  .msg-error { margin: var(--sp-4) 0 0; font-size: var(--fs-small); color: var(--c-err); }
  .msg-error.stopped { color: var(--c-muted); }
  .error { margin: 0; font-size: var(--fs-small); color: var(--c-err); }

  .thinking { display: flex; align-items: center; gap: 4px; padding: var(--sp-8) 0; }
  .thinking .word { font-size: var(--fs-small); color: var(--c-muted); margin-right: var(--sp-4); }
  .thinking .dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--c-muted);
    animation: blink calc(1.2s / var(--speed, 1)) infinite ease-in-out;
  }
  .thinking .dot:nth-child(3) { animation-delay: 0.15s; }
  .thinking .dot:nth-child(4) { animation-delay: 0.3s; }
  @keyframes blink { 0%, 80%, 100% { opacity: 0.25; } 40% { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .thinking .dot { animation: none; opacity: 0.6; } }
  :global([data-reduce-motion="true"]) .thinking .dot { animation: none; opacity: 0.6; }

  .composer { display: flex; align-items: flex-end; gap: var(--sp-8); }
  textarea {
    flex: 1;
    resize: none;
    box-sizing: border-box;
    min-height: 36px;
    border: none;
    border-radius: var(--r-card);
    padding: var(--sp-8) var(--sp-12);
    background: var(--c-surface-card);
    color: var(--c-ink);
    font: inherit;
    font-size: var(--fs-body);
    line-height: 20px;
    outline: none;
  }
  /* Focus ring drawn inside the box: an outline sits outside it, where the
     tab's pane clips it and the rounded corners turn square. */
  textarea:focus-visible { outline: none; box-shadow: inset 0 0 0 var(--focus-width) var(--c-accent); }

  /* ---------- rendered Markdown ---------- */
  .md { font-size: var(--fs-body); line-height: var(--lh-loose); color: var(--c-ink); overflow-wrap: anywhere; user-select: text; }
  .md :global(p) { margin: 0 0 var(--sp-8); }
  .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(.md-h) { font-weight: var(--fw-strong); margin-top: var(--sp-12); }
  .md :global(.md-h1) { font-size: var(--fs-title); }
  .md :global(ul), .md :global(ol) { margin: 0 0 var(--sp-8); padding-left: var(--sp-24); }
  .md :global(li) { margin: 2px 0; }
  .md :global(code) { font-family: var(--font-code); font-size: var(--fs-code); background: var(--c-surface-card); border-radius: 4px; padding: 1px 4px; }
  .md :global(a) { color: var(--c-accent-text); text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
  .md :global(blockquote) { margin: 0 0 var(--sp-8); padding-left: var(--sp-12); border-left: 2px solid var(--c-line); color: var(--c-muted); }
  .md :global(hr) { border: none; border-top: 1px solid var(--c-line); margin: var(--sp-12) 0; }
  .md :global(.md-code) { margin: 0 0 var(--sp-8); border-radius: var(--r-card); background: var(--c-surface-card); overflow: hidden; }
  .md :global(.md-code-head) {
    display: flex; justify-content: space-between; align-items: center;
    padding: var(--sp-4) var(--sp-8) var(--sp-4) var(--sp-12);
    font-size: var(--fs-label); color: var(--c-muted);
  }
  .md :global(.md-copy) {
    border: none; background: transparent; color: var(--c-muted); font: inherit; font-size: var(--fs-label);
    padding: 2px var(--sp-6); border-radius: var(--r-control); cursor: pointer;
  }
  .md :global(.md-copy:hover) { background: var(--c-hover); color: var(--c-ink); }
  .md :global(pre) { margin: 0; padding: 0 var(--sp-12) var(--sp-10); overflow-x: auto; }
  .md :global(pre code) { background: none; padding: 0; white-space: pre; }
  .md :global(.md-table) { overflow-x: auto; margin: 0 0 var(--sp-8); }
  .md :global(table) { border-collapse: collapse; font-size: var(--fs-small); }
  .md :global(th), .md :global(td) { border: 1px solid var(--c-line); padding: var(--sp-4) var(--sp-8); text-align: left; }
  .md :global(th) { font-weight: var(--fw-strong); }
</style>
