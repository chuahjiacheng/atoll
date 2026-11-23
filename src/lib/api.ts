// Typed wrappers around the Rust commands and events (see src-tauri/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Status = "idle" | "working" | "attention" | "done" | "error";

export interface Activity {
  kind: "read" | "edit" | "run" | "web" | "agent" | "tool";
  label: string;
  detail: string;
  active: boolean;
}

export interface PlanStep {
  content: string;
  status: "pending" | "in_progress" | "completed";
}

/** A question Claude ended its turn with (Claude app's "next step" box). */
export interface Ask {
  question: string;
  /** Reply buttons, recommended first; picking one sends it to Claude. */
  choices: string[];
  at: number;
}

/** One of Claude's multiple-choice questions (AskUserQuestion). */
export interface Question {
  question: string;
  header?: string;
  multiSelect?: boolean;
  options: { label: string; description?: string }[];
}

export interface Session {
  id: string;
  project: string;
  cwd: string;
  source: "terminal" | "desktop";
  status: Status;
  activity: Activity | null;
  /** Claude's own task list for this turn (empty when it made none). */
  plan: PlanStep[];
  message: string | null;
  prompt: string | null;
  /** Claude's full final message of the last turn. */
  answer: string | null;
  toolCount: number;
  /** Claude's end-of-turn question with reply choices, until answered or cleared. */
  ask?: Ask | null;
  startedAt: number;
  updatedAt: number;
}

export interface Permission {
  id: number;
  sessionId: string;
  project: string;
  tool: string;
  label: string;
  detail: string;
  preview: string;
  createdAt: number;
  expiresAt: number;
  /** AskUserQuestion only: the questions to answer in the island. */
  questions?: Question[];
  // ---- permission-intel: begin ----
  /** What the request does and how risky it is (absent if the app predates it). */
  intel?: PermissionIntel;
  /** A rule matched but still asks (Stage C). */
  rule?: import("./rules").RuleHint;
  // ---- permission-intel: end ----
}

// ---- permission-intel: begin ----
/** unknown = Atoll could not check it; treated like risky. */
export type Risk = "safe" | "caution" | "unknown" | "risky";
export interface Finding {
  flag: string;
  risk: Risk;
  note: string;
}
export interface DiffLine {
  kind: "context" | "removed" | "added" | "gap";
  text: string;
}
export interface Diff {
  path: string;
  lang: string;
  lines: DiffLine[];
  added: number;
  removed: number;
  more: number;
  cut: boolean;
  newFile: boolean;
}
export interface PermissionIntel {
  risk: Risk;
  shell: "bash" | "powerShell" | "tool";
  parsed: boolean;
  summary: string;
  /** Up to two further reasons the sentence does not already give. */
  reasons?: Finding[];
  /** Paths outside the project the request touches, as written. */
  outside?: string[];
  findings: Finding[];
  autoAllowEligible: boolean;
  ineligibleReason: string;
  diff?: Diff;
  elapsedUs: number;
}
// ---- permission-intel: end ----

/** allow / deny, or pass = hand it back to Claude Code's own prompt. */
export type Choice = "allow" | "deny" | "pass" | "answer";

export interface HookStatus {
  installedEvents: string[];
  allInstalled: boolean;
  bridgePresent: boolean;
  settingsPath: string;
  bridgePath: string;
  /** "crest" = ours, "other" = yours (left alone), "none" */
  statusLine: "crest" | "other" | "none";
}

export interface LimitWindow {
  /** 0..100 */
  used: number;
  /** unix seconds */
  resetsAt: number;
}

export interface LogEntry {
  sessionId: string;
  project: string;
  kind: Activity["kind"];
  verb: string;
  target: string;
  at: number;
}

export interface HistoryEntry {
  sessionId: string;
  project: string;
  title: string;
  outcome: "done" | "error";
  at: number;
}

export interface AgentFeed {
  log: LogEntry[];
  history: HistoryEntry[];
}

export interface Usage {
  fiveHour: LimitWindow | null;
  sevenDay: LimitWindow | null;
  updatedAt: number;
  /** Why the sign-in usage check isn't working, if it isn't. */
  signInError?: string | null;
}

/** A Claude chat (claude.ai / the desktop Chat tab), read from Claude's local cache. */
export interface Chat {
  uuid: string;
  name: string;
  updatedAt: number;
  /** Your last message (only when the chat's messages are cached). */
  ask: string | null;
  /** Claude's last reply (only when cached). */
  answer: string | null;
  answeredAt: number;
  messageCount: number;
  /** The chat changed after its messages were cached (reply being written, or not opened yet). */
  behind: boolean;
}

export interface ChatsView {
  /** A claude.ai cache was found on this PC. */
  available: boolean;
  /** "free" | "pro" | "max" | "team" | "enterprise" | "" (unknown). */
  plan: string;
  /** Newest first. */
  chats: Chat[];
}

export type AlertKind = "done" | "attention" | "error" | "usage" | "chatReply";

export interface AlertEvent {
  kind: AlertKind;
  sessionId: string | null;
  title: string;
  body: string;
}

export const api = {
  getSessions: () => invoke<Session[]>("get_sessions"),
  dismissSession: (id: string) => invoke<void>("dismiss_session", { id }),
  /** Clear an error / "needs you" / "done" message; the session stays, back to idle. */
  clearSession: (id: string) => invoke<void>("clear_session", { id }),
  getPermissions: () => invoke<Permission[]>("get_permissions"),
  decidePermission: (id: number, choice: Choice, reply?: string, answers?: Record<string, string>) =>
    invoke<void>("decide_permission", { id, choice, reply, answers }),
  /** Send a reply to Claude through the Claude app (types it and presses Enter). */
  replyInClaude: (sessionId: string, text: string) => invoke<void>("reply_in_claude", { sessionId, text }),
  getAgentFeed: () => invoke<AgentFeed>("get_agent_feed"),
  getChats: () => invoke<ChatsView>("get_chats"),
  /** Open a chat in the Claude desktop app, or with `web` on claude.ai. */
  openChat: (uuid: string, web = false) => invoke<void>("open_chat", { uuid, web }),
  quit: () => invoke<void>("quit_app"),
  setHitRect: (r: { x: number; y: number; w: number; h: number }) => invoke<void>("set_hit_rect", r),
  getUsage: () => invoke<Usage>("get_usage"),
  hooksStatus: () => invoke<HookStatus>("hooks_status"),
  installHooks: () => invoke<HookStatus>("install_hooks"),
  uninstallHooks: () => invoke<HookStatus>("uninstall_hooks"),
};

export const onSessions = (cb: (s: Session[]) => void): Promise<UnlistenFn> =>
  listen<Session[]>("sessions", (e) => cb(e.payload));

export const onUsage = (cb: (u: Usage) => void): Promise<UnlistenFn> => listen<Usage>("usage", (e) => cb(e.payload));

export const onAlert = (cb: (a: AlertEvent) => void): Promise<UnlistenFn> =>
  listen<AlertEvent>("alert", (e) => cb(e.payload));

export const onPermissions =(cb: (p: Permission[]) => void): Promise<UnlistenFn> =>
  listen<Permission[]>("permissions", (e) => cb(e.payload));

/** Emitted by the Rust click-through poller when the cursor enters/leaves the island. */
export const onHover = (cb: (inside: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("island-hover", (e) => cb(e.payload));

export const onAgentFeed = (cb: (f: AgentFeed) => void): Promise<UnlistenFn> =>
  listen<AgentFeed>("agent-feed", (e) => cb(e.payload));

export const onChats = (cb: (c: ChatsView) => void): Promise<UnlistenFn> =>
  listen<ChatsView>("chats", (e) => cb(e.payload));

