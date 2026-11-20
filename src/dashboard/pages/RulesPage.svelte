<!--
  Permission rules (permission-intel, Stage C). Three parts, in the order you need them:
  1. your rules, each with one switch that stops it at once;
  2. the editor: what the rule covers, its dry run over past requests, and auto-allow
     (only after a dry run of the rule as it is now, and always with an end date);
  3. every decision a rule made, newest first, each with "Turn off rule".
  A rule never answers a risky or unchecked request and never allows more than its
  dry run saw you allow: the Rust side enforces that; this page only explains it.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { ago } from "../../lib/format";
  import { DAY, blankRule, fingerprint, onRulesChanged, rulesApi, toolLabel, type Answer, type Rule, type RulesView } from "../../lib/rules";
  import { confirm, toast } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import Seg from "../ui/Seg.svelte";
  import Switch from "../ui/Switch.svelte";

  let view = $state<RulesView | null>(null);
  let log = $state<Answer[]>([]);
  let now = $state(Date.now());
  /** The rule being made or changed (a copy; nothing is saved until "Save rule"). */
  let editing = $state<Rule | null>(null);
  let patternText = $state("");
  let running = $state(false);
  let saving = $state(false);
  let deniedOk = $state(false);
  let days = $state("7");
  /** The rule's own project, so "Where" can switch back to it after "Every project". */
  let home = $state<string | null>(null);
  /** The button that opened the editor; focus returns there when it closes. */
  let opener: HTMLElement | null = null;
  let editor = $state<HTMLElement>();

  async function load() {
    try {
      [view, log] = await Promise.all([rulesApi.list(), rulesApi.log(50)]);
      now = Date.now();
    } catch (e) {
      toast(String(e), true);
    }
  }

  async function takeDraft() {
    const d = await rulesApi.takeDraft().catch(() => null);
    if (d) open(d, true);
  }

  let unlisten: (() => void) | undefined;
  onMount(async () => {
    await load();
    await takeDraft();
    unlisten = await onRulesChanged(load);
  });
  onDestroy(() => unlisten?.());

  /** Called by the window when the peek's "Make a rule…" sends us here again. */
  export function draftArrived() {
    takeDraft();
  }

  /** The editor offers 1, 7 or 30 days; an existing rule shows the closest. */
  function nearestDays(left: number): string {
    return [1, 7, 30].reduce((a, b) => (Math.abs(b - left) < Math.abs(a - left) ? b : a)).toString();
  }

  function open(rule: Rule, fromRequest = false) {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    editing = structuredClone($state.snapshot(rule)) as Rule;
    patternText = rule.pattern.join("\n");
    deniedOk = false;
    home = rule.project;
    days = rule.expiresAt ? nearestDays((rule.expiresAt - Date.now()) / DAY) : "7";
    if (fromRequest) toast("Rule drafted from the request: check it, then run the dry run");
    queueMicrotask(() => editor?.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "start" }));
    if (fromRequest) dryRun();
  }

  function close() {
    editing = null;
    const back = opener;
    opener = null;
    queueMicrotask(() => (back?.isConnected ? back : document.querySelector<HTMLElement>(".add button"))?.focus());
  }

  const pattern = $derived(patternText.split("\n").map((p) => p.trim()).filter(Boolean));
  const draft = $derived(editing ? { ...editing, pattern } : null);
  /** The dry run still describes the rule as it is in the editor. */
  const dryCurrent = $derived(!!draft?.dryRun && draft.dryRun.fingerprint === fingerprint(draft));
  const canAuto = $derived(dryCurrent && (draft?.dryRun?.shapes.length ?? 0) > 0);

  const PATTERN_HELP: Record<string, string> = {
    Bash: "One per line: a program, or a program and its subcommand (npm test, cargo build, tail). Shells and wrappers like bash, sudo or env are not allowed.",
    files: "One per line: folders inside the project (src/**, docs). Files outside the project never match.",
    WebFetch: "One per line: a site (docs.rs) or every site under a domain (*.rust-lang.org).",
  };
  const patternHelp = $derived(editing ? (PATTERN_HELP[editing.tool] ?? "This rule covers every use of this tool; there is nothing more to choose.") : "");
  const autoOn = $derived(!!editing?.autoAllow && canAuto);
  const until = $derived(Date.now() + Number(days) * DAY);
  const needsOk = $derived(autoOn && (editing?.dryRun?.denied ?? 0) > 0);
  const needsPattern = $derived(!!editing && editing.tool in PATTERN_HELP);

  async function dryRun() {
    if (!draft) return;
    running = true;
    try {
      const result = await rulesApi.dryRun($state.snapshot(draft) as Rule);
      if (editing) editing.dryRun = result;
      deniedOk = false;
    } catch (e) {
      toast(String(e), true);
    } finally {
      running = false;
    }
  }

  function setAuto(on: boolean) {
    if (!editing) return;
    editing.autoAllow = on && canAuto;
  }

  async function save() {
    if (!draft || !editing) return;
    const rule = $state.snapshot(draft) as Rule;
    rule.expiresAt = rule.autoAllow ? Date.now() + Number(days) * DAY : null;
    if (rule.autoAllow && (rule.dryRun?.denied ?? 0) > 0 && !deniedOk) {
      toast("Tick the box to confirm: you said no to some of these requests before", true);
      return;
    }
    saving = true;
    try {
      await rulesApi.save(rule);
      toast(rule.id ? "Rule saved" : "Rule added");
      await load();
      close();
    } catch (e) {
      toast(String(e), true);
    } finally {
      saving = false;
    }
  }

  async function setEnabled(rule: Rule, on: boolean) {
    try {
      if (on) await rulesApi.save({ ...$state.snapshot(rule), enabled: true } as Rule);
      else await rulesApi.disable(rule.id);
      toast(on ? "Rule on" : "Rule off: it no longer acts");
      await load();
    } catch (e) {
      toast(String(e), true);
    }
  }

  async function turnOff(ruleId: string) {
    try {
      await rulesApi.disable(ruleId);
      toast("Rule off: it no longer acts");
      await load();
    } catch (e) {
      toast(String(e), true);
    }
  }

  function remove(rule: Rule) {
    confirm("Delete this rule?", `“${rule.name}” is removed. Its past decisions stay in the log.`, "Delete rule", async () => {
      await rulesApi.remove(rule.id);
      if (editing?.id === rule.id) editing = null;
      await load();
    });
  }

  const dateText = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" });
  const projectName = (p: string | null) => (p ? (p.split("/").filter(Boolean).pop() ?? p) : "every project");

  function effect(r: Rule): { text: string; tone: string } {
    if (!r.enabled) return { text: "Paused: no hints, no answers", tone: "" };
    if (r.autoAllow && r.expiresAt && r.expiresAt > now) return { text: `Allows without asking until ${dateText(r.expiresAt)}`, tone: "ok" };
    if (r.autoAllow) return { text: "Auto-allow ended: asks again", tone: "warn" };
    return { text: "Hint only: still asks you", tone: "ink" };
  }

  const ruleById = (id?: string) => view?.rules.find((r) => r.id === id);
</script>

{#if view?.error}
  <p class="banner" role="alert"><Icon name="alert" size={18} /><span>{view.error}</span></p>
{/if}

<Section title="Your rules" desc="A rule notices a kind of request you answer often. By default it only says so in the peek; it can answer for you only if you turn that on, for a set time.">
  {#if view && view.rules.length === 0}
    <Row icon="shield" label="No rules yet" help="Make one from a waiting request with “Make a rule…” in the island, or start one here." />
  {/if}
  {#each view?.rules ?? [] as r (r.id)}
    {@const fx = effect(r)}
    <Row
      icon={r.tool === "files" ? "folder" : r.tool === "WebFetch" ? "external" : "terminal"}
      label={r.name}
      help={`${toolLabel(r.tool)}${r.pattern.length ? `: ${r.pattern.join(", ")}` : ""} · ${projectName(r.project)}`}
    >
      <GButton label="Edit" ariaLabel={`Edit rule: ${r.name}`} onclick={() => open(r)} />
      <Switch checked={r.enabled} label={`${r.name}: on or off`} onchange={(v) => setEnabled(r, v)} />
      {#snippet below()}
        <p class="status" data-tone={fx.tone}>
          <Icon name={fx.tone === "ok" ? "check" : fx.tone === "warn" ? "clock" : r.enabled ? "info" : "x"} size={16} />
          <span>{fx.text}</span>
          {#if r.hits}<span class="uses tabular">· used {r.hits} {r.hits === 1 ? "time" : "times"}, last {ago(r.lastHit ?? 0, now)}</span>{/if}
        </p>
      {/snippet}
    </Row>
  {/each}
  {#if !editing}
    <Row line={false}>
      {#snippet below()}
        <div class="add"><GButton label="Add a rule" icon="plus" iconSize={16} onclick={() => open(blankRule())} /></div>
      {/snippet}
    </Row>
  {/if}
</Section>

{#if editing}
  <section class="d-section" bind:this={editor} aria-label={editing.id ? "Change rule" : "New rule"}>
    <h2>{editing.id ? "Change rule" : "New rule"}</h2>
    <div class="d-card">
      <Row label="Name" icon="edit">
        <input class="d-field name" bind:value={editing.name} maxlength="60" aria-label="Rule name" />
      </Row>
      <Row label="Covers" icon="shield" help={editing.tool in PATTERN_HELP ? "" : toolLabel(editing.tool)}>
        {#if editing.tool in PATTERN_HELP}
          <Seg
            label="What the rule covers"
            items={[{ id: "Bash", label: "Commands" }, { id: "files", label: "File edits" }, { id: "WebFetch", label: "Web pages" }]}
            value={editing.tool}
            wide
            onchange={(v) => editing && (editing.tool = v)}
          />
        {/if}
      </Row>
      <Row label="Where" icon="folder" help={editing.project ? editing.project : "Sessions started in any folder"}>
        <Seg
          label="Which projects"
          items={[
            { id: "this", label: home ? projectName(home) : "One project" },
            { id: "all", label: "Every project" },
          ]}
          value={editing.project ? "this" : "all"}
          disabled={!home}
          wide
          onchange={(v) => editing && (editing.project = v === "this" ? home : null)}
        />
      </Row>
      {#if needsPattern}
        <Row label="Which ones" icon="text" help={patternHelp}>
          {#snippet below()}
            <textarea class="d-field pattern" rows="4" bind:value={patternText} aria-label="Commands, folders or sites the rule covers" spellcheck="false"></textarea>
          {/snippet}
        </Row>
      {/if}

      <Row label="Dry run" icon="play" help="Replays the rule over past requests, so you see what it would have done before it does anything.">
        <GButton label={running ? "Running…" : dryCurrent ? "Run again" : "Run dry run"} icon="play" iconSize={16} disabled={running || (needsPattern && pattern.length === 0)} onclick={dryRun} />
        {#snippet below()}
          {#if editing?.dryRun && dryCurrent}
            {@const d = editing.dryRun}
            <div class="dry" aria-live="polite">
              <p class="sum tabular">
                Looked at {d.lookedAt} past {d.lookedAt === 1 ? "request" : "requests"}; it matches <strong>{d.matched}</strong>:
                {d.allowed} allowed, {d.denied} denied{d.unanswered ? `, ${d.unanswered} not known` : ""}{d.risky ? `, ${d.risky} risky` : ""}.
              </p>
              {#if d.shapes.length}
                <p class="shapes"><span>It could allow only:</span> {#each d.shapes as s (s)}<code>{s}</code>{/each}</p>
              {:else}
                <p class="warn"><Icon name="info" size={16} /><span>Nothing here was allowed by you yet, so this rule can only hint for now. It learns from your answers in the island.</span></p>
              {/if}
              {#if d.denied}
                <div class="warn"><Icon name="alert" size={16} /><div>
                    <span>You said no to {d.denied} of these before. Check the examples below before letting it answer for you.</span>
                    {#if editing?.autoAllow && canAuto}<label class="confirm"><input type="checkbox" bind:checked={deniedOk} /> I checked them; let it answer anyway</label>{/if}
                  </div></div>
              {/if}
              {#if d.risky}
                <p class="note"><Icon name="shield" size={16} /><span>{d.risky} of these {d.risky === 1 ? "was" : "were"} risky or couldn't be checked. Those always ask you, whatever the rule says.</span></p>
              {/if}
              {#if d.examples.length}
                <ul class="examples">
                  {#each d.examples as e (e.at + e.key)}
                    <li>
                      <span class="when tabular">{ago(e.at, now)}</span>
                      <code>{e.key}</code>
                      <span class="choice" data-choice={e.choice}>{e.choice === "allow" ? "Allowed" : e.choice === "deny" ? "Denied" : "Not known"}{e.risk === "risky" || e.risk === "unknown" ? " · risky" : ""}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
              <p class="src">{d.sources.join(" · ")}</p>
            </div>
          {:else if editing?.dryRun}
            <p class="note"><Icon name="info" size={16} /><span>The rule changed since its dry run. Run it again.</span></p>
          {/if}
        {/snippet}
      </Row>

      <Row
        label="Allow without asking"
        icon="check"
        help={canAuto
          ? "Only requests exactly like the ones you allowed in the dry run, never risky ones. Every answer it gives is listed below."
          : "Run the dry run first. It must find requests like this that you allowed."}
      >
        <Switch checked={editing.autoAllow && canAuto} label="Allow without asking" disabled={!canAuto} onchange={setAuto} />
        {#snippet below()}
          {#if editing?.autoAllow && canAuto}
            <div class="auto">
              <span class="lbl">For</span>
              <Seg label="How long" items={[{ id: "1", label: "1 day" }, { id: "7", label: "7 days" }, { id: "30", label: "30 days" }]} value={days} onchange={(v) => (days = v)} />
              <span class="then">then it goes back to hinting</span>
            </div>
          {/if}
        {/snippet}
      </Row>

      <Row line={false}>
        {#snippet below()}
          {#if autoOn && editing}
            <p class="commit"><Icon name="check" size={16} /><span>Until {dateText(until)}, Atoll answers yes for you to {editing.dryRun?.shapes.join(", ")}{editing.project ? ` in ${projectName(editing.project)}` : ""}. Anything else, and anything risky, still asks.</span></p>
          {/if}
          <div class="actions">
            {#if editing?.id}
              {@const saved = view?.rules.find((r) => r.id === editing?.id)}
              {#if saved}<GButton label="Delete rule" icon="trash" iconSize={16} destructive onclick={() => remove(saved)} />{/if}
              <span class="flex"></span>
            {/if}
            <GButton label="Cancel" onclick={close} />
            <GButton label={saving ? "Saving…" : autoOn ? `Allow until ${dateText(until)}` : "Save rule"} variant="filled" disabled={saving || !editing?.name.trim() || (needsPattern && pattern.length === 0) || (needsOk && !deniedOk)} onclick={save} />
          </div>
        {/snippet}
      </Row>
    </div>
  </section>
{/if}

<Section title="Answered by rules" desc="Every request a rule allowed without asking you, newest first.">
  {#if log.length === 0}
    <Row icon="clock" label="Nothing yet" help="When a rule allows a request for you, it is listed here with a button to turn the rule off." />
  {/if}
  {#each log as a (a.at + (a.ruleId ?? ""))}
    {@const rule = ruleById(a.ruleId)}
    <Row icon="check" label={a.summary || a.keys.join(" · ")} help={`${a.keys.join(" · ")} · ${a.ruleName ?? "a rule"} · ${projectName(a.project)} · ${ago(a.at, now)}`}>
      {#if rule?.enabled}
        <GButton label="Turn off rule" ariaLabel={`Turn off rule: ${rule.name}`} onclick={() => turnOff(rule.id)} />
      {:else}
        <span class="d-badge">{rule ? "Rule is off" : "Rule deleted"}</span>
      {/if}
    </Row>
  {/each}
</Section>

{#if view}
  <p class="foot">
    Dry runs use {view.answers} {view.answers === 1 ? "answer" : "answers"} you gave in the island (only the kind of request, never the full command){view.recorderConnected ? " and the recorder's history" : "; the recorder's longer history joins once it is connected"}.
  </p>
{/if}

<style>
  .banner {
    display: flex;
    gap: var(--sp-8);
    align-items: flex-start;
    margin: 0 0 var(--sp-24);
    padding: var(--sp-12) var(--sp-16);
    border-radius: var(--r-card);
    box-shadow: inset 0 0 0 1px var(--c-err);
    color: var(--c-ink);
    font-size: var(--fs-small);
  }
  .banner :global(.icon) { color: var(--c-err); flex: none; }
  .add, .actions { display: flex; gap: var(--sp-8); }
  .flex { flex: 1; }
  .status {
    display: flex;
    align-items: center;
    gap: var(--sp-6);
    margin: 0;
    padding-left: calc(var(--size-row-tile) + var(--sp-12));
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .status[data-tone="ok"] :global(.icon), .status[data-tone="ok"] > span:first-of-type { color: var(--c-ok); }
  .status[data-tone="warn"] :global(.icon), .status[data-tone="warn"] > span:first-of-type { color: var(--c-warn); }
  .status[data-tone="ink"] > span:first-of-type { color: var(--c-ink); }
  .uses { color: var(--c-muted); }
  .actions { justify-content: flex-end; }
  .name { width: min(420px, 100%); }
  .pattern {
    width: 100%;
    height: auto;
    min-height: calc(4 * 1.5em + 16px);
    padding: var(--sp-8) var(--sp-12);
    font-family: var(--font-code);
    font-size: var(--fs-small);
    line-height: 1.5;
    resize: vertical;
  }
  .dry { display: flex; flex-direction: column; gap: var(--sp-8); font-size: var(--fs-small); }
  .dry p { margin: 0; }
  .sum strong { font-weight: var(--fw-strong); }
  .shapes { display: flex; flex-wrap: wrap; gap: var(--sp-6); align-items: center; }
  .shapes > span { color: var(--c-muted); }
  code {
    font-family: var(--font-code);
    font-size: var(--fs-code);
    padding: 1px var(--sp-6);
    border-radius: var(--r-nav);
    background: var(--c-sel);
  }
  .warn, .note { display: flex; gap: var(--sp-6); align-items: flex-start; }
  .warn :global(.icon) { color: var(--c-warn); flex: none; margin-top: 1px; }
  .note { color: var(--c-muted); }
  .note :global(.icon) { flex: none; margin-top: 1px; }
  .examples { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-4); }
  .examples li { display: grid; grid-template-columns: 88px minmax(0, 1fr) auto; gap: var(--sp-8); align-items: center; }
  .examples code { justify-self: start; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when, .src { color: var(--c-muted); }
  .src { font-size: var(--fs-label); }
  .choice { font-size: var(--fs-label); font-weight: var(--fw-strong); color: var(--c-muted); }
  .choice[data-choice="allow"] { color: var(--c-ok); }
  .choice[data-choice="deny"] { color: var(--c-err); }
  .auto { display: flex; gap: var(--sp-12); align-items: center; flex-wrap: wrap; font-size: var(--fs-small); }
  .lbl { font-weight: var(--fw-strong); }
  .then { color: var(--c-muted); }
  .confirm { display: flex; gap: var(--sp-8); align-items: center; margin-top: var(--sp-6); color: var(--c-ink); cursor: pointer; min-height: 24px; }
  .confirm input { width: 18px; height: 18px; margin: 0; accent-color: var(--c-accent); }
  .commit { display: flex; gap: var(--sp-8); align-items: flex-start; margin: 0 0 var(--sp-12); font-size: var(--fs-small); color: var(--c-ink); }
  .commit :global(.icon) { flex: none; margin-top: 1px; color: var(--c-ok); }
  .foot { margin: var(--sp-8) 0 0; font-size: var(--fs-label); color: var(--c-muted); }
</style>
