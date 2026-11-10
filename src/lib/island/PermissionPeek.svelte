<!--
  Permission request peek (design: Island → perm / perm-long).
  Deny / Allow, plus two extras kept from the first version: type a reply (sent to Claude
  as the reason for saying no) and "Answer in Claude" (hand it back).
  Keys while Atoll has focus: Esc = Deny, Enter = Allow.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { api, type Choice, type Permission } from "../api";
  import { truncFolder } from "../format";
  import { Input } from "glisse";
  import GButton from "../glass/GButton.svelte";
  import { icon as ricon, react } from "../glass/bridge";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

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

  // Activity labels are "-ing" forms ("Running"); the request reads as a plain verb.
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
  const title = $derived(`${VERBS[request.label] ?? "Use"} ${request.detail || request.tool}`.trim());
  const full = $derived(request.preview || `${request.tool} ${request.detail}`);
  const secondsLeft = $derived(Math.max(0, Math.ceil((request.expiresAt - now) / 1000)));

  // Commands that can delete or overwrite work get a warning, and Deny becomes the main button.
  const RISKY =
    /\brm\s+-\w*[rf]|\bdel\s+\/[sqf]|\b(rd|rmdir)\s+\/s|Remove-Item\b[^|;]*-(Recurse|Force)|git\s+reset\s+--hard|git\s+push\b[^|;]*(-f\b|--force)|git\s+clean\s+-\w*f|git\s+checkout\s+--\s|git\s+branch\s+-D|drop\s+(table|database)|truncate\s+table|\bformat\s+[a-z]:|\bmkfs|\bdd\s+if=|Stop-Computer|\bshutdown\b/i;
  export const risky = (r: Permission) => r.tool !== "Read" && RISKY.test(`${r.preview} ${r.detail}`);
  const danger = $derived(risky(request));

  export async function decide(choice: Choice) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      await api.decidePermission(request.id, choice, choice === "deny" && reply.trim() ? reply.trim() : undefined);
    } catch (e) {
      error = String(e);
      busy = false;
    }
  }

  export function isTyping() {
    return replying && document.activeElement === field();
  }

  async function openReply() {
    replying = true;
    await tick();
    field()?.focus();
  }
</script>

<div class="peek" role="dialog" aria-label="Permission request from {request.project || 'Claude Code'}">
  <div class="top">
    <Pip mood="curious" {tone} size={40} />
    <div class="body">
      <div class="kicker" class:danger>
        <Icon name="alert" size={16} /><span>{danger ? "Can delete or overwrite work" : "Permission needed"}</span>
        <span class="flex"></span>
        <span class="meta tabular" title="When this runs out, Claude Code asks in its own window instead">
          {queued > 1 ? `1 of ${queued} · ` : ""}{secondsLeft} s left
        </span>
      </div>
      <div class="title" title={full}>{title}</div>
      {#if request.project}<div class="project">{request.project}</div>{/if}
      {#if folder}
        <div class="folder" title={folder}><Icon name="folder" size={16} /><span class="ellipsis">{truncFolder(folder)}</span></div>
      {/if}
    </div>
  </div>

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
    <GButton label="Deny" icon="x" grow variant={danger || replying ? "filled" : "glass"} destructive={danger && !replying} disabled={busy} onclick={() => decide("deny")} />
    <GButton label="Allow" icon="check" grow variant={danger || replying ? "glass" : "filled"} disabled={busy} onclick={() => decide("allow")} />
  </div>
  <div class="keys" aria-hidden="true">
    <span>{focused ? "Esc" : keys.deny}</span>
    <span>{focused && !danger ? "Enter" : keys.allow}</span>
  </div>

  <div class="extras">
    {#if !replying}
      <button class="link" onclick={openReply}>Reply instead</button>
    {/if}
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
  .kicker.danger { color: var(--c-error); }
  .project { font-size: var(--fs-small); font-weight: var(--fw-strong); margin-top: calc(var(--sp-4) * -1); }
  .kicker {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    color: var(--c-accent-text);
  }
  .flex { flex: 1; }
  .meta { color: var(--c-muted); font-weight: var(--fw-regular); }
  .title {
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    word-break: break-all;
  }
  .folder { display: flex; align-items: center; gap: var(--sp-8); font-size: var(--fs-small); color: var(--c-muted); min-width: 0; }
  .actions, .keys { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-8); }
  .keys { font-size: var(--fs-label); color: var(--c-muted); text-align: center; margin-top: calc(var(--sp-8) * -1); }
  .reply { display: flex; gap: var(--sp-8); }
  .input-host { flex: 1; display: flex; }
  .input-host :global(.lg-input) { max-width: none; }
  .extras { display: flex; justify-content: center; gap: var(--sp-24); margin-top: calc(var(--sp-8) * -1); }
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
