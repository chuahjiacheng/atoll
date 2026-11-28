import type { Session, Status } from "./api";
import type { Mood } from "./ui/Pip.svelte";
import type { IconName } from "./ui/Icon.svelte";
import type { Signal } from "./island/Shell.svelte";

/** Most urgent status wins for the collapsed pill. */
const rank: Record<Status, number> = { attention: 4, error: 3, working: 2, done: 1, idle: 0 };

export function overallStatus(sessions: Session[]): Status {
  return sessions.reduce<Status>((best, s) => (rank[s.status] > rank[best] ? s.status : best), "idle");
}

/** The session the pill should talk about: most urgent, then most recent. */
export function headline(sessions: Session[]): Session | null {
  return [...sessions].sort((a, b) => rank[b.status] - rank[a.status] || b.updatedAt - a.updatedAt)[0] ?? null;
}

/** How each agent status looks: word, icon, Pip's mood, glow. */
export const STATUS_LOOK: Record<Status, { word: string; icon: IconName; mood: Mood; signal: Signal }> = {
  idle: { word: "Idle", icon: "dot", mood: "idle", signal: null },
  working: { word: "Busy", icon: "busy", mood: "focused", signal: "busy" },
  attention: { word: "Needs you", icon: "info", mood: "curious", signal: "accent" },
  done: { word: "Done", icon: "check", mood: "happy", signal: "done" },
  error: { word: "Error", icon: "alert", mood: "dizzy", signal: "error" },
};

/** Statuses that carry a message you can clear (back to idle). */
export const clearable = (s: Status | undefined | null) => s === "error" || s === "attention" || s === "done";

/** "4 min ago", "Yesterday"... */
export function ago(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return h < 48 ? "Yesterday" : `${Math.round(h / 24)} days ago`;
}

/** Elapsed time as m:ss (or h:mm:ss). */
export function elapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}` : `${m}:${String(s).padStart(2, "0")}`;
}

/** Clock time, 24-hour: "14:32". */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** The step of Claude's own task list it is on now: "2/4 · Fix the parser".
 *  Null when it made no list, or every step is finished. */
export function currentStep(s: Session): string | null {
  const plan = s.plan ?? [];
  let i = plan.findIndex((p) => p.status === "in_progress");
  // Between steps (one ticked off, the next not started yet): the next one.
  if (i < 0) i = plan.findIndex((p) => p.status === "pending");
  if (i < 0) return null;
  return `${i + 1}/${plan.length} · ${plan[i].content}`;
}

/** One-line summary for a session: "Editing peek.tsx", "3 files changed"... */
/** `live: false` (Settings → AI agents → Show live activity off) shows the status only. */
export function summary(s: Session, live = true): string {
  if ((s.status === "attention" || s.status === "error") && s.message) return s.message;
  if (s.status === "done") return s.message || "Task finished";
  const step = s.status === "working" ? currentStep(s) : null;
  if (step) return step;
  if (live && s.activity) return `${s.activity.label} ${s.activity.detail}`.trim();
  if (s.status === "working") return live ? "Thinking…" : "Working";
  return "Waiting for a prompt";
}

// ---------- truncation rules (design: Scene → Truncation rules) ----------

/** File paths are cut in the middle with "…/" and always keep the file name. */
export function truncPath(path: string, max = 48): string {
  if (path.length <= max) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/);
  const file = parts.pop() ?? path;
  let head = "";
  for (const p of parts) {
    if ((head + p + sep).length + file.length + 2 > max) break;
    head += p + sep;
  }
  return `${head}…${sep}${file}`;
}

/** Folder paths keep the drive and the last two folders. */
export function truncFolder(path: string): string {
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 4) return path;
  return [parts[0], "…", ...parts.slice(-2)].join(sep);
}
