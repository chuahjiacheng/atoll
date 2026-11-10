// Focus timer (src-tauri/src/focus.rs): types, commands and the `focus` event.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FocusSettings } from "./settings";

export type FocusPhase = "ready" | "focus" | "break";
export type HeldKind = "done" | "attention" | "error" | "usage" | "chatReply" | "timer";

export interface Held {
  kind: HeldKind;
  sessionId: string | null;
  title: string;
  body: string;
  at: number;
}

export interface DayBar {
  date: string;
  label: string;
  focusMs: number;
}

export interface FocusStats {
  todayMs: number;
  todaySessions: number;
  streak: number;
  week: DayBar[];
}

export interface FocusView {
  phase: FocusPhase;
  running: boolean;
  /** This phase's length; ready = the next session's. 0 = no time limit. */
  lengthMs: number;
  /** Measured at `at`; add the time since while running. */
  elapsedMs: number;
  at: number;
  round: number;
  rounds: number;
  longBreak: boolean;
  intention: string;
  held: Held[];
  digestOpen: boolean;
  distractions: number;
  nudge: { app: string; at: number } | null;
  stats: FocusStats;
}

export type FocusAction = "start" | "pause" | "resume" | "toggle" | "skip" | "reset" | "intention" | "dismiss" | "clearHistory";

export const focusApi = {
  get: () => invoke<FocusView>("focus_get"),
  act: (action: FocusAction, text?: string) => invoke<FocusView>("focus_action", { action, text: text ?? null }),
  openWindowsDnd: () => invoke<void>("open_windows_dnd"),
};

export const onFocus = (cb: (v: FocusView) => void): Promise<UnlistenFn> => listen<FocusView>("focus", (e) => cb(e.payload));

/** Time run in this phase at `now`. */
export function focusElapsed(v: FocusView, now: number): number {
  return v.elapsedMs + (v.running ? Math.max(0, now - v.at) : 0);
}

/** Time left (ms), or null when the session has no time limit. */
export function focusLeft(v: FocusView, now: number): number | null {
  return v.lengthMs > 0 ? Math.max(0, v.lengthMs - focusElapsed(v, now)) : null;
}

/** What the clock shows: time left, or time run for a session without a limit. */
export function focusClockMs(v: FocusView, now: number): number {
  return focusLeft(v, now) ?? focusElapsed(v, now);
}

/** 0..1 through this phase (0 for a session without a limit). */
export function focusFraction(v: FocusView, now: number): number {
  return v.lengthMs > 0 ? Math.min(1, focusElapsed(v, now) / v.lengthMs) : 0;
}

/** "24:13", or "1:05:09" past an hour. Rounds up, so a timer never shows 0:00 early. */
export function timerText(ms: number, up = false): string {
  const total = up ? Math.floor(ms / 1000) : Math.ceil(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** "18 minutes 42 seconds" for screen readers. */
export function spokenTime(ms: number): string {
  const total = Math.round(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  const part = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  return m > 0 ? `${part(m, "minute")} ${part(s, "second")}` : part(s, "second");
}

/** "1 h 25 m" / "40 m" */
export function focusTotal(ms: number): string {
  const mins = Math.round(ms / 60000);
  return mins >= 60 ? `${Math.floor(mins / 60)} h ${mins % 60} m` : `${mins} m`;
}

/** Is a session running or paused, or a break on (not just ready)? */
export const focusActive = (v: FocusView | null) => !!v && v.phase !== "ready";

/** The design's presets (focus / break minutes). */
export const FOCUS_PRESETS: { id: string; focus: number; brk: number; label: string }[] = [
  { id: "25", focus: 25, brk: 5, label: "25 / 5" },
  { id: "50", focus: 50, brk: 10, label: "50 / 10" },
  { id: "90", focus: 90, brk: 15, label: "90 / 15" },
];

/** Which preset the settings match: a preset id, "open" (no limit) or "custom". */
export function presetOf(f: FocusSettings): string {
  if (f.openEnded) return "open";
  return FOCUS_PRESETS.find((p) => p.focus === f.focusMin && p.brk === f.breakMin)?.id ?? "custom";
}
