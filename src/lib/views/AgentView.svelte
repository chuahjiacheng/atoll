<!--
  Agent tab (design: Island → agent / agent-na).
  Left: each Claude Code session's status, then live activity.
  Right: the usage ring and recent finished tasks.
  Claude chats (any plan, Free included) show here too: agentSource.ts decides
  when the tab shows chats instead of Code; the Auto / Code / Chats switch overrides it.
-->
<script lang="ts">
  import { api, type AgentFeed, type ChatsView, type Session, type Usage } from "../api";
  import { pickAgentSource, type ChatSource } from "../agentSource";
  import { ago, clearable, clockTime, summary } from "../format";
  import StatusChip from "../ui/StatusChip.svelte";
  import Icon, { type IconName } from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";
  import Ring from "../ui/Ring.svelte";
  import GButton from "../glass/GButton.svelte";
  import { settingsApi } from "../settings";

  let {
    sessions,
    feed,
    usage,
    window: ringWindow,
    usageOn,
    now,
    tone,
    live = true,
    onClear = (id: string) => api.clearSession(id),
    onReply,
    chats = null,
    chatSource = "auto",
    onSource,
  }: {
    /** Claude chats from the claude.ai cache (null = not read yet / turned off). */
    chats?: ChatsView | null;
    /** The user's Auto / Code / Chats choice. */
    chatSource?: ChatSource;
    onSource?: (v: ChatSource) => void;
    /** Answer a session's end-of-turn question (sends the text to Claude). */
    onReply?: (s: Session, text: string) => Promise<void>;
    sessions: Session[];
    /** Clear one session's error / needs you / done message. */
    onClear?: (id: string) => void;
    /** Show live activity (Settings → AI agents). */
    live?: boolean;
    feed: AgentFeed;
    usage: Usage | null;
    window: "fiveHour" | "sevenDay";
    usageOn: boolean;
    now: number;
    tone: Tone;
  } = $props();

  const KIND_ICON: Record<string, IconName> = { read: "read", edit: "edit", run: "terminal", web: "external", agent: "agent", tool: "settings" };
  const rawLimit = $derived(usage ? (ringWindow === "sevenDay" ? usage.sevenDay : usage.fiveHour) : null);
  // A window whose reset time has passed no longer means anything.
  // resetsAt 0 = no window running yet (nothing used).
  const limit = $derived(rawLimit && (rawLimit.resetsAt === 0 || rawLimit.resetsAt * 1000 > now) ? rawLimit : null);
  const limitName = $derived(ringWindow === "sevenDay" ? "Weekly limit" : "5-hour limit");
  // The newest finished turn with an answer, shown first ("View answer" lands here).
  const answered = $derived(sessions.filter((s) => s.status === "done" && s.answer).sort((a, b) => b.updatedAt - a.updatedAt)[0] ?? null);

  // Questions Claude ended its turn with, still waiting for a reply.
  const asks = $derived(sessions.filter((s) => s.ask && s.status === "attention"));
  let sending = $state("");
  let replyError = $state("");
  async function reply(s: Session, text: string) {
    if (!onReply || sending) return;
    sending = `${s.id}:${text}`;
    replyError = "";
    try {
      await onReply(s, text);
    } catch (e) {
      replyError = String(e);
    } finally {
      sending = "";
    }
  }

  const choice = $derived(pickAgentSource(chatSource, chats, sessions, now));
  const SOURCES: { id: ChatSource; label: string }[] = [
    { id: "auto", label: "Auto" },
    { id: "code", label: "Claude Code" },
    { id: "chat", label: "Chats" },
  ];
  const PLAN_NAME: Record<string, string> = { free: "Free plan", pro: "Pro plan", max: "Max plan", team: "Team plan", enterprise: "Enterprise plan" };
  // The chat shown on the left: the one picked on the right, else the newest.
  let pickedChat = $state("");
  /** A chat that changed this recently without a cached reply is still being answered. */
  const REPLYING_MS = 3 * 60 * 1000;
  const chat = $derived(chats?.chats.find((c) => c.uuid === pickedChat) ?? chats?.chats[0] ?? null);

  function resets(at: number) {
    const mins = Math.max(0, Math.round((at * 1000 - now) / 60000));
    if (mins >= 60 * 24) return `Resets in ${Math.round(mins / 60 / 24)} d`;
    return mins >= 60 ? `Resets in ${Math.floor(mins / 60)} h ${mins % 60} m` : `Resets in ${mins} m`;
  }
</script>

{#snippet sourceSwitch()}
  {#if choice.canSwitch && onSource}
    <div class="sources" role="radiogroup" aria-label="Show in this tab">
      {#each SOURCES as o (o.id)}
        <button
          class="source"
          class:on={chatSource === o.id}
          role="radio"
          aria-checked={chatSource === o.id}
          title={o.id === "auto" ? "Claude Code while it's in use, otherwise your chats" : undefined}
          onclick={() => onSource(o.id)}
        >{o.id === "auto" && chatSource === "auto" ? `Auto · ${choice.show === "code" ? "Code" : "Chats"}` : o.label}</button>
      {/each}
    </div>
  {/if}
{/snippet}

{#if choice.show === "chat" && chats}
  <div class="agent clay">
    <div class="left">
      {@render sourceSwitch()}
      {#if chat}
        <div class="chat-head">
          <span class="name ellipsis grow" title={chat.name}>{chat.name || "Untitled chat"}</span>
          <span class="label tabular">{ago(chat.updatedAt, now)}</span>
          <button class="icon-btn bare small" aria-label="Open this chat on claude.ai" title="Open on claude.ai" onclick={() => api.openChat(chat.uuid, true).catch(() => {})}><Icon name="external" size={16} /></button>
        </div>
        {#if chat.behind}
          <!-- Claude saves a chat once a reply is finished; until then the cache is older. -->
          <div class="status-row behind">
            {#if now - chat.updatedAt < REPLYING_MS}
              <span class="dot-pulse" aria-hidden="true"></span>
              <span class="label">Claude is replying. It shows here when it's done.</span>
            {:else if chat.answer}
              <span class="label">Newer messages in Claude. Open the chat there to update this.</span>
            {:else}
              <span class="label">Open this chat in Claude once and its messages show up here.</span>
            {/if}
          </div>
        {/if}
        {#if chat.ask}
          <div class="you" class:old={chat.behind} data-lenis-prevent>
            <div class="label strong">You</div>
            <p>{chat.ask}</p>
          </div>
        {/if}
        {#if chat.answer}
          <div class="answer grow-answer" class:old={chat.behind} data-lenis-prevent>
            <div class="label strong">Claude{chat.answeredAt ? ` · ${ago(chat.answeredAt, now)}` : ""}</div>
            <p>{chat.answer}</p>
          </div>
        {/if}
      {/if}
    </div>

    <div class="right">
      {#if chats.plan === "free" || !usageOn}
        <div class="card plancard">
          <Icon name="agent" size={16} />
          <div class="ringtext">
            <div class="name">{PLAN_NAME[chats.plan] ?? "Claude"}</div>
            <div class="label">{chats.plan === "free" ? "Chats from Claude on this PC" : "Usage ring is off"}</div>
          </div>
        </div>
      {:else}
        <div class="card ringcard">
          <Ring
            value={limit ? limit.used / 100 : 0}
            size={80}
            stroke={8}
            dashed={!limit}
            text={limit ? `${Math.round(limit.used)}%` : "N/A"}
            label={limit ? `${Math.round(limit.used)} percent of the ${limitName} used` : "Usage not reported yet"}
          />
          <div class="ringtext">
            <div class="name">{limitName}</div>
            <div class="label">{limit ? (limit.resetsAt === 0 ? "Not started yet" : resets(limit.resetsAt)) : (PLAN_NAME[chats.plan] ?? "Shared with Claude Code")}</div>
          </div>
        </div>
      {/if}

      <div class="label strong heading">Recent chats</div>
      <div class="list" data-lenis-prevent>
        {#each chats.chats as c (c.uuid)}
          <button class="hist chat-item" class:on={chat?.uuid === c.uuid} onclick={() => (pickedChat = c.uuid)}>
            <div class="ellipsis" title={c.name}>{c.name || "Untitled chat"}</div>
            <div class="label">{ago(c.updatedAt, now)}</div>
          </button>
        {/each}
      </div>
    </div>
  </div>
{:else if sessions.length === 0 && feed.history.length === 0}
  <div class="empty-state clay">
    {@render sourceSwitch()}
    <Pip mood="idle" {tone} size={64} />
    <div class="title">No agent session yet</div>
    <div class="hint">Start Claude Code in a terminal or the Claude desktop Code tab. Its activity, status and usage appear here.</div>
    <GButton label="Connect Claude Code" icon="agent" iconSize={16} variant="filled" onclick={() => settingsApi.openDashboard("agents").catch(() => {})} />
  </div>
{:else}
  <div class="agent clay">
    <div class="left">
      {@render sourceSwitch()}
      {#each asks as s (s.id)}
        <div class="ask">
          <div class="label strong">{s.project || "Claude"} asks</div>
          <p>{s.ask?.question}</p>
          {#if s.ask?.choices.length}
            <div class="choices">
              {#each s.ask.choices as c, i (c)}
                <button class="choice" class:first={i === 0} disabled={!!sending} onclick={() => reply(s, c)}>{c}</button>
              {/each}
            </div>
          {/if}
          {#if replyError}<div class="reply-error">{replyError}</div>{/if}
        </div>
      {/each}
      {#if answered}
        <div class="answer" data-lenis-prevent>
          <div class="label strong">Latest answer · {answered.project || "Claude Code"} · {ago(answered.updatedAt, now)}</div>
          <p>{answered.answer}</p>
        </div>
      {/if}
      {#each sessions.slice(0, 3) as s (s.id)}
        <div class="status-row" title={s.cwd}>
          <StatusChip status={s.status} onClear={clearable(s.status) ? () => onClear(s.id) : undefined} />
          <span class="ellipsis grow">{s.project ? `${s.project} · ` : ""}{summary(s, live)}</span>
          {#if !clearable(s.status)}
            <button class="icon-btn bare small" aria-label="Hide {s.project || 'session'}" title="Hide session" onclick={() => api.dismissSession(s.id)}><Icon name="x" size={16} /></button>
          {/if}
        </div>
      {/each}

      {#if live}
      <div class="label strong heading">Live activity</div>
      <div class="list" data-lenis-prevent>
        {#each feed.log.slice(0, 12) as a (a.at + a.target + a.sessionId)}
          <div class="act" title={`${a.project}: ${a.verb} ${a.target}`}>
            <span class="label"><Icon name={KIND_ICON[a.kind] ?? "dot"} size={16} /></span>
            <span class="verb">{a.verb}</span>
            <span class="target ellipsis">{a.target}</span>
            <span class="label tabular">{clockTime(a.at)}</span>
          </div>
        {:else}
          <div class="act"><span class="label">Nothing yet. Tool calls show up here as Claude works.</span></div>
        {/each}
      </div>
      {/if}
    </div>

    <div class="right">
      <div class="card ringcard">
        <Ring
          value={limit ? limit.used / 100 : 0}
          size={80}
          stroke={8}
          dashed={!usageOn || !limit}
          text={usageOn && limit ? `${Math.round(limit.used)}%` : "N/A"}
          label={usageOn && limit ? `${Math.round(limit.used)} percent of the ${limitName} used` : usageOn ? "Usage not reported yet" : "Usage ring is off"}
        />
        <div class="ringtext">
          <div class="name">{limitName}</div>
          <div class="label">
            {#if usageOn && limit}
              {limit.resetsAt === 0 ? "Not started yet" : limit.used >= 100 ? `Limit reached · ${resets(limit.resetsAt)}` : resets(limit.resetsAt)}
            {:else if usageOn && usage?.signInError}
              <span title={usage.signInError}>Sign-in needed. See Settings.</span>
            {:else}
              {usageOn ? "Waiting for Claude Code to report it" : "Turned off in Settings"}
            {/if}
          </div>
        </div>
      </div>

      <div class="label strong heading">History</div>
      <div class="list" data-lenis-prevent>
        {#each feed.history.slice(0, 6) as h (h.at + h.sessionId)}
          <div class="hist">
            <div class="ellipsis" title={h.title}>{h.title}</div>
            <div class="status-word {h.outcome}">
              <Icon name={h.outcome === "done" ? "check" : "alert"} size={16} />{h.outcome === "done" ? "Done" : "Error"}<span class="label">· {ago(h.at, now)}</span>
            </div>
          </div>
        {:else}
          <div class="hist"><span class="label">Finished tasks appear here.</span></div>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .agent { display: flex; gap: var(--sp-16); flex: 1; min-height: 0; }
  /* Claude-inspired warm clay: accent, ring, headings and a soft tint on every card. */
  .clay {
    --c-accent: var(--clay);
    --c-accent-text: var(--clay);
    --c-accent-ink: var(--clay-ink);
  }
  .clay :is(.status-row, .answer, .list, .card) {
    background-image: linear-gradient(var(--clay-soft), var(--clay-soft));
  }
  .clay .heading,
  .clay .verb,
  .answer > .label {
    color: var(--clay);
  }
  .left { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-8); }
  .right { width: var(--size-agent-side); flex: none; display: flex; flex-direction: column; gap: var(--sp-8); min-height: 0; }
  .status-row {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    height: var(--size-control);
    padding: 0 var(--sp-4) 0 var(--sp-16);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    font-size: var(--fs-small);
    flex: none;
  }
  .grow { flex: 1; }
  .answer {
    flex: none;
    max-height: calc(var(--size-row) * 2.5);
    overflow-y: auto;
    padding: var(--sp-12) var(--sp-16);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    font-size: var(--fs-small);
    line-height: var(--lh-loose);
  }
  .answer {
    box-shadow: inset var(--focus-width) 0 0 var(--clay);
  }
  /* Claude's answers read in a warm serif, like a letter. */
  .answer p { margin: var(--sp-4) 0 0; white-space: pre-wrap; user-select: text; font-family: var(--font-reading); font-size: var(--fs-body); }
  .ask {
    flex: none;
    padding: var(--sp-12) var(--sp-16);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    box-shadow: inset var(--focus-width) 0 0 var(--c-accent);
    font-size: var(--fs-small);
  }
  .ask > .label { color: var(--c-accent-text); }
  .ask p { margin: var(--sp-4) 0 var(--sp-8); font-weight: var(--fw-strong); }
  .choices { display: flex; flex-wrap: wrap; gap: var(--sp-8); }
  .choice {
    height: 28px;
    padding: 0 var(--sp-12);
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--c-accent) 14%, transparent);
    color: var(--c-accent-text);
    font-size: var(--fs-small);
    font-weight: var(--fw-strong);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: background-color var(--dur-fast);
  }
  .choice.first { background: var(--c-accent); color: var(--c-accent-ink); }
  .choice:hover:not(:disabled) { background-color: color-mix(in srgb, var(--c-accent) 26%, transparent); }
  .choice.first:hover:not(:disabled) { filter: brightness(1.08); background-color: var(--c-accent); }
  .choice:disabled { opacity: 0.6; cursor: default; }
  .reply-error { margin-top: var(--sp-8); color: var(--c-error); }
  .icon-btn.small { width: var(--size-tab); height: var(--size-tab); }
  .heading { margin-top: var(--sp-8); }
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    padding: var(--sp-8);
    overflow-y: auto;
    min-height: 0;
  }
  .act { display: flex; align-items: center; gap: var(--sp-8); min-height: var(--size-activity-row); padding: 0 var(--sp-8); font-size: var(--fs-small); min-width: 0; }
  .act .label { display: flex; }
  .verb { font-weight: var(--fw-strong); flex: none; width: var(--size-verb); }
  .target { flex: 1; color: var(--c-muted); }
  .ringcard { display: flex; gap: var(--sp-16); align-items: center; flex: none; }
  .ringtext { min-width: 0; }
  .name { font-size: var(--fs-small); font-weight: var(--fw-strong); }
  .hist { padding: var(--sp-4) var(--sp-8); min-width: 0; font-size: var(--fs-small); }
  .hist .status-word .label { font-weight: var(--fw-regular); }
  /* Auto / Claude Code / Chats */
  .sources { display: flex; gap: var(--sp-4); flex: none; }
  .source {
    height: 24px;
    padding: 0 var(--sp-12);
    border-radius: var(--r-pill);
    font-size: var(--fs-small);
    color: var(--c-muted);
    background: transparent;
    transition: background-color var(--dur-fast), color var(--dur-fast);
  }
  .source:hover { background: color-mix(in srgb, var(--c-accent) 12%, transparent); }
  .source.on { background: var(--c-accent); color: var(--c-accent-ink); font-weight: var(--fw-strong); }
  .chat-head { display: flex; align-items: center; gap: var(--sp-8); flex: none; min-width: 0; padding: 0 var(--sp-4); }
  .chat-head .name { font-weight: var(--fw-strong); font-size: var(--fs-body); }
  .you {
    flex: none;
    max-height: calc(var(--size-row) * 1.5);
    overflow-y: auto;
    padding: var(--sp-8) var(--sp-16);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    font-size: var(--fs-small);
  }
  .you p { margin: var(--sp-4) 0 0; white-space: pre-wrap; user-select: text; }
  .answer.grow-answer { flex: 1 1 auto; max-height: none; min-height: 0; }
  .old { opacity: 0.55; }
  .status-row.behind { height: auto; min-height: var(--size-control); padding: var(--sp-8) var(--sp-16); }
  .dot-pulse { width: 8px; height: 8px; flex: none; border-radius: 50%; background: var(--clay); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.3; } }
  @media (prefers-reduced-motion: reduce) { .dot-pulse { animation: none; } }
  .plancard { display: flex; gap: var(--sp-12); align-items: center; flex: none; color: var(--clay); }
  .plancard .ringtext { color: var(--c-text); }
  .chat-item { display: block; width: 100%; text-align: left; border-radius: var(--r-control); }
  .chat-item:hover { background: color-mix(in srgb, var(--clay) 10%, transparent); }
  .chat-item.on { box-shadow: inset var(--focus-width) 0 0 var(--clay); }
</style>
