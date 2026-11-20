<!--
  Permission request peek (design: Island → perm / perm-long).
  Reads top to bottom in the order a decision needs: how risky (kicker), what it does
  (one sentence), where, the exact command or the change, any further reasons, then
  Deny / Allow. Risky or unchecked requests make Deny the main button.
  Kept from the first version: type a reply (sent to Claude as the reason for saying no)
  and "Answer in Claude" (hand it back). Keys while Atoll has focus: Esc = Deny, Enter = Allow.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { api, type Choice, type Finding, type Permission, type Risk } from "../api";
  import { truncFolder } from "../format";
  import { Input } from "glisse";
  import GButton from "../glass/GButton.svelte";
  import { icon as ricon, react } from "../glass/bridge";
  import Icon, { type IconName } from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";
  import { highlight } from "./highlight";
  import { rulesApi } from "../rules";

  let {
    request,
    folder,
    queued,
    now,
    tone,
    keys,
    focused,
  }: {
    request: Permission;
    folder: string;
    queued: number;
    now: number;
    tone: Tone;
    /** Global hotkeys that answer from anywhere, e.g. { allow: "Ctrl+Alt+Y", deny: "Ctrl+Alt+X" }. */
    keys: { allow: string; deny: string };
    /** Atoll has keyboard focus (then Esc / Enter work too). */
    focused: boolean;
  } = $props();

  let busy = $state(false);
  let error = $state("");
  let replying = $state(false);
  let reply = $state("");
  let host = $state<HTMLDivElement>();
  const field = () => host?.querySelector("input") ?? null;

  // Activity labels are "-ing" forms ("Running"); the fallback title reads as a plain verb.
  const VERBS: Record<string, string> = {
    Running: "Run",
    Editing: "Edit",
    Writing: "Write",
    Reading: "Read",
    Searching: "Search",
    Finding: "Find",
    Fetching: "Fetch",
    "Searching web": "Search the web for",
    Delegating: "Start an agent for",
    Planning: "Update its plan",
    Using: "Use",
  };

  // Used only when the app sent no analysis (an older listener): the first version's text check.
  const RISKY =
    /\brm\s+-\w*[rf]|\bdel\s+\/[sqf]|\b(rd|rmdir)\s+\/s|Remove-Item\b[^|;]*-(Recurse|Force)|git\s+reset\s+--hard|git\s+push\b[^|;]*(-f\b|--force)|git\s+clean\s+-\w*f|git\s+checkout\s+--\s|git\s+branch\s+-D|drop\s+(table|database)|truncate\s+table|\bformat\s+[a-z]:|\bmkfs|\bdd\s+if=|Stop-Computer|\bshutdown\b/i;

  /** Risky or unchecked: Deny is the main button and Enter does not allow. */
  export const risky = (r: Permission) =>
    r.intel ? r.intel.risk === "risky" || r.intel.risk === "unknown" : r.tool !== "Read" && RISKY.test(`${r.preview} ${r.detail}`);

  const intel = $derived(request.intel);
  const level = $derived<Risk>(intel?.risk ?? (risky(request) ? "risky" : "caution"));
  const danger = $derived(level === "risky" || level === "unknown");

  const KICKER: Record<Risk, { word: string; icon: IconName; tip: string }> = {
    safe: { word: "Low risk", icon: "shield", tip: "Only reads, prints or talks to this computer" },
    caution: { word: "Permission needed", icon: "shield", tip: "Normal work: changes files in the project, runs code or uses the network" },
    unknown: { word: "Couldn't check", icon: "alert", tip: "Atoll could not read all of this request, so treat it like a risky one" },
    risky: { word: "Risky", icon: "alert", tip: "Can destroy work, leave the project, change the system or run hidden code" },
  };
  const kicker = $derived(KICKER[level]);

  const title = $derived(intel?.summary || `${VERBS[request.label] ?? "Use"} ${request.detail || request.tool}`.trim());
  const full = $derived(request.preview || `${request.tool} ${request.detail}`);
  const secondsLeft = $derived(Math.max(0, Math.ceil((request.expiresAt - now) / 1000)));

  // Up to two reasons the sentence does not already give (worked out by the analysis).
  const reasons = $derived.by((): Finding[] => {
    if (!intel) return [];
    if (intel.reasons) return intel.reasons;
    const said = intel.summary.toLowerCase();
    // A reason the sentence already gives, worded a little differently ("can also overwrite"),
    // or about the same file or address, is not repeated.
    const repeats = (note: string) => {
      const n = note.toLowerCase().replace(/\.$/, "");
      const core = n.replace(/^(can|may) /, "");
      const subjects = n.split(/\s+/).filter((w) => w.length > 3 && /[/\\.]/.test(w)).map((w) => w.replace(/[,;()“”]/g, ""));
      return said.includes(n) || said.includes(core) || subjects.some((s) => said.includes(s));
    };
    const seen = new Set<string>();
    return intel.findings
      .filter((f) => f.risk !== "safe" || intel.risk === "safe")
      // The diff already shows the file being written.
      .filter((f) => !(intel.diff && f.risk === "caution" && (f.flag === "writesFile" || f.flag === "overwrite")))
      .filter((f) => !repeats(f.note))
      .filter((f) => !seen.has(f.note) && !!seen.add(f.note))
      .slice(0, 2);
  });

  const diff = $derived(intel?.diff);
  const outside = $derived(intel?.outside?.[0] ?? "");

  // A long command scrolls inside its block; say so, since the cut can hide what matters.
  let commandBox = $state<HTMLDivElement>();
  let hiddenLines = $state(0);
  $effect(() => {
    void codeLines;
    const box = commandBox;
    if (!box) return;
    const lineHeight = parseFloat(getComputedStyle(box).lineHeight) || 18;
    hiddenLines = Math.max(0, Math.round((box.scrollHeight - box.clientHeight) / lineHeight));
  });
  const codeLang = $derived(request.tool === "Bash" ? "bash_command" : request.tool === "PowerShell" ? "powershell_command" : request.preview.trimStart().startsWith("{") ? "json" : "");
  // Edits without an analysis fall back to the first version's text preview.
  const codeLines = $derived(diff ? [] : request.preview.split("\n"));

  /** Inside the session's folder: "src/lib/format.ts"; long paths: "…/lib/island/Peek.svelte". */
  function middle(path: string, max = 46) {
    let p = path.replace(/\\/g, "/");
    const base = folder.replace(/\\/g, "/").replace(/\/$/, "");
    if (base && p.toLowerCase().startsWith(`${base.toLowerCase()}/`)) p = p.slice(base.length + 1);
    if (p.length <= max) return p;
    const parts = p.split("/");
    let out = parts.pop() ?? p;
    while (parts.length && out.length + parts[parts.length - 1].length + 1 <= max - 2) out = `${parts.pop()}/${out}`;
    return `…/${out}`;
  }

  export async function decide(choice: Choice) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      await api.decidePermission(request.id, choice, choice === "deny" && reply.trim() ? reply.trim() : undefined);
    } catch (e) {
      error = `Atoll couldn't send your answer (${String(e).replace(/^Error:\s*/, "")}). Try again, or answer in Claude.`;
      busy = false;
    }
  }

  export function isTyping() {
    return replying && document.activeElement === field();
  }

  // Opens Settings → Permission rules with a rule drafted from this request.
  async function makeRule() {
    error = "";
    try {
      await rulesApi.fromRequest(request.id);
    } catch (e) {
      error = `Atoll couldn't start a rule (${String(e).replace(/^Error:\s*/, "")}).`;
    }
  }

  async function openReply() {
    replying = true;
    await tick();
    field()?.focus();
  }
</script>

<div class="peek" role="dialog" aria-label="Permission request from {request.project || 'Claude Code'}" data-risk={level}>
  <div class="top">
    <Pip mood="curious" {tone} size={40} />
    <div class="body">
      <div class="kicker" title={kicker.tip}>
        <span class="level"><Icon name={kicker.icon} size={16} /><span>{kicker.word}</span></span>
        <span class="flex"></span>
        <span class="meta tabular" title="When this runs out, Claude Code asks in its own window instead">
          {queued > 1 ? `1 of ${queued} · ` : ""}{secondsLeft} s left
        </span>
      </div>
      <div class="title" title={intel?.summary ? `${intel.summary}\n\n${full}` : full}>{title}</div>
      {#if outside}
        <div class="where outside" title={intel?.outside?.join("\n")}>
          <Icon name="alert" size={16} />
          <span class="project">Outside {request.project || "the project"}</span>
          <span class="ellipsis">{middle(outside)}{(intel?.outside?.length ?? 0) > 1 ? ` and ${(intel?.outside?.length ?? 1) - 1} more` : ""}</span>
        </div>
      {:else if request.project || folder}
        <div class="where" title={folder}>
          <Icon name="folder" size={16} />
          {#if request.project}<span class="project">{request.project}</span>{/if}
          {#if folder}<span class="ellipsis">{truncFolder(folder)}</span>{/if}
        </div>
      {/if}
      {#if request.rule}
        <div class="rule" title={`Your rule “${request.rule.ruleName}” matches, but it still asks you: ${request.rule.whyNot}.`}>
          <Icon name="shield" size={16} />
          <span class="clamp">Rule “{request.rule.ruleName}” matches. Still asks: {request.rule.whyNot}.</span>
        </div>
      {/if}
    </div>
  </div>

  {#if diff}
    <figure class="code diff" aria-label="Change to {diff.path}">
      <figcaption>
        <span class="path" title={diff.path}>{middle(diff.path)}</span>
        {#if diff.newFile}<span class="tag">new file</span>{/if}
        <span class="flex"></span>
        <span class="counts tabular">
          <span class="add" aria-hidden="true">+{diff.added}</span>{#if diff.removed || !diff.newFile}<span class="del" aria-hidden="true">−{diff.removed}</span>{/if}
          <span class="sr-only">{diff.added} {diff.added === 1 ? "line" : "lines"} added{diff.removed ? `, ${diff.removed} removed` : ""}</span>
        </span>
      </figcaption>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrolling region must take focus so it can be scrolled from the keyboard) -->
      <div class="lines" tabindex="0" role="region" aria-label="Changed lines">
        {#each diff.lines as line, i (i)}
          {#if line.kind === "gap"}
            <div class="line gap" aria-hidden="true"><span class="sign"></span><span class="text">⋯</span></div>
          {:else}
            <div class="line {line.kind}">
              <span class="sign" aria-hidden="true">{line.kind === "added" ? "+" : line.kind === "removed" ? "−" : ""}</span>
              {#if line.kind !== "context"}<span class="sr-only">{line.kind === "added" ? "Added: " : "Removed: "}</span>{/if}
              <span class="text">{#each highlight(line.text, diff.lang) as t, j (j)}<span class="t-{t.kind || 'plain'}">{t.text}</span>{/each}</span>
            </div>
          {/if}
        {/each}
        {#if diff.more}
          <div class="line note"><span class="sign"></span><span class="text">{diff.more} more changed {diff.more === 1 ? "line" : "lines"} not shown</span></div>
        {/if}
      </div>
      {#if diff.cut}<p class="cut">The change reached Atoll cut short; its end is not shown.</p>{/if}
    </figure>
  {:else if request.preview}
    <figure class="code command" aria-label={request.tool === "Bash" || request.tool === "PowerShell" ? "Command" : "Request details"}>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrolling region must take focus so it can be scrolled from the keyboard) -->
      <div class="lines" tabindex="0" role="region" aria-label="Full text" bind:this={commandBox}>
        {#each codeLines as text, i (i)}
          <div class="line"><span class="text">{#each highlight(text, codeLang) as t, j (j)}<span class="t-{t.kind || 'plain'}">{t.text}</span>{/each}</span></div>
        {/each}
      </div>
      {#if hiddenLines}<p class="cut">{hiddenLines} more {hiddenLines === 1 ? "line" : "lines"} below · scroll to read all of it</p>{/if}
    </figure>
  {/if}

  {#if reasons.length}
    <ul class="reasons">
      {#each reasons as r (r.note)}
        <li class="reason {r.risk}">
          <Icon name={r.risk === "risky" || r.risk === "unknown" ? "alert" : "dot"} size={16} />
          <span>{r.note.charAt(0).toUpperCase() + r.note.slice(1)}</span>
        </li>
      {/each}
    </ul>
  {/if}

  {#if replying}
    <form
      class="reply"
      onsubmit={(e) => {
        e.preventDefault();
        if (reply.trim()) decide("deny");
      }}
    >
      <div
        class="input-host"
        bind:this={host}
        use:react={{
          component: Input,
          props: {
            value: reply,
            onValueChange: (v: string) => (reply = v),
            placeholder: "Tell Claude what to do instead",
            "aria-label": "Reply to Claude",
            maxLength: 2000,
            icon: ricon("agent", 16),
          },
        }}
      ></div>
      <GButton icon="send" variant="filled" type="submit" ariaLabel="Send reply and deny" disabled={busy || !reply.trim()} />
    </form>
  {/if}

  <div class="actions">
    <GButton label="Deny" icon="x" grow variant={danger || replying ? "filled" : "glass"} disabled={busy} onclick={() => decide("deny")} />
    <GButton
      label="Allow"
      icon="check"
      grow
      variant={danger || replying ? "glass" : "filled"}
      destructive={danger && !replying}
      ariaLabel={danger && focused ? `Allow (Enter is off for this request; use ${keys.allow})` : undefined}
      disabled={busy}
      onclick={() => decide("allow")}
    />
  </div>
  <div class="keys" aria-hidden="true">
    <span>{focused ? "Esc" : keys.deny}</span>
    <!-- Risky: Enter does nothing, so the hint says so instead of silently switching keys. -->
    <span>{focused && !danger ? "Enter" : focused ? `${keys.allow} · Enter off` : keys.allow}</span>
  </div>

  <div class="extras">
    {#if !replying}
      <button class="link" onclick={openReply}>Reply instead</button>
    {/if}
    <button class="link" disabled={busy} onclick={makeRule} title="Open Settings with a rule for requests like this one">Make a rule…</button>
    <button class="link" disabled={busy} onclick={() => decide("pass")} title="Let Claude Code ask you in its own window">Answer in Claude</button>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .peek {
    padding: var(--sp-16);
    display: flex;
    flex-direction: column;
    gap: var(--sp-16);
  }
  .top { display: flex; gap: var(--sp-16); align-items: flex-start; }
  .body { display: flex; flex-direction: column; gap: var(--sp-8); min-width: 0; flex: 1; }
  .kicker {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
  }
  .level { display: inline-flex; align-items: center; gap: var(--sp-4); color: var(--c-accent-text); }
  [data-risk="risky"] .level { color: var(--c-error); }
  [data-risk="unknown"] .level { color: var(--c-error); }
  .flex { flex: 1; }
  .meta { color: var(--c-muted); font-weight: var(--fw-regular); }
  .title {
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .where { display: flex; align-items: center; gap: var(--sp-8); font-size: var(--fs-small); color: var(--c-muted); min-width: 0; }
  .where :global(.icon) { flex: none; }
  .where.outside { color: var(--c-ink); }
  .rule { display: flex; align-items: flex-start; gap: var(--sp-8); font-size: var(--fs-small); color: var(--c-muted); min-width: 0; }
  .rule :global(.icon) { flex: none; color: var(--c-accent-text); }
  .rule .clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; min-width: 0; }
  .where.outside :global(.icon) { color: var(--c-error); }
  .project { color: var(--c-ink); font-weight: var(--fw-strong); flex: none; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }

  /* ---- the command or the change ---- */
  .code {
    margin: 0;
    background: var(--c-raised);
    border-radius: var(--r-control);
    overflow: hidden;
    font-family: var(--font-code);
    font-size: var(--fs-code);
    line-height: 1.5;
  }
  .code figcaption {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    padding: var(--sp-8) 12px;
    font-family: var(--font);
    font-size: var(--fs-label);
    color: var(--c-muted);
  }
  .path { color: var(--c-ink); font-weight: var(--fw-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .tag { flex: none; padding: 0 var(--sp-8); border-radius: var(--r-pill); background: var(--c-highlight); }
  .counts { display: inline-flex; gap: var(--sp-8); font-weight: var(--fw-strong); flex: none; }
  .counts .add { color: var(--c-diff-sign-add); }
  .counts .del { color: var(--c-diff-sign-del); }
  .lines {
    box-sizing: border-box;
    overflow: auto;
    padding: var(--sp-8) 0;
    scrollbar-width: thin;
    scrollbar-color: var(--c-track) transparent;
  }
  .command .lines { max-height: calc(7 * 1.5em + 16px); }
  .diff .lines { max-height: 168px; padding-top: 0; }
  .lines:focus-visible { outline: var(--focus-width) solid var(--c-accent); outline-offset: calc(var(--focus-width) * -1); border-radius: var(--r-control); }
  .line { display: flex; min-width: max-content; padding-right: 12px; }
  /* Commands wrap so all of it is in view, at spaces where possible, with a hanging indent so
     a wrapped part never looks like a new line. Diffs keep their indentation and scroll sideways. */
  .command .line { padding-left: calc(12px + 2ch); min-width: 0; }
  .command .text { white-space: pre-wrap; overflow-wrap: break-word; text-indent: -2ch; }
  .sign { flex: none; width: 24px; text-align: center; user-select: none; color: var(--c-muted); }
  .text { white-space: pre; }
  .line.added { background: var(--c-diff-add); }
  .line.removed { background: var(--c-diff-del); }
  .line.added .sign { color: var(--c-diff-sign-add); }
  .line.removed .sign { color: var(--c-diff-sign-del); }
  .line.gap .text, .line.note .text { color: var(--c-muted); font-family: var(--font); }
  .t-key { color: var(--c-code-key); }
  .t-str { color: var(--c-code-str); }
  .t-dim { color: var(--c-code-dim); }
  .t-cmd { color: var(--c-ink); font-weight: var(--fw-strong); }
  .cut { margin: 0; padding: 0 12px var(--sp-8); font-family: var(--font); font-size: var(--fs-label); color: var(--c-muted); }

  /* ---- further reasons ---- */
  .reasons { list-style: none; margin: calc(var(--sp-4) * -1) 0 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-4); }
  .reason { display: flex; gap: var(--sp-8); align-items: flex-start; font-size: var(--fs-small); line-height: 1.4; color: var(--c-muted); }
  .reason :global(.icon) { flex: none; margin-top: 1px; }
  .reason.risky { color: var(--c-ink); }
  .reason.risky :global(.icon) { color: var(--c-error); }
  .reason.unknown { color: var(--c-ink); }
  .reason.unknown :global(.icon) { color: var(--c-warn); }

  .actions, .keys { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-8); }
  .keys { font-size: var(--fs-label); color: var(--c-muted); text-align: center; margin-top: calc(var(--sp-8) * -1); }
  .reply { display: flex; gap: var(--sp-8); }
  .input-host { flex: 1; display: flex; }
  .input-host :global(.lg-input) { max-width: none; }
  .extras { display: flex; justify-content: center; gap: var(--sp-24); margin-top: calc(var(--sp-8) * -1); }
  .link {
    min-height: 24px;
    padding: 0 var(--sp-4);
    font-size: var(--fs-small);
    color: var(--c-muted);
    text-decoration: underline;
    text-underline-offset: var(--sp-4);
    text-decoration-color: var(--c-dash);
  }
  .link:hover { color: var(--c-ink); }
  .error { margin: 0; color: var(--c-error); font-size: var(--fs-small); }
</style>
