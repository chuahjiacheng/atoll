// Rules lab (dev only, rules-lab.html): the real Settings → Permission rules page
// with made-up rules, a dry run and a decision log. Tauri is stubbed; saves only log.
// ?empty shows the first-run state; ?draft opens the editor as "Make a rule…" would.
import { mount } from "svelte";
import "../../app.css";
import "../dashboard.css";
import type { Answer, DryRun, Rule } from "../../lib/rules";

const DAY = 24 * 60 * 60 * 1000;
const now = Date.now();
const params = new URLSearchParams(location.search);
const project = "c:/users/me/proj";

const dry: DryRun = {
  at: now,
  fingerprint: "",
  lookedAt: 214,
  matched: 23,
  allowed: 21,
  denied: 1,
  unanswered: 0,
  risky: 1,
  shapes: ["npm test", "npm test --watch", "tail"],
  examples: [
    { at: now - 40 * 60_000, key: "npm test · tail", choice: "allow", risk: "caution" },
    { at: now - 3 * 3600_000, key: "npm test --watch", choice: "deny", risk: "caution" },
    { at: now - 26 * 3600_000, key: "npm test", choice: "allow", risk: "caution" },
  ],
  sources: ["214 answered requests in Atoll", "the recorder is not connected yet"],
};

const rules: Rule[] = [
  { id: "a", name: "npm test, tail in proj", createdAt: now - 9 * DAY, updatedAt: now - 2 * DAY, enabled: true, project, tool: "Bash", pattern: ["npm test", "tail"], autoAllow: true, expiresAt: now + 5 * DAY, dryRun: dry, hits: 37, lastHit: now - 12 * 60_000 },
  { id: "b", name: "Edits in src/**", createdAt: now - 4 * DAY, updatedAt: now - 4 * DAY, enabled: true, project, tool: "files", pattern: ["src/**"], autoAllow: false, expiresAt: null, dryRun: null, hits: 0, lastHit: null },
  { id: "c", name: "Pages from docs.rs", createdAt: now - 20 * DAY, updatedAt: now - 20 * DAY, enabled: false, project: null, tool: "WebFetch", pattern: ["docs.rs"], autoAllow: false, expiresAt: null, dryRun: null, hits: 4, lastHit: now - 6 * DAY },
];

const log: Answer[] = [
  { at: now - 12 * 60_000, project, tool: "Bash", keys: ["npm test", "tail"], risk: "caution", choice: "allow", by: "rule", ruleId: "a", ruleName: rules[0].name, summary: "Runs the project's “test” script." },
  { at: now - 3 * 3600_000, project, tool: "Bash", keys: ["npm test"], risk: "caution", choice: "allow", by: "rule", ruleId: "a", ruleName: rules[0].name, summary: "Runs the project's “test” script." },
  { at: now - 6 * DAY, project: "c:/users/me/other", tool: "WebFetch", keys: ["fetch docs.rs"], risk: "caution", choice: "allow", by: "rule", ruleId: "c", ruleName: rules[2].name, summary: "Fetches a page from docs.rs." },
];

const draft: Rule = { ...rules[0], id: "", name: "npm test in proj", pattern: ["npm test"], autoAllow: false, expiresAt: null, dryRun: null, hits: 0, lastHit: null };
let draftTaken = !params.has("draft");

(window as unknown as { __calls: unknown[] }).__calls = [];
(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args: Record<string, unknown>) => {
    (window as unknown as { __calls: unknown[] }).__calls.push({ cmd, args });
    const empty = params.has("empty");
    switch (cmd) {
      case "rules_list":
        return { rules: empty || params.has("broken") ? [] : rules, error: params.has("broken") ? "Your rules file was damaged and could not be read, so no rule is acting. It was set aside as C:\Users\me\AppData\Local\Crest\permissions\rules.json.broken-1795; rules you add now start a new file." : null, answers: 214, recorderConnected: false, maxAutoDays: 30 };
      case "rules_log":
        return empty || params.has("broken") ? [] : log;
      case "rules_dry_run": {
        const { fingerprint } = await import("../../lib/rules");
        return { ...dry, fingerprint: fingerprint(args.rule as Rule) };
      }
      case "rules_take_draft":
        if (draftTaken) return null;
        draftTaken = true;
        return draft;
      case "rules_save":
        return args.rule;
      case "plugin:event|listen":
        return 0;
      default:
        return null;
    }
  },
  transformCallback: () => 0,
  metadata: { currentWindow: { label: "dashboard" }, currentWebview: { label: "dashboard" } },
};

document.documentElement.dataset.theme = params.get("theme") === "light" ? "light" : "dark";
const { default: Lab } = await import("./RulesLab.svelte");
mount(Lab, { target: document.getElementById("app")! });
