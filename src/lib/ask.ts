// Ask, the AI assistant tab: types mirror src-tauri/src/ask.rs (ask-assistant).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface AskMsg {
  id: number;
  role: "user" | "assistant";
  text: string;
  /** ms since epoch */
  at: number;
  /** Why the reply failed or stopped early. */
  error?: string;
  model?: string;
}

export interface AskView {
  messages: AskMsg[];
  /** Id of the reply still being written. */
  busy: number | null;
  hasKey: boolean;
  model: string;
}

export interface AskKeyStatus {
  hasKey: boolean;
  masked: string;
}

/** Longest question accepted (MAX_INPUT in ask.rs). */
export const ASK_MAX_INPUT = 12_000;

export const askApi = {
  get: () => invoke<AskView>("ask_get"),
  send: (text: string) => invoke<void>("ask_send", { text }),
  stop: () => invoke<void>("ask_stop"),
  clear: () => invoke<void>("ask_clear"),
  keyStatus: () => invoke<AskKeyStatus>("ask_key_status"),
  /** Tests the key with Google, saves it if it works, returns its chat models. */
  setKey: (key: string) => invoke<string[]>("ask_set_key", { key }),
  forgetKey: () => invoke<void>("ask_forget_key"),
  models: () => invoke<string[]>("ask_models"),
  /** Opens an https link from a reply in the browser. */
  openLink: (url: string) => invoke<void>("calendar_open", { url }),
};

export const onAsk = (cb: (v: AskView) => void): Promise<UnlistenFn> => listen<AskView>("ask", (e) => cb(e.payload));

export const onAskDelta = (cb: (d: { id: number; text: string }) => void): Promise<UnlistenFn> =>
  listen<{ id: number; text: string }>("ask:delta", (e) => cb(e.payload));

/** Where to get a free key. */
export const AI_STUDIO_KEYS = "https://aistudio.google.com/app/apikey";
