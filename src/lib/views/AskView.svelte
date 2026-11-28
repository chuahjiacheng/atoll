<!--
  Ask tab (ask-assistant): a chat with Gemini inside the island.
  Replies stream in and render as Markdown; code blocks have a Copy button
  and links open in the browser. Enter sends, Shift+Enter adds a line, and
  the button turns into Stop while an answer is being written.
  With no API key yet, the tab explains where to add one.
  Pictures: paste one into the box (Ctrl+V) or press Snip (or its hotkey) to
  drag over part of the screen; it waits above the box until you send.
  Stage 3: buttons above the box attach your latest copy, your Claude
  sessions or a picture of the window you were using to the next question
  (each one only when switched on in Settings → Ask). Answers can go to the
  whiteboard.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { askApi, onAsk, onAskDelta, onAskPicture, onAskPictureError, ASK_MAX_INPUT, CONTEXT_LABEL, PROVIDER_LABEL, type AskContext, type AskMsg, type AskPicture, type AskView } from "../ask";
  import { boardApi } from "../tools.svelte";
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
  let boarded = $state<number | null>(null);
  /** Context to send with the next question. */
  let attach = $state<AskContext[]>([]);
  /** The window Screen would picture. */
  let screenTitle = $state("");
  /** Pictures to send with the next question. */
  let pictures = $state<AskPicture[]>([]);
  let pasting = $state(false);
  const MAX_PICTURES = 6;

  function addPicture(p: AskPicture) {
    if (pictures.length >= MAX_PICTURES) {
      error = `Send at most ${MAX_PICTURES} pictures with one question.`;
      return;
    }
    pictures = [...pictures, p];
    error = "";
  }

  /** Ctrl+V: a picture on the clipboard waits above the box; text pastes as usual. */
  async function paste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.items ?? [])].filter((i) => i.kind === "file" && i.type.startsWith("image/")).map((i) => i.getAsFile()).filter((f): f is File => !!f);
    if (!files.length) return;
    e.preventDefault();
    pasting = true;
    try {
      for (const f of files) {
        const data = await new Promise<string>((ok, fail) => {
          const r = new FileReader();
          r.onload = () => ok(String(r.result));
          r.onerror = () => fail(r.error);
          r.readAsDataURL(f);
        });
        addPicture(await askApi.pastePicture(data));
      }
    } catch (err) {
      error = String(err);
    } finally {
      pasting = false;
    }
  }
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
    const screenNew = !view?.context.includes("screen") && v.context.includes("screen");
    view = v;
    messages = v.messages;
    // A context switched off in Settings is dropped from the next question.
    attach = attach.filter((k) => v.context.includes(k));
    if (screenNew) refreshScreen();
    follow();
  }

  onMount(() => {
    askApi.get().then(apply).catch((e) => (error = String(e)));
    askApi.warm().catch(() => {});
    const un1 = onAsk(apply);
    const un2 = onAskDelta(({ id, text }) => {
      const m = messages.find((x) => x.id === id);
      if (m) m.text = text;
      follow();
    });
    const un3 = onAskPicture((p) => {
      addPicture(p);
      tick().then(() => input?.focus());
    });
    const un4 = onAskPictureError((msg) => (error = msg));
    return () => {
      for (const u of [un1, un2, un3, un4]) u.then((f) => f());
    };
  });

  export function focus() {
    input?.focus();
    refreshScreen();
    askApi.warm().catch(() => {});
  }

  function refreshScreen() {
    if (view?.context.includes("screen")) askApi.screenTitle().then((t) => (screenTitle = t)).catch(() => {});
  }

  function toggle(k: AskContext) {
    attach = attach.includes(k) ? attach.filter((x) => x !== k) : [...attach, k];
    if (k === "screen") refreshScreen();
    input?.focus();
  }

  async function toBoard(m: AskMsg) {
    try {
      await boardApi.addText(m.text, 24, 24);
      boarded = m.id;
      setTimeout(() => boarded === m.id && (boarded = null), 1500);
    } catch (e) {
      error = String(e);
    }
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
    // A picture on its own gets a default question.
    const t = text.trim() || (pictures.length ? "What's in this picture?" : "");
    if (!t || busy) return;
    error = "";
    stick = true;
    try {
      await askApi.send(t, attach, pictures.map((p) => p.path));
      draft = "";
      attach = [];
      pictures = [];
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

  /** Web sources behind an answer: those attached to the question before it. */
  function sourcesFor(m: AskMsg): { title: string; url: string }[] {
    const i = messages.indexOf(m);
    for (let j = i - 1; j >= 0; j--) {
      if (messages[j].role === "user") return messages[j].attached?.find((a) => a.kind === "web")?.sources ?? [];
    }
    return [];
  }

  function host(url: string): string {
    try {
      return new URL(url).hostname.replace(/^www\./, "");
    } catch {
      return url;
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
          <div class="msg user">
            {#if m.attached?.length}
              <div class="attached">
                {#each m.attached as a}<span class="att" title="Sent with this question">{a.label}</span>{/each}
              </div>
            {/if}
            <div class="bubble">{m.text}</div>
          </div>
        {:else}
          <div class="msg assistant">
            {#if m.text}
              <div class="md">{@html renderMarkdown(m.text)}</div>
            {:else if busy === m.id}
              <div class="thinking" role="status"><span class="word">{view?.searching ? "Searching the web" : "Thinking"}</span><span class="dot"></span><span class="dot"></span><span class="dot"></span></div>
            {/if}
            {#if m.text && sourcesFor(m).length}
              <ol class="sources" aria-label="Sources">
                {#each sourcesFor(m) as src, i}
                  <li>
                    <a href={src.url} data-href={src.url} title={src.url}><span class="n">{i + 1}</span>{src.title}<span class="host">{host(src.url)}</span></a>
                  </li>
                {/each}
              </ol>
            {/if}
            {#if m.error}<p class="msg-error" class:stopped={m.error === "Stopped."} role={m.error === "Stopped." ? undefined : "alert"}>{m.error}</p>{/if}
            {#if m.text && busy !== m.id}
              <div class="tools">
                <button type="button" class="tool" onclick={() => copy(m.text, m.id)} title="Copy the answer">
                  <Icon name={copied === m.id ? "check" : "copy"} size={14} />{copied === m.id ? "Copied" : "Copy"}
                </button>
                <button type="button" class="tool" onclick={() => toBoard(m)} title="Put the answer on the whiteboard">
                  <Icon name={boarded === m.id ? "check" : "whiteboard"} size={14} />{boarded === m.id ? "On the board" : "To board"}
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

    {#if pictures.length}
      <div class="pics" role="list" aria-label="Pictures to send">
        {#each pictures as p, i (p.path)}
          <div class="pic" role="listitem">
            <img src={p.thumb} alt="Picture {i + 1}, {p.width} by {p.height}" />
            <button type="button" class="pic-x" aria-label="Remove picture {i + 1}" title="Remove" onclick={() => (pictures = pictures.filter((x) => x !== p))}>
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/if}

    <div class="context" role="group" aria-label="Send with the next question">
      <button type="button" class="ctx" title="Drag over part of your screen; the picture comes here (Ctrl+Alt+Q from anywhere)" onclick={() => askApi.snip()}>
        <Icon name="image" size={14} />Snip
      </button>
      {#if view?.context.length}
        {#each view.context as k}
          <button
            type="button"
            class="ctx"
            class:on={attach.includes(k)}
            aria-pressed={attach.includes(k)}
            title={k === "web"
              ? "Search the web first and answer from what it finds"
              : k === "screen"
              ? screenTitle
                ? `Send a picture of “${screenTitle}” (Gemini only)`
                : "Send a picture of the window you were using (Gemini only)"
              : k === "clipboard"
                ? "Send your latest copy"
                : "Send what your Claude Code sessions are doing"}
            onclick={() => toggle(k)}
          >
            <Icon name={k === "web" ? "search" : k === "screen" ? "monitor" : k === "clipboard" ? "clipboard" : "agent"} size={14} />
            {CONTEXT_LABEL[k]}{#if k === "screen" && screenTitle}<span class="ctx-sub">· {screenTitle}</span>{/if}
          </button>
        {/each}
      {/if}
    </div>

    <form class="composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
      <textarea
        bind:this={input}
        bind:value={draft}
        oninput={grow}
        onkeydown={keydown}
        onpaste={paste}
        rows="1"
        maxlength={ASK_MAX_INPUT}
        placeholder={pasting ? "Adding picture…" : pictures.length ? "Ask about the picture" : "Ask anything (Ctrl+V pastes pictures too)"}
        aria-label="Ask a question"
        spellcheck="true"
        data-lenis-prevent
      ></textarea>
      {#if busy}
        <GButton icon="stop" iconSize={16} ariaLabel="Stop answering" title="Stop" onclick={() => askApi.stop()} />
      {:else}
        <GButton icon="send" iconSize={16} ariaLabel="Send" title="Send (Enter)" variant="filled" type="submit" disabled={!draft.trim() && !pictures.length} />
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
  .attached { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--sp-4); margin-bottom: var(--sp-4); max-width: 85%; }
  .att {
    font-size: var(--fs-label);
    color: var(--c-muted);
    background: var(--c-surface-card);
    border-radius: var(--r-pill);
    padding: 2px var(--sp-8);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pics { display: flex; gap: var(--sp-6); flex-wrap: wrap; }
  .pic { position: relative; width: 56px; height: 56px; border-radius: var(--r-control); overflow: hidden; background: var(--c-surface-card); flex: none; }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .pic-x {
    position: absolute; top: 2px; right: 2px;
    width: 18px; height: 18px; padding: 0;
    display: grid; place-items: center;
    border: none; border-radius: 50%;
    background: var(--c-scrim); color: var(--c-ink);
    cursor: pointer;
  }
  .pic-x:hover { background: var(--c-err); color: var(--c-err-ink); }
  .pic-x:focus-visible { outline: none; box-shadow: 0 0 0 var(--focus-width) var(--c-accent); }
  .context { display: flex; gap: var(--sp-6); flex-wrap: nowrap; overflow: hidden; }
  .ctx {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    min-width: 0;
    border: none;
    border-radius: var(--r-pill);
    background: var(--c-surface-card);
    color: var(--c-muted);
    font: inherit;
    font-size: var(--fs-label);
    padding: var(--sp-4) var(--sp-10);
    cursor: pointer;
    white-space: nowrap;
  }
  .ctx:hover { background: var(--c-hover); color: var(--c-ink); }
  .ctx.on { background: var(--c-accent); color: var(--c-accent-ink); }
  .ctx:focus-visible { outline: none; box-shadow: inset 0 0 0 var(--focus-width) var(--c-accent); }
  .ctx.on:focus-visible { box-shadow: inset 0 0 0 var(--focus-width) var(--c-accent-ink); }
  .sources { list-style: none; margin: var(--sp-8) 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .sources a {
    display: flex;
    align-items: baseline;
    gap: var(--sp-6);
    font-size: var(--fs-small);
    color: var(--c-ink);
    text-decoration: none;
    border-radius: var(--r-control);
    padding: 2px var(--sp-4);
    cursor: pointer;
    min-width: 0;
  }
  .sources a:hover { background: var(--c-hover); }
  .sources a:focus-visible { outline: none; box-shadow: inset 0 0 0 var(--focus-width) var(--c-accent); }
  .sources .n { color: var(--c-muted); font-variant-numeric: tabular-nums; min-width: 1.2em; }
  .sources .host { color: var(--c-muted); font-size: var(--fs-label); margin-left: auto; padding-left: var(--sp-8); white-space: nowrap; }
  .ctx-sub { overflow: hidden; text-overflow: ellipsis; max-width: 180px; opacity: 0.8; }
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
