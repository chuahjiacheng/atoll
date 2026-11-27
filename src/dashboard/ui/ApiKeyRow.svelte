<!-- An API key kept in Windows Credential Manager (ask-assistant): paste, check with the provider, save; then Test / Change / Remove. -->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import type { IconName } from "../../lib/ui/Icon.svelte";
  import type { AskKeyStatus } from "../../lib/ask";
  import { confirm, toast } from "../store.svelte";
  import Row from "./Row.svelte";
  import Badge from "./Badge.svelte";

  let {
    name,
    icon,
    howTo,
    status,
    save,
    forget,
    test,
    onmodels,
  }: {
    /** "Gemini" or "Groq" */
    name: string;
    icon: IconName;
    /** Where the key comes from, shown before one is saved. */
    howTo: string;
    status: () => Promise<AskKeyStatus>;
    /** Checks the key with the provider and saves it; returns its models. */
    save: (key: string) => Promise<string[]>;
    forget: () => Promise<void>;
    /** Lists models with the saved key (proves it still works). */
    test: () => Promise<string[]>;
    onmodels: (models: string[]) => void;
  } = $props();

  let key = $state<AskKeyStatus | null>(null);
  let draft = $state("");
  let editing = $state(false);
  let busy = $state(false);
  let error = $state("");

  onMount(() => {
    status().then((k) => {
      key = k;
      if (k.hasKey) test().then(onmodels).catch(() => {});
    });
  });

  async function doSave() {
    if (!draft.trim() || busy) return;
    busy = true;
    error = "";
    try {
      onmodels(await save(draft));
      key = await status();
      draft = "";
      editing = false;
      toast(`${name} key works. Saved in Windows Credential Manager`);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function doTest() {
    busy = true;
    error = "";
    try {
      const models = await test();
      onmodels(models);
      toast(`${name} key works: ${models.length} ${models.length === 1 ? "model" : "models"} available`);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function doRemove() {
    confirm(`Remove the ${name} key?`, `${name} can't answer until you add a key again. The key is deleted from Windows Credential Manager.`, "Remove", async () => {
      await forget();
      key = await status();
      onmodels([]);
      toast(`${name} key removed`);
    });
  }
</script>

<Row
  label="{name} API key"
  size="big"
  {icon}
  help={key?.hasKey && !editing ? `Saved in Windows Credential Manager, not in the settings file (${key.masked}).` : howTo}
  note={error}
  noteIcon="alert"
  noteTone="err"
>
  {#if key?.hasKey && !editing}
    <Badge kind="connected" />
    <GButton label={busy ? "Testing…" : "Test"} disabled={busy} onclick={doTest} />
    <GButton label="Change" onclick={() => ((editing = true), (error = ""))} />
    <GButton label="Remove" onclick={doRemove} />
  {:else}
    <input
      class="d-field secret"
      type="password"
      autocomplete="off"
      spellcheck="false"
      placeholder="Paste your API key"
      aria-label="{name} API key"
      bind:value={draft}
      onkeydown={(e) => e.key === "Enter" && doSave()}
    />
    <GButton label={busy ? "Checking…" : "Save"} variant="filled" disabled={!draft.trim() || busy} onclick={doSave} />
    {#if editing}<GButton label="Cancel" onclick={() => ((editing = false), (draft = ""), (error = ""))} />{/if}
  {/if}
</Row>

<style>
  .secret {
    width: var(--size-text-field);
  }
</style>
