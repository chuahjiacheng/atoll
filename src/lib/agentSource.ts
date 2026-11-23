/**
 * What the Agent tab shows: Claude Code sessions or Claude chats.
 *
 * - Free plan: Claude Code isn't included, so chats only.
 * - Pro and up: Code while it is in use; when it isn't and you're chatting,
 *   chats. The user can pin either one (`chatSource`), or go back to Auto.
 */
import type { ChatsView, Session } from "./api";

export type ChatSource = "auto" | "code" | "chat";

export interface SourceChoice {
  show: "code" | "chat";
  /** Both are available, so the tab offers the switch. */
  canSwitch: boolean;
}

/** A Code session counts as "in use" this long after its last event. */
export const CODE_ACTIVE_MS = 10 * 60 * 1000;

export function pickAgentSource(pref: ChatSource, chats: ChatsView | null, sessions: Session[], now: number): SourceChoice {
  const chatsOk = !!chats?.available && chats.chats.length > 0;
  // Claude Code needs Pro or above (or an API key, which shows up as a session).
  const codeOk = sessions.length > 0 || chats?.plan !== "free";
  if (!chatsOk) return { show: "code", canSwitch: false };
  if (!codeOk) return { show: "chat", canSwitch: false };
  if (pref === "code" || pref === "chat") return { show: pref, canSwitch: true };

  const codeActive = sessions.some((s) => s.status === "working" || s.status === "attention" || now - s.updatedAt < CODE_ACTIVE_MS);
  if (codeActive) return { show: "code", canSwitch: true };
  const lastChat = chats!.chats[0]?.updatedAt ?? 0;
  const lastCode = sessions.reduce((m, s) => Math.max(m, s.updatedAt), 0);
  return { show: lastChat > lastCode ? "chat" : "code", canSwitch: true };
}
