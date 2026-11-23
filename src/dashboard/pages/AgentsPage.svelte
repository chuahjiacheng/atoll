<!--
  AI agents (Claude Code only; the design's Codex section is left out by
  choice). Connection status and test, what the island shows, the hooks
  Atoll installs, the usage ring and the experimental Chat tab watcher.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, onUsage, type Usage } from "../../lib/api";
  import { settingsApi, type ClaudeStatus, type Settings } from "../../lib/settings";
  import { ago } from "../../lib/format";
  import GButton from "../../lib/glass/GButton.svelte";
  import { act, change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SelectRow from "../ui/SelectRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import SliderRow from "../ui/SliderRow.svelte";
  import Badge from "../ui/Badge.svelte";
  import CountUp from "../ui/CountUp.svelte";

  let { s }: { s: Settings } = $props();

  let status = $state<ClaudeStatus | null>(null);
  let test = $state<{ state: "" | "testing" | "ok" | "fail"; msg: string }>({ state: "", msg: "" });
  let hookBusy = $state(false);
  let usage = $state<Usage | null>(null);
  let watch = $state<{ running: boolean; lastSignalAt: number } | null>(null);
  let port = $state(0);
  let now = $state(Date.now());

  const refresh = async () => (status = await settingsApi.claudeStatus());

  onMount(() => {
    refresh();
    api.getUsage().then((u) => (usage = u));
    settingsApi.chatWatchStatus().then((w) => (watch = w));
    settingsApi.appInfo().then((i) => (port = i.port));
    const unUsage = onUsage((u) => (usage = u));
    const unSignal = listen<number>("chatwatch-signal", (e) => watch && (watch = { ...watch, lastSignalAt: e.payload }));
    // While this page is open, keep "last event" and the status fresh.
    const timer = setInterval(() => {
      now = Date.now();
      refresh();
    }, 5000);
    return () => {
      unUsage.then((f) => f());
      unSignal.then((f) => f());
      clearInterval(timer);
    };
  });

  // Turning Connect on or off changes the status straight away.
  $effect(() => {
    s.modules.claude;
    refresh();
  });

  const on = $derived(s.modules.claude);
  const state = $derived(!on ? "off" : (status?.state ?? "connecting"));
  const ok = $derived(state === "connected");
  const hooks = $derived(status?.hooks);
  const detail = $derived(
    {
      connected: `Connected. Its activity shows in the island.${status?.lastEventAt ? ` Last event ${ago(status.lastEventAt, now)}.` : ""}`,
      notfound: "Claude Code was not found on this PC.",
      error: "The hooks aren't installed, so Claude Code can't reach the island.",
      connecting: "Looking for Claude Code on this PC.",
      off: "Turn on Connect to show its activity in the island.",
    }[state] ?? "",
  );
  const note = $derived.by(() => {
    if (test.state === "testing") return { text: "Testing the connection.", icon: "busy" as const, tone: "ink" as const };
    if (test.state === "ok") return { text: `${test.msg} Checked just now.`, icon: "check" as const, tone: "ok" as const };
    if (test.state === "fail") return { text: test.msg, icon: "alert" as const, tone: "err" as const };
    if (state === "notfound") return { text: "Install Claude Code, then select Test connection.", icon: "alert" as const, tone: "ink" as const };
    if (state === "error") return { text: "Select Install hooks below, then Test connection.", icon: "alert" as const, tone: "ink" as const };
    return null;
  });

  async function runTest() {
    test = { state: "testing", msg: "" };
    try {
      test = { state: "ok", msg: await settingsApi.testConnection() };
    } catch (e) {
      test = { state: "fail", msg: String(e) };
    }
    refresh();
  }

  async function hooksDo(install: boolean) {
    hookBusy = true;
    await act(() => (install ? api.installHooks() : api.uninstallHooks()), install ? "Hooks installed" : "Hooks removed");
    await refresh();
    hookBusy = false;
  }

  const needs = (h: string) => (ok ? h : "Available once Claude Code is connected.");
  const pct = (n: number | undefined) => Math.round(n ?? 0);
</script>

<Section title="Claude Code">
  <Row
    label="Claude Code"
    size="big"
    icon="terminal"
    help={detail}
    note={note?.text ?? ""}
    noteIcon={note?.icon}
    noteTone={note?.tone}
    noteIndent
  >
    <Badge kind={state as "connected" | "notfound" | "error" | "connecting" | "off"} />
    <GButton label="Test connection" disabled={!on || test.state === "testing"} onclick={runTest} />
  </Row>
  <SwitchRow
    label="Connect"
    help={on ? "Turn off to stop showing Claude Code in the island. Atoll then answers no permission prompts." : "Turn on to follow Claude Code on this PC."}
    checked={on}
    onchange={(v) => {
      test = { state: "", msg: "" };
      change("modules.claude", v);
    }}
  />
  <SwitchRow
    label="Show live activity"
    help={needs("Shows what the agent is doing in the island. Off shows only its status.")}
    disabled={!ok}
    checked={s.claude.liveActivity}
    onchange={(v) => change("claude.liveActivity", v)}
  />
  <SwitchRow
    label="Ask before allowing actions"
    help={needs("Shows a permission request in the island first. Off leaves the decision to Claude Code.")}
    disabled={!ok}
    checked={s.claude.askPermissions}
    onchange={(v) => change("claude.askPermissions", v)}
  />
  <SwitchRow
    label="Play a sound when done"
    help="Uses the sound set in Alerts and sounds."
    checked={s.claude.soundOnDone}
    onchange={(v) => change("claude.soundOnDone", v)}
  />
  <SwitchRow
    label="Pop up when a task finishes"
    help="Off keeps the island closed and just shows Done in it, so nothing interrupts you."
    checked={s.claude.popupOnDone}
    onchange={(v) => change("claude.popupOnDone", v)}
  />
  <SelectRow
    label="Clear stuck messages"
    help="An error (like a usage limit) or a Needs you message clears itself after this long. You can always clear one sooner with the × on its status in the island, or its shortcut (Hotkeys)."
    value={String(s.claude.clearAfterMin ?? 15)}
    options={[
      { v: "1", label: "After 1 minute" },
      { v: "5", label: "After 5 minutes" },
      { v: "15", label: "After 15 minutes" },
      { v: "60", label: "After 1 hour" },
      { v: "0", label: "Never" },
    ]}
    onchange={(v) => change("claude.clearAfterMin", Number(v))}
  />
  <SwitchRow
    label="Show the usage ring"
    help={needs("Shows N/A when usage is not available.")}
    disabled={!ok}
    checked={s.modules.usageRing}
    onchange={(v) => change("modules.usageRing", v)}
  />
</Section>

<Section
  title="Claude chats"
  desc="Your claude.ai chats (Chat tab, any plan, Free included) in the Agent tab. Atoll reads the chat list and the chats you opened from Claude's own cache on this PC. Nothing is sent anywhere."
>
  <SwitchRow
    label="Show Claude chats"
    help="Free plans show chats only. On Pro and up, the Agent tab switches between Claude Code and your chats."
    checked={s.claude.showChats}
    onchange={(v) => change("claude.showChats", v)}
  />
  <SegRow
    label="Agent tab shows"
    help="Auto shows Claude Code while it's in use, otherwise your chats. You can also switch in the Agent tab."
    disabled={!s.claude.showChats}
    items={[
      { id: "auto", label: "Auto" },
      { id: "code", label: "Claude Code" },
      { id: "chat", label: "Chats" },
    ]}
    value={s.claude.chatSource}
    wide
    onchange={(v) => change("claude.chatSource", v)}
  />
</Section>

<Section
  title="Hooks"
  desc="Atoll adds its hooks to your Claude Code user settings. Your other hooks stay, a backup is saved on every change, and only new Claude Code sessions pick the change up."
>
  <Row
    label={hooks?.allInstalled ? `Installed for ${hooks.installedEvents.length} events` : hooks?.installedEvents.length ? "Partly installed" : "Not installed"}
    help={hooks?.settingsPath ?? ""}
    icon="file"
  >
    {#if hooks?.installedEvents.length}
      <GButton label="Remove" destructive disabled={hookBusy} onclick={() => hooksDo(false)} />
    {/if}
    <GButton label={hooks?.allInstalled ? "Reinstall" : "Install hooks"} variant="filled" disabled={hookBusy} onclick={() => hooksDo(true)} />
  </Row>
  <Row
    label="Local listener"
    help="Only reachable from this PC. Every request needs a random token made at launch."
    icon="lock"
    meta={port ? `127.0.0.1:${port}` : ""}
  />
</Section>

<Section title="Usage ring" desc="Your 5-hour and weekly Claude limits. They come from your Claude sign-in (below), from Claude Code's status line in a terminal, or from a session's log when a limit is hit. Pro, Max and Ultra plans only.">
  <SwitchRow
    label="Live percentage from your Claude sign-in"
    help="Asks Anthropic for your usage every 3 minutes with Claude Code's own sign-in, so the ring works in the desktop app too. When that sign-in runs out, Atoll renews it like Claude Code does and saves it back, so your terminal stays signed in."
    checked={s.usage.fromSignIn}
    disabled={!s.modules.usageRing}
    onchange={(v) => change("usage.fromSignIn", v)}
    note={s.usage.fromSignIn && usage?.signInError ? usage.signInError : ""}
    noteIcon="alert"
    noteTone="err"
  />
  <SwitchRow
    label="Show the ring in the collapsed island"
    checked={s.usage.showInPill}
    disabled={!s.modules.usageRing}
    onchange={(v) => change("usage.showInPill", v)}
  />
  <SelectRow
    label="Ring shows"
    value={s.usage.ringWindow}
    disabled={!s.modules.usageRing}
    options={[
      { v: "fiveHour", label: "5-hour limit" },
      { v: "sevenDay", label: "Weekly limit" },
    ]}
    onchange={(v) => change("usage.ringWindow", v)}
  />
  <SliderRow
    label="Warn me at"
    help="An alert when either limit passes this."
    value={s.alerts.usageThreshold}
    min={50}
    max={100}
    step={5}
    unit="%"
    unitWord="percent"
    onchange={(v) => change("alerts.usageThreshold", v)}
  />
  <Row
    label="Latest data"
    help={usage?.fiveHour || usage?.sevenDay ? `Updated ${ago(usage.updatedAt, now)}.` : "No data yet."}
    icon="clock"
    note={hooks?.statusLine === "other" ? "You have your own Claude Code status line, so Atoll left it alone and the ring stays empty." : ""}
    noteIndent
  >
    {#if usage?.fiveHour || usage?.sevenDay}
      <span class="nums">
        <span>5 h <CountUp value={pct(usage.fiveHour?.used)} suffix="%" /></span>
        <span>Week <CountUp value={pct(usage.sevenDay?.used)} suffix="%" /></span>
      </span>
    {/if}
  </Row>
</Section>

<Section title="Chat tab watcher (experimental)">
  <SwitchRow
    label="Alert when a Chat tab reply finishes"
    help="The Claude desktop Chat tab has no hooks. Atoll watches for the Claude window flashing its taskbar button, which may not happen on every reply."
    checked={s.modules.chatWatcher}
    onchange={(v) => change("modules.chatWatcher", v)}
  />
  <SwitchRow
    label="Only when the Claude window is in the background"
    checked={s.chatWatcher.onlyWhenUnfocused}
    disabled={!s.modules.chatWatcher}
    onchange={(v) => change("chatWatcher.onlyWhenUnfocused", v)}
  />
  <Row
    label="Last signal from the Claude app"
    help={watch ? (watch.lastSignalAt ? ago(watch.lastSignalAt, now) : "None yet.") : ""}
    note={watch && !watch.running ? "The watcher could not start. The log folder has the details." : ""}
    noteTone="err"
    noteIcon="alert"
  >
    <GButton label="Test alert" onclick={() => act(() => settingsApi.testAlert("chat"), "")} />
  </Row>
</Section>

<style>
  .nums {
    display: flex;
    gap: var(--sp-16);
    font-size: var(--fs-small);
    font-weight: var(--fw-strong);
  }
</style>
