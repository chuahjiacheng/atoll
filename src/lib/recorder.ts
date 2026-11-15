// Flight recorder: typed wrappers for the recorder_* commands
// (docs/contracts/event-store.md section 5). Owner: recorder-forecast.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface SessionRow {
  sessionId: string;
  project: string;
  entrypoint: "cli" | "desktop" | "";
  startedAt: number;
  lastEventAt: number;
  endedAt: number | null;
  eventCount: number;
}

export interface EventRow {
  id: number;
  ts: number;
  sessionId: string;
  kind: string;
  tool: string | null;
  agentId: string | null;
  summary: string;
}

export interface DiffHunk {
  path: string;
  old: string;
  new: string;
  truncated: boolean;
}

export interface EventDetail extends EventRow {
  toolUseId: string | null;
  agentType: string | null;
  detail: string;
  payload: unknown;
  diff: DiffHunk[] | null;
}

export interface Breakdown {
  total: number;
  thinking: number;
  tools: [string, number][];
  waitingForYou: number;
  idle: number;
}

export interface AgentNode {
  agentId: string;
  agentType: string | null;
  parent: string | null;
  startedAt: number;
  endedAt: number | null;
  eventCount: number;
}

export interface StoreStats {
  events: number;
  sessions: number;
  usageSamples: number;
  fileBytes: number;
  oldestTs: number | null;
  dropped: number;
}

export type LimitWindow = "five_hour" | "seven_day";

export interface UsageSample {
  ts: number;
  window: LimitWindow;
  used: number;
  resetsAt: number;
  source: string;
}

export const recorder = {
  sessions: (sinceMs?: number, limit?: number) => invoke<SessionRow[]>("recorder_sessions", { query: { sinceMs, limit } }),
  events: (sessionId: string, limit = 5000) => invoke<EventRow[]>("recorder_events", { query: { sessionId, limit } }),
  event: (id: number) => invoke<EventDetail | null>("recorder_event", { id }),
  search: (text: string, limit = 100) => invoke<EventRow[]>("recorder_search", { text, limit }),
  breakdown: (sessionId: string) => invoke<Breakdown>("recorder_breakdown", { sessionId }),
  agentTree: (sessionId: string) => invoke<AgentNode[]>("recorder_agent_tree", { sessionId }),
  usage: (window: LimitWindow, sinceMs: number) => invoke<UsageSample[]>("recorder_usage", { window, sinceMs }),
  stats: () => invoke<StoreStats>("recorder_stats"),
  deleteAll: () => invoke<null>("recorder_delete_all"),
  onChanged: (cb: (sessionId: string) => void): Promise<UnlistenFn> =>
    listen<{ sessionId: string }>("recorder:changed", (e) => cb(e.payload.sessionId)),
};

/** "1 h 12 m", "4 m", "38 s". Rounds; never shows more than two units. */
export function span(ms: number): string {
  const s = Math.round(Math.max(0, ms) / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} m`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} m` : `${h} h`;
}

/** "1.2 MB" */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** "Today 14:03", "Yesterday 09:10", "3 Nov 16:20" */
export function dayTime(ms: number, now: number): string {
  const d = new Date(ms);
  const t = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const dayStart = new Date(ms);
  dayStart.setHours(0, 0, 0, 0);
  const days = Math.round((today.getTime() - dayStart.getTime()) / 86_400_000);
  if (days === 0) return `Today ${t}`;
  if (days === 1) return `Yesterday ${t}`;
  return `${d.getDate()} ${d.toLocaleString("en-GB", { month: "short" })} ${t}`;
}

/** "14:03:22" */
export function clockSeconds(ms: number): string {
  const d = new Date(ms);
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":");
}

export type DiffLine = { op: " " | "+" | "-"; text: string };

/** Line diff (longest common subsequence). Large inputs fall back to
 *  "all removed, then all added" so the island never stalls. */
export function lineDiff(oldText: string, newText: string): DiffLine[] {
  const a = oldText ? oldText.split("\n") : [];
  const b = newText ? newText.split("\n") : [];
  if (a.length * b.length > 250_000) {
    return [...a.map((text) => ({ op: "-" as const, text })), ...b.map((text) => ({ op: "+" as const, text }))];
  }
  const n = a.length;
  const m = b.length;
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ op: " ", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ op: "-", text: a[i++] });
    } else {
      out.push({ op: "+", text: b[j++] });
    }
  }
  while (i < n) out.push({ op: "-", text: a[i++] });
  while (j < m) out.push({ op: "+", text: b[j++] });
  return out;
}

/** What a timeline mark means, for its colour and its spoken label. */
export function markKind(kind: string): "prompt" | "tool" | "ask" | "stop" | "fail" | "other" {
  switch (kind) {
    case "UserPromptSubmit":
      return "prompt";
    case "PreToolUse":
    case "PostToolUse":
      return "tool";
    case "PermissionRequest":
    case "Notification":
      return "ask";
    case "Stop":
    case "SessionEnd":
      return "stop";
    case "StopFailure":
    case "PostToolUseFailure":
      return "fail";
    default:
      return "other";
  }
}

/** Plain-English name of a hook event. */
export function kindName(kind: string): string {
  const names: Record<string, string> = {
    SessionStart: "Session started",
    SessionEnd: "Session ended",
    UserPromptSubmit: "Your message",
    PreToolUse: "Tool call",
    PostToolUse: "Tool finished",
    PostToolUseFailure: "Tool failed",
    PermissionRequest: "Asked permission",
    Notification: "Needed you",
    Stop: "Turn finished",
    StopFailure: "Turn failed",
    SubagentStart: "Subagent started",
    SubagentStop: "Subagent finished",
  };
  return names[kind] ?? kind;
}
