<!-- Settings → Ask (ask-assistant): who answers (Gemini, Groq or a local Ollama model), keys, models, usage and chat options. -->
<script lang="ts">
  import { onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { AI_STUDIO_KEYS, GROQ_KEYS, OLLAMA_DOWNLOAD, askApi, onAsk, type AskUsage } from "../../lib/ask";
  import type { Settings } from "../../lib/settings";
  import { change, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import SwitchRow from "../ui/SwitchRow.svelte";
  import SelectRow from "../ui/SelectRow.svelte";
  import SegRow from "../ui/SegRow.svelte";
  import Badge from "../ui/Badge.svelte";
  import ApiKeyRow from "../ui/ApiKeyRow.svelte";

  let { s }: { s: Settings } = $props();

  const RECOMMENDED = "gemini-flash-lite-latest";

  let geminiModels = $state<string[]>([]);
  let groqModels = $state<string[]>([]);
  let ollama = $state<{ running: boolean; models: string[]; error: string } | null>(null);
  let usage = $state<AskUsage | null>(null);

  onMount(() => {
    checkOllama();
    askApi.usage().then((u) => (usage = u));
    // Counts change after every answer.
    const un = onAsk((v) => {
      if (!v.busy) askApi.usage().then((u) => (usage = u));
    });
    return () => un.then((f) => f());
  });

  async function checkOllama() {
    try {
      ollama = { running: true, models: await askApi.ollamaModels(), error: "" };
    } catch (e) {
      ollama = { running: false, models: [], error: String(e) };
    }
  }

  const open = (url: string) => askApi.openLink(url).catch((e) => toast(String(e), true));

  const geminiOptions = $derived.by(() => {
    const ids = new Set([RECOMMENDED, s.ask.model, ...geminiModels]);
    return [...ids].map((v) => ({ v, label: v === RECOMMENDED ? `${v} (fastest, recommended)` : v }));
  });
  const groqOptions = $derived([
    { v: "", label: "Automatic (fastest good model)" },
    ...[...new Set([...(s.ask.groqModel ? [s.ask.groqModel] : []), ...groqModels])].map((v) => ({ v, label: v })),
  ]);
  const ollamaOptions = $derived([
    { v: "", label: ollama?.models[0] ? `Automatic (${ollama.models[0]})` : "Automatic (first installed)" },
    ...[...new Set([...(s.ask.ollamaModel ? [s.ask.ollamaModel] : []), ...(ollama?.models ?? [])])].map((v) => ({ v, label: v })),
  ]);
  const local = $derived(s.ask.provider === "ollama");
</script>

<Section title="Who answers">
  <SegRow
    label="Main provider"
    help={local
      ? "A model running on this PC with Ollama: unlimited, works offline, and nothing leaves this PC. Slower and less capable than the cloud."
      : "Free cloud models. Gemini is the most capable; Groq is very fast. Both have daily free limits."}
    items={[
      { id: "gemini", label: "Gemini" },
      { id: "groq", label: "Groq" },
      { id: "ollama", label: "Local (Ollama)" },
    ]}
    value={s.ask.provider}
    onchange={(v) => change("ask.provider", v)}
  />
  {#if !local}
    <SwitchRow
      label="Use the other as backup"
      help={s.ask.provider === "groq"
        ? "When Groq is busy, out of free requests or unreachable, Gemini answers instead (needs a Gemini key)."
        : "When every Gemini model is busy or out of free requests, Groq answers instead (needs a Groq key)."}
      checked={s.ask.backup}
      onchange={(v) => change("ask.backup", v)}
    />
  {/if}
</Section>

<Section title="Gemini" desc="Google's models. Free key from Google AI Studio.">
  <ApiKeyRow
    name="Gemini"
    icon="sparkle"
    howTo="Free from Google AI Studio: sign in, choose Create API key, then paste it here. Atoll checks it with Google before saving."
    status={askApi.keyStatus}
    save={askApi.setKey}
    forget={askApi.forgetKey}
    test={askApi.models}
    onmodels={(m) => (geminiModels = m)}
  />
  <Row label="Get a free Gemini key" help="Opens Google AI Studio in your browser.">
    <GButton label="Open AI Studio" icon="external" iconSize={16} onclick={() => open(AI_STUDIO_KEYS)} />
  </Row>
  <SelectRow
    label="Gemini model"
    help="If it hasn't started answering within a moment, Ask also asks the other Flash models your key can use and shows whichever starts first. Flash and Pro models are smarter but slower."
    value={s.ask.model}
    options={geminiOptions}
    onchange={(v) => change("ask.model", v)}
  />
</Section>

<Section title="Groq" desc="Open models on very fast hardware. Free key from the Groq console.">
  <ApiKeyRow
    name="Groq"
    icon="sparkle"
    howTo="Free from the Groq console: sign in, choose Create API Key, then paste it here. Atoll checks it with Groq before saving."
    status={askApi.groqKeyStatus}
    save={askApi.setGroqKey}
    forget={askApi.forgetGroqKey}
    test={askApi.groqModels}
    onmodels={(m) => (groqModels = m)}
  />
  <Row label="Get a free Groq key" help="Opens the Groq console in your browser.">
    <GButton label="Open Groq console" icon="external" iconSize={16} onclick={() => open(GROQ_KEYS)} />
  </Row>
  <SelectRow label="Groq model" help="Automatic picks the fastest good model your key has." value={s.ask.groqModel} options={groqOptions} onchange={(v) => change("ask.groqModel", v)} />
</Section>

<Section title="Local (Ollama)" desc="Runs a model on this PC. Free and unlimited; it needs a good graphics card or plenty of memory.">
  <Row
    label="Ollama"
    size="big"
    icon="laptop"
    help={!ollama
      ? "Checking…"
      : ollama.running
        ? ollama.models.length
          ? `Running, with ${ollama.models.length} ${ollama.models.length === 1 ? "model" : "models"} installed.`
          : "Running, but no models yet. In a terminal, run: ollama pull qwen3.5:9b"
        : "Not running. Install Ollama, start it, then pull a model (for example: ollama pull qwen3.5:9b)."}
  >
    <Badge kind={ollama?.running ? "connected" : "off"} />
    <GButton label="Check again" icon="reset" iconSize={16} onclick={checkOllama} />
    {#if ollama && !ollama.running}<GButton label="Get Ollama" icon="external" iconSize={16} onclick={() => open(OLLAMA_DOWNLOAD)} />{/if}
  </Row>
  <SelectRow label="Local model" help="Thinking is turned off so answers start in seconds." value={s.ask.ollamaModel} options={ollamaOptions} onchange={(v) => change("ask.ollamaModel", v)} />
</Section>

<Section title="Used today" desc="Questions answered today, counted on this PC. Free limits are set per model by each provider and change over time.">
  <Row label="Gemini" help="Google doesn't report what's left, so only answers are counted.">
    <span class="count tabular">{usage?.gemini ?? 0}</span>
  </Row>
  <Row
    label="Groq"
    help={usage?.groqLimit != null
      ? `Groq says ${usage.groqRemaining ?? "?"} of ${usage.groqLimit} requests are left today for the model last used.`
      : "Groq reports what's left after its first answer."}
  >
    <span class="count tabular">{usage?.groq ?? 0}</span>
  </Row>
  <Row label="Local" help="Unlimited.">
    <span class="count tabular">{usage?.ollama ?? 0}</span>
  </Row>
</Section>

<Section title="Chat" desc={local ? "With Local, questions never leave this PC." : "On free cloud tiers, what you send may be used to improve the provider's products. Don't paste passwords or private data."}>
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
  .count {
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
    min-width: 3ch;
    text-align: right;
  }
</style>
