<script lang="ts">
  import { onMount } from "svelte";
  import { Input } from "glisse";
  import { react } from "../../lib/glass/bridge";
  import GButton from "../../lib/glass/GButton.svelte";
  import { calendarApi, onCalendarStatus, type CalendarStatus } from "../../lib/calendar";
  import type { Settings } from "../../lib/settings";
  import { change, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import Badge from "../ui/Badge.svelte";

  let { s }: { s: Settings } = $props();

  let status = $state<CalendarStatus | null>(null);
  let clientId = $state("");
  let secret = $state("");
  let editSecret = $state(false);

  $effect(() => {
    clientId = s.calendar.clientId;
  });

  onMount(() => {
    calendarApi.status().then((st) => (status = st));
    const un = onCalendarStatus((st) => {
      if (status?.signingIn && st.connected && !status.connected) toast("Google Calendar connected");
      status = st;
    });
    return () => un.then((f) => f());
  });

  const connected = $derived(!!status?.connected);
  const signingIn = $derived(!!status?.signingIn);
  const hasSecret = $derived(!!status?.hasSecret);
  const badge = $derived(connected ? "connected" : signingIn ? "connecting" : "off");
  const idBad = $derived(clientId.trim() !== "" && !/^[0-9A-Za-z._-]+$/.test(clientId.trim()));
  const idOdd = $derived(!idBad && clientId.trim() !== "" && !clientId.trim().endsWith(".apps.googleusercontent.com"));
  // Your own Google client (Advanced); empty = Atoll's built-in one.
  const ownClient = $derived(!!s.calendar.clientId);
  let advanced = $state(false);
  $effect(() => {
    if (ownClient) advanced = true;
  });

  function saveId(v: string) {
    clientId = v;
    const t = v.trim();
    if (!idBad && t !== s.calendar.clientId) change("calendar.clientId", t);
  }

  async function saveSecret() {
    try {
      await calendarApi.setSecret(secret);
      secret = "";
      editSecret = false;
      toast("Client secret saved in Windows Credential Manager");
    } catch (e) {
      toast(String(e), true);
    }
  }

  async function connect() {
    toast("Opening Google sign-in");
    // Let the client ID save land before Rust reads it.
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    try {
      await calendarApi.login();
    } catch (e) {
      toast(String(e), true);
    }
  }

  async function disconnect() {
    await calendarApi.logout();
    toast("Google Calendar disconnected");
  }

  function show(id: string, on: boolean) {
    // The whole map: calendar ids contain dots, which a dotted path would split.
    const shown = Object.fromEntries((status?.calendars ?? []).map((c) => [c.id, c.on]));
    shown[id] = on;
    change("calendar.shown", shown);
    if (status) status = { ...status, calendars: status.calendars.map((c) => (c.id === id ? { ...c, on } : c)) };
  }
</script>

<Section title="Connection">
  <Row
    label="Google Calendar"
    size="big"
    icon="calendar"
    help={connected
      ? `Connected${status?.account ? ` as ${status.account}` : ""}. Read-only: Atoll never changes your calendar. The sign-in is kept in Windows Credential Manager.`
      : signingIn
        ? "Finish signing in to Google in your browser."
        : "Connect opens Google in your browser. Pick your account and allow Atoll to see your calendar. Read-only."}
    note={status?.error && !signingIn ? status.error : ""}
    noteIcon="alert"
    noteTone="err"
  >
    <Badge kind={badge} />
    {#if connected}
      <GButton label="Refresh" icon="reset" iconSize={16} onclick={() => calendarApi.refresh().then(() => toast("Refreshing"))} />
      <GButton label="Disconnect" onclick={disconnect} />
    {:else if signingIn}
      <GButton label="Cancel" onclick={() => calendarApi.cancelLogin().then(() => toast("Sign-in cancelled"))} />
    {:else}
      <GButton label="Connect" variant="filled" disabled={ownClient && (idBad || !hasSecret)} onclick={connect} />
    {/if}
  </Row>
</Section>

{#if connected && status}
  <Section title="Calendars" desc="Choose which calendars the island shows. New calendars follow your choice in Google Calendar until you set them here.">
    {#each status.calendars as c (c.id)}
      <SwitchRow label={c.name} help={c.primary ? "Your main calendar" : ""} checked={c.on} onchange={(v) => show(c.id, v)}>
        {#snippet lead()}<span class="swatch" style:background={c.color} aria-hidden="true"></span>{/snippet}
      </SwitchRow>
    {:else}
      <Row label="Loading your calendars…" />
    {/each}
  </Section>
{/if}

{#if !connected && !signingIn}
  <Section title="Advanced">
    <SwitchRow
      label="Use my own Google client"
      help="Optional. Atoll connects with its own Google client; turn this on only if you'd rather use one you created in Google Cloud."
      checked={advanced}
      onchange={(v) => {
        advanced = v;
        if (!v && s.calendar.clientId) change("calendar.clientId", "");
      }}
    />
  </Section>
{/if}

{#if !connected && !signingIn && advanced}
  <Section title="Your Google client" desc="A Desktop app OAuth client from Google Cloud, with the Google Calendar API enabled.">
    <Row
      label="Client ID"
      help="Ends in .apps.googleusercontent.com."
      note={idBad ? "A client ID has only letters, numbers, dots and dashes." : idOdd ? "Google client IDs usually end in .apps.googleusercontent.com." : ""}
      noteIcon="alert"
      noteTone={idBad ? "err" : "muted"}
    >
      <span
        class="field"
        class:bad={idBad}
        use:react={{
          component: Input,
          props: {
            value: clientId,
            placeholder: "Paste your client ID",
            spellCheck: false,
            "aria-label": "Client ID",
            "aria-invalid": idBad,
            onValueChange: saveId,
          },
        }}
      ></span>
    </Row>
    <Row label="Client secret" help={hasSecret && !editSecret ? "Saved in Windows Credential Manager, not in the settings file." : "Shown next to the client ID in Google Cloud. Kept in Windows Credential Manager."}>
      {#if hasSecret && !editSecret}
        <Badge kind="connected" />
        <GButton label="Change" onclick={() => (editSecret = true)} />
      {:else}
        <input
          class="d-field secret"
          type="password"
          autocomplete="off"
          spellcheck="false"
          placeholder="Paste your client secret"
          aria-label="Client secret"
          bind:value={secret}
          onkeydown={(e) => e.key === "Enter" && secret.trim() && saveSecret()}
        />
        <GButton label="Save" disabled={!secret.trim()} onclick={saveSecret} />
        {#if editSecret}<GButton label="Cancel" onclick={() => ((editSecret = false), (secret = ""))} />{/if}
      {/if}
    </Row>
  </Section>
{/if}

<Section title="In the island">
  <SwitchRow label="Calendar in the island" help="Same as the Calendar switch on Modules." checked={s.modules.calendar} onchange={(v) => change("modules.calendar", v)} />
  <SegRow
    label="Days to show"
    help="Today included."
    items={[
      { id: "1", label: "Today" },
      { id: "3", label: "3 days" },
      { id: "7", label: "Week" },
      { id: "14", label: "2 weeks" },
    ]}
    value={String(s.calendar.daysAhead)}
    disabled={!s.modules.calendar}
    onchange={(v) => change("calendar.daysAhead", Number(v))}
  />
  <SegRow
    label="Upcoming event in the pill"
    help="The collapsed island names your next event this long before it starts. Claude needing you, errors and the focus timer still come first."
    items={[
      { id: "0", label: "Off" },
      { id: "5", label: "5 min" },
      { id: "10", label: "10 min" },
      { id: "15", label: "15 min" },
      { id: "30", label: "30 min" },
    ]}
    value={String(s.calendar.pillMinutes)}
    disabled={!s.modules.calendar}
    onchange={(v) => change("calendar.pillMinutes", Number(v))}
  />
  <SwitchRow
    label="Show declined events"
    help="Struck through and dimmed."
    checked={s.calendar.showDeclined}
    disabled={!s.modules.calendar}
    onchange={(v) => change("calendar.showDeclined", v)}
  />
</Section>

<style>
  .field {
    display: contents;
  }
  .field :global(.lg-input) {
    width: var(--size-text-field);
    border-radius: var(--r-control);
    background: var(--c-field);
    box-shadow: inset 0 0 0 1px var(--c-ctrl);
  }
  .field.bad :global(.lg-input) {
    box-shadow: inset 0 0 0 var(--focus-width) var(--c-err);
  }
  .secret {
    width: var(--size-text-field);
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 4px;
    flex: none;
  }
</style>
