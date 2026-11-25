<!-- Settings → Ask (ask-assistant): Gemini API key, model and chat options. -->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { AI_STUDIO_KEYS, askApi, type AskKeyStatus } from "../../lib/ask";
  import type { Settings } from "../../lib/settings";
  import { change, confirm, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SelectRow from "../ui/SelectRow.svelte";
  import Badge from "../ui/Badge.svelte";

  let { s }: { s: Settings } = $props();

  const RECOMMENDED = "gemini-flash-latest";

  let key = $state<AskKeyStatus | null>(null);
  let draft = $state("");
  let editing = $state(false);
  let saving = $state(false);
  let testing = $state(false);
  let keyError = $state("");
  let models = $state<string[]>([]);

  onMount(() => {
    askApi.keyStatus().then((k) => {
      key = k;
      if (k.hasKey) loadModels();
    });
  });

  async function loadModels() {
    try {
      models = await askApi.models();
    } catch {
      models = [];
    }
  }

  async function save() {
    if (!draft.trim() || saving) return;
    saving = true;
    keyError = "";
    try {
      models = await askApi.setKey(draft);
      key = await askApi.keyStatus();
      draft = "";
      editing = false;
      toast("Key works. Saved in Windows Credential Manager");
    } catch (e) {
      keyError = String(e);
    } finally {
      saving = false;
    }
  }

  async function test() {
    testing = true;
    keyError = "";
    try {
      models = await askApi.models();
      toast(`Key works: ${models.length} Gemini ${models.length === 1 ? "model" : "models"} available`);
    } catch (e) {
      keyError = String(e);
    } finally {
      testing = false;
    }
  }

  function remove() {
    confirm("Remove the Gemini key?", "Ask stops working until you add a key again. The key is deleted from Windows Credential Manager.", "Remove", async () => {
      await askApi.forgetKey();
      key = await askApi.keyStatus();
      models = [];
      toast("Gemini key removed");
    });
  }

  const modelOptions = $derived.by(() => {
    const ids = new Set([RECOMMENDED, s.ask.model, ...models]);
    return [...ids].map((v) => ({ v, label: v === RECOMMENDED ? `${v} (newest Flash, recommended)` : v }));
  });
</script>

<Section title="Gemini">
  <Row
    label="Gemini API key"
    size="big"
    icon="sparkle"
    help={key?.hasKey && !editing
      ? `Saved in Windows Credential Manager, not in the settings file (${key.masked}).`
      : "Free from Google AI Studio: sign in, choose Create API key, then paste it here. Atoll checks it with Google before saving."}
    note={keyError}
    noteIcon="alert"
    noteTone="err"
  >
    {#if key?.hasKey && !editing}
      <Badge kind="connected" />
      <GButton label={testing ? "Testing…" : "Test"} disabled={testing} onclick={test} />
      <GButton label="Change" onclick={() => ((editing = true), (keyError = ""))} />
      <GButton label="Remove" onclick={remove} />
    {:else}
      <input
        class="d-field secret"
        type="password"
        autocomplete="off"
        spellcheck="false"
        placeholder="Paste your API key"
        aria-label="Gemini API key"
        bind:value={draft}
        onkeydown={(e) => e.key === "Enter" && save()}
      />
      <GButton label={saving ? "Checking…" : "Save"} variant="filled" disabled={!draft.trim() || saving} onclick={save} />
      {#if editing}<GButton label="Cancel" onclick={() => ((editing = false), (draft = ""), (keyError = ""))} />{/if}
    {/if}
  </Row>
  {#if !key?.hasKey || editing}
    <Row label="Get a free key" help="Opens Google AI Studio in your browser.">
      <GButton label="Open AI Studio" icon="external" iconSize={16} onclick={() => askApi.openLink(AI_STUDIO_KEYS).catch((e) => toast(String(e), true))} />
    </Row>
  {/if}
  <SelectRow
    label="Model"
    help="Flash models are fast and have the most generous free limits. Pro models are smarter but allow far fewer questions a day."
    value={s.ask.model}
    options={modelOptions}
    onchange={(v) => change("ask.model", v)}
  />
</Section>

<Section title="Chat" desc="On Google's free tier, what you send may be used to improve Google's products. Don't paste passwords or private data.">
  <SwitchRow label="Ask in the island" help="Same as the Ask switch on Modules." checked={s.modules.ask} onchange={(v) => change("modules.ask", v)} />
  <SwitchRow
    label="Keep chat history"
    help="Saved on this PC so the chat is still there after a restart. A new chat starts by itself after 30 minutes without a question. Off: forgotten when Atoll closes."
    checked={s.ask.keepHistory}
    onchange={(v) => change("ask.keepHistory", v)}
  />
  <Row label="Clear the chat" help="Starts a new chat in the island.">
    <GButton label="Clear" onclick={() => askApi.clear().then(() => toast("Chat cleared"))} />
  </Row>
</Section>

<style>
  .secret {
    width: var(--size-text-field);
  }
</style>
