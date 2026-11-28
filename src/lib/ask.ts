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
  /** "gemini" | "groq" | "ollama" */
  provider?: string;
  /** Context sent with a question. */
  attached?: { kind: AskContext; label: string; sources?: { title: string; url: string }[] }[];
}

export type AskContext = "web" | "clipboard" | "sessions" | "screen";

export const CONTEXT_LABEL: Record<AskContext, string> = { web: "Web", clipboard: "Clipboard", sessions: "Claude sessions", screen: "Screen" };

export type SearchProvider = "tavily" | "ollama";

export interface AskView {
  messages: AskMsg[];
  /** Id of the reply still being written. */
  busy: number | null;
  /** The main provider can be used (has its key, or is local). */
  hasKey: boolean;
  /** Main provider, and the model it is set to ("" = automatic). */
  provider: AskProvider;
  model: string;
  /** Context switched on in Settings → Ask ("web" when a search key is saved). */
  context: AskContext[];
  /** The answer in progress is waiting on a web search. */
  searching: boolean;
}

export type AskProvider = "gemini" | "groq" | "ollama";

export interface AskUsage {
  /** Local date, YYYY-MM-DD */
  day: string;
  gemini: number;
  groq: number;
  ollama: number;
  /** From Groq's own rate-limit headers on its last answer. */
  groqLimit: number | null;
  groqRemaining: number | null;
}

export const PROVIDER_LABEL: Record<string, string> = { gemini: "Gemini", groq: "Groq", ollama: "Local" };

export interface AskKeyStatus {
  hasKey: boolean;
  masked: string;
}

/** Longest question accepted (MAX_INPUT in ask.rs). */
export const ASK_MAX_INPUT = 12_000;

export const askApi = {
  get: () => invoke<AskView>("ask_get"),
  send: (text: string, attach: AskContext[] = []) => invoke<void>("ask_send", { text, attach }),
  /** Title of the window Screen would picture ("" = none known). */
  screenTitle: () => invoke<string>("ask_screen_title"),
  /** Load the local model now, so the first answer doesn't wait for it. */
  warm: () => invoke<void>("ask_warm"),
  stop: () => invoke<void>("ask_stop"),
  clear: () => invoke<void>("ask_clear"),
  keyStatus: () => invoke<AskKeyStatus>("ask_key_status"),
  /** Tests the key with Google, saves it if it works, returns its chat models. */
  setKey: (key: string) => invoke<string[]>("ask_set_key", { key }),
  forgetKey: () => invoke<void>("ask_forget_key"),
  models: () => invoke<string[]>("ask_models"),
  groqKeyStatus: () => invoke<AskKeyStatus>("ask_groq_key_status"),
  setGroqKey: (key: string) => invoke<string[]>("ask_set_groq_key", { key }),
  forgetGroqKey: () => invoke<void>("ask_forget_groq_key"),
  groqModels: () => invoke<string[]>("ask_groq_models"),
  ollamaModels: () => invoke<string[]>("ask_ollama_models"),
  usage: () => invoke<AskUsage>("ask_usage"),
  searchKeyStatus: (provider: SearchProvider) => invoke<AskKeyStatus>("ask_search_key_status", { provider }),
  setSearchKey: (provider: SearchProvider, key: string) => invoke<void>("ask_set_search_key", { provider, key }),
  forgetSearchKey: (provider: SearchProvider) => invoke<void>("ask_forget_search_key", { provider }),
  testSearchKey: (provider: SearchProvider) => invoke<void>("ask_test_search_key", { provider }),
  /** Opens an https link from a reply in the browser. */
  openLink: (url: string) => invoke<void>("calendar_open", { url }),
};

export const onAsk = (cb: (v: AskView) => void): Promise<UnlistenFn> => listen<AskView>("ask", (e) => cb(e.payload));

export const onAskDelta = (cb: (d: { id: number; text: string }) => void): Promise<UnlistenFn> =>
  listen<{ id: number; text: string }>("ask:delta", (e) => cb(e.payload));

/** Where to get a free key. */
export const AI_STUDIO_KEYS = "https://aistudio.google.com/app/apikey";
export const GROQ_KEYS = "https://console.groq.com/keys";
export const OLLAMA_DOWNLOAD = "https://ollama.com/download";
export const TAVILY_KEYS = "https://app.tavily.com/home";
export const OLLAMA_KEYS = "https://ollama.com/settings/keys";
