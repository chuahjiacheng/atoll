// Permission rules (mirrors src-tauri/src/rules): types and commands.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Risk } from "./api";

export interface Example {
  at: number;
  key: string;
  /** "allow", "deny" or "" (not known). */
  choice: string;
  risk: Risk;
}

export interface DryRun {
  at: number;
  fingerprint: string;
  lookedAt: number;
  matched: number;
  allowed: number;
  denied: number;
  unanswered: number;
  risky: number;
  /** The only command kinds the rule may auto-allow. */
  shapes: string[];
  examples: Example[];
  sources: string[];
}

export interface Rule {
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
  enabled: boolean;
  /** The session's starting folder (normalised), or null for every project. */
  project: string | null;
  /** "Bash", "files", "WebFetch" or an exact tool name. */
  tool: string;
  pattern: string[];
  autoAllow: boolean;
  expiresAt: number | null;
  dryRun: DryRun | null;
  hits: number;
  lastHit: number | null;
}

export interface Answer {
  at: number;
  project: string;
  tool: string;
  keys: string[];
  risk: Risk;
  choice: string;
  by: string;
  ruleId?: string;
  ruleName?: string;
  summary?: string;
}

export interface RulesView {
  rules: Rule[];
  error: string | null;
  answers: number;
  recorderConnected: boolean;
  maxAutoDays: number;
}

/** A rule hint on a waiting request: a rule matched but it still asks. */
export interface RuleHint {
  ruleId: string;
  ruleName: string;
  whyNot: string;
}

export interface AutoAllowed {
  ruleId: string;
  ruleName: string;
  summary: string;
  at: number;
}

export const DAY = 24 * 60 * 60 * 1000;

export const rulesApi = {
  list: () => invoke<RulesView>("rules_list"),
  save: (rule: Rule) => invoke<Rule>("rules_save", { rule }),
  remove: (id: string) => invoke<void>("rules_delete", { id }),
  disable: (id: string) => invoke<void>("rules_disable", { id }),
  dryRun: (rule: Rule) => invoke<DryRun>("rules_dry_run", { rule }),
  log: (limit = 50) => invoke<Answer[]>("rules_log", { limit }),
  fromRequest: (id: number) => invoke<void>("rules_from_request", { id }),
  takeDraft: () => invoke<Rule | null>("rules_take_draft"),
};

export const onRulesChanged = (f: () => void): Promise<UnlistenFn> => listen("rules:changed", () => f());
export const onAutoAllowed = (f: (a: AutoAllowed) => void): Promise<UnlistenFn> => listen<AutoAllowed>("permissions:auto-allowed", (e) => f(e.payload));

/** What a rule covers, in words: "Commands", "File edits", "Web pages", or the tool. */
export function toolLabel(tool: string): string {
  if (tool === "Bash") return "Commands";
  if (tool === "files") return "File edits";
  if (tool === "WebFetch") return "Web pages";
  if (tool.startsWith("mcp__")) {
    const [, server = "", name = ""] = tool.split("__");
    return `${name.replace(/_/g, " ")} (${server} connector)`;
  }
  return tool;
}

/** Whether the rule's dry run still describes the rule as it is (same tool, project, pattern). */
export function fingerprint(rule: Pick<Rule, "tool" | "project" | "pattern">): string {
  const pattern = [...new Set(rule.pattern.map((p) => p.trim().toLowerCase()).filter(Boolean))].sort();
  const project = (rule.project ?? "").trim().replace(/\\/g, "/").toLowerCase().replace(/\/+$/, "");
  const text = `${rule.tool}|${project}|${pattern.join("\n")}`;
  // FNV-1a 64 bit, same as the Rust side.
  let h = 0xcbf29ce484222325n;
  for (const b of new TextEncoder().encode(text)) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16).padStart(16, "0");
}

export const blankRule = (): Rule => ({
  id: "",
  name: "",
  createdAt: 0,
  updatedAt: 0,
  enabled: true,
  project: null,
  tool: "Bash",
  pattern: [],
  autoAllow: false,
  expiresAt: null,
  dryRun: null,
  hits: 0,
  lastHit: null,
});
