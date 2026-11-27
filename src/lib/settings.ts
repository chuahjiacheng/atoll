// Settings shape (mirrors src-tauri/src/settings.rs) and the commands that read/write it.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A named preset, or a custom hex (applied only if it has 4.5:1 contrast). */
export interface ColorChoice {
  preset: string;
  custom: string;
  customOn: boolean;
}

export interface Appearance {
  /** "compact" | "standard" | "large" | "custom" */
  preset: string;
  /** Overall island size, 25..100 % */
  scale: number;
  /** Centre of the island, 0..100 % of the screen width (50 = centre) */
  positionX: number;
  /** Gap from the top of the screen, 0..400 px */
  positionY: number;
  collapsedWidth: number;
  collapsedHeight: number;
  expandedWidth: number;
  expandedHeight: number;
  /** 0..100 % of a full pill */
  roundness: number;
  /** 50..100 % */
  opacity: number;
  /** 0..48 px */
  blur: number;
  /** Liquid: refracting glass drawn by Atoll. Frosted: Windows' fixed blur. */
  glass: "liquid" | "frosted";
  /** Liquid only, 0..100 %: how far the rim bends the desktop. */
  distortion: number;
  /** Liquid only, 0..100 %: rim light, colour split, saturation. */
  liquid: number;
  theme: "follow" | "dark" | "light";
  accent: ColorChoice;
  busy: ColorChoice;
  done: ColorChoice;
  error: ColorChoice;
  /** 50..200 % */
  animationSpeed: number;
  reduceMotion: boolean;
  monitor: string;
}

export interface Modules {
  claude: boolean;
  alerts: boolean;
  usageRing: boolean;
  chatWatcher: boolean;
  spotify: boolean;
  clipboard: boolean;
  whiteboard: boolean;
  fileTray: boolean;
  note: boolean;
  focus: boolean;
  calendar: boolean;
  mascot: boolean;
  screenQuestion: boolean;
  ask: boolean; // ask-assistant
}

export type ModuleKey = keyof Modules;

export interface Hotkeys {
  toggleIsland: string;
  expandIsland: string;
  openSettings: string;
  modules: Record<string, string>;
}

export type SoundName = "chime" | "pop" | "bell" | "wood" | "none";

export interface EventAlert {
  sound: SoundName;
  /** 0..100 */
  volume: number;
  /** Windows notification */
  notify: boolean;
}

export interface Alerts {
  /** Pulse the island in the event's colour. */
  flash: boolean;
  done: EventAlert;
  error: EventAlert;
  permission: EventAlert;
  usage: EventAlert;
  chat: EventAlert;
  timer: EventAlert;
  usageThreshold: number;
}

/** Focus timer choices (the end-of-session sound is `alerts.timer`). */
export interface FocusSettings {
  /** 5..90 minutes */
  focusMin: number;
  /** 1..30 minutes */
  breakMin: number;
  /** 5..60 minutes, after every `longEvery` (2..8) sessions */
  longMin: number;
  longEvery: number;
  /** Sessions run until you end them (the timer counts up). */
  openEnded: boolean;
  /** Do not disturb: alerts follow the three rules below during focus. */
  dnd: boolean;
  /** Finished tasks, chat replies, usage warnings. */
  finished: "hold" | "show" | "silent";
  errors: "hold" | "show" | "silent";
  permissions: "quiet" | "show";
  nudge: boolean;
  nudgeApps: string[];
  /** Play Spotify when a session starts, pause it at the break. */
  music: boolean;
}

/** Google Calendar (client secret and sign-in live in Windows Credential Manager). */
export interface CalendarSettings {
  clientId: string;
  /** On/off per calendar id; missing ids follow Google's own choice. */
  shown: Record<string, boolean>;
  /** 1..14 days, today included. */
  daysAhead: number;
  showDeclined: boolean;
  /** Minutes before an event that the pill mentions it (0 = never). */
  pillMinutes: number;
}

export interface Settings {
  version: number;
  general: { startWithWindows: boolean; showTray: boolean; hideFullscreen: boolean; language: string };
  appearance: Appearance;
  modules: Modules;
  hotkeys: Hotkeys;
  alerts: Alerts;
  usage: { showInPill: boolean; ringWindow: "fiveHour" | "sevenDay"; fromSignIn: boolean };
  chatWatcher: { onlyWhenUnfocused: boolean };
  spotify: { connected: boolean; clientId: string; lyrics: boolean; showInPill: boolean; progressLine: boolean; pillText: "track" | "lyrics"; pillPriority: "claude" | "spotify"; callbackPort: number };
  clipboard: {
    persist: boolean;
    maxItems: number;
    images: boolean;
    files: boolean;
    clickPaste: boolean;
    paused: boolean;
    skipPrivate: boolean;
    encrypt: boolean;
    // ---- search-clipboard: begin ----
    detectSecrets: boolean;
    /** 0 = never */
    secretExpiryMin: number;
    ocr: boolean;
    // ---- search-clipboard: end ----
  };
  // ---- search-clipboard: begin ----
  search: { seeded: boolean };
  // ---- search-clipboard: end ----
  /** clearAfterMin: an error or "needs you" clears itself after this many minutes (0 = never). */
  claude: {
    liveActivity: boolean;
    askPermissions: boolean;
    soundOnDone: boolean;
    popupOnDone: boolean;
    clearAfterMin: number;
    /** Also show Claude chats in the Agent tab. */
    showChats: boolean;
    /** What the Agent tab shows: Code while it is in use (auto), or always one. */
    chatSource: "auto" | "code" | "chat";
  };
  whiteboard: { autoAddCopies: boolean; clickPaste: boolean };
  mascot: { tone: "peri" | "mint" | "peach" | "lilac"; reactMusic: boolean; reactPokes: boolean };
  focus: FocusSettings;
  calendar: CalendarSettings;
  // ---- ask-assistant: begin ----
  /** Ask, the AI assistant tab (the API key lives in Windows Credential Manager). */
  ask: {
    provider: "gemini" | "groq" | "ollama";
    /** Use the other cloud provider when the main one can't answer. */
    backup: boolean;
    model: string;
    /** "" = automatic */
    groqModel: string;
    /** "" = the first model installed */
    ollamaModel: string;
    keepHistory: boolean;
    seeded: boolean;
    seedVersion: number;
  };
  // ---- ask-assistant: end ----
  /** Tab order of the expanded island (module keys). */
  moduleOrder: string[];
}

export type ResetSection =
  | "general"
  | "appearance"
  | "colors"
  | "modules"
  | "agents"
  | "spotify"
  | "calendar"
  | "clipboard"
  | "focus"
  | "alerts"
  | "hotkeys"
  | "mascot"
  | "ask" // ask-assistant
  | "all";

export interface ClaudeStatus {
  state: "connected" | "notfound" | "error" | "off";
  installed: boolean;
  hooks: import("./api").HookStatus;
  lastEventAt: number;
}

export interface MonitorInfo {
  name: string;
  width: number;
  height: number;
  primary: boolean;
  /** Windows display scaling (1.25 = 125 %). */
  scale: number;
}

/** Deep-partial patch sent to update_settings. */
export type Patch = { [K in keyof Settings]?: Partial<Settings[K]> | any };

export const settingsApi = {
  get: () => invoke<Settings>("get_settings"),
  update: (patch: Patch) => invoke<Settings>("update_settings", { patch }),
  reset: (section: ResetSection) =>
    invoke<Settings>("reset_settings", { section }),
  hotkeyStatus: () => invoke<Record<string, string | null>>("get_hotkey_status"),
  validateHotkey: (combo: string) => invoke<void>("validate_hotkey", { combo }),
  pauseHotkeys: (paused: boolean) => invoke<void>("pause_hotkeys", { paused }),
  monitors: () => invoke<MonitorInfo[]>("list_monitors"),
  /** Open Settings; with `page`, on that page instead of where it was left. */
  openDashboard: (page?: "agents" | "spotify" | "calendar" | "ask") => invoke<void>("open_dashboard", { page: page ?? null }),
  islandVisible: () => invoke<boolean>("get_island_visible"),
  setIslandVisible: (visible: boolean) => invoke<void>("set_island_visible", { visible }),
  autostartEnabled: () => invoke<boolean>("autostart_enabled"),
  testAlert: (kind: "done" | "attention" | "error" | "usage" | "chat" | "timer") => invoke<void>("test_alert", { kind }),
  previewSound: (sound: SoundName, volume: number) => invoke<void>("preview_sound", { sound, volume }),
  exportSettings: () => invoke<boolean>("export_settings"),
  importSettings: () => invoke<boolean>("import_settings"),
  clearAllData: () => invoke<void>("clear_all_data"),
  testConnection: () => invoke<string>("test_connection"),
  claudeStatus: () => invoke<ClaudeStatus>("claude_status"),
  openFolder: (which: "data" | "logs") => invoke<void>("open_folder", { which }),
  chatWatchStatus: () => invoke<{ running: boolean; lastSignalAt: number }>("get_chatwatch_status"),
  appInfo: () =>
    invoke<{ version: string; port: number; settingsPath: string; dataDir: string; exe: string }>("app_info"),
};

export const onSettings = (cb: (s: Settings) => void): Promise<UnlistenFn> =>
  listen<Settings>("settings", (e) => cb(e.payload));

export const onHotkeyStatus = (cb: (s: Record<string, string | null>) => void): Promise<UnlistenFn> =>
  listen<Record<string, string | null>>("hotkey-status", (e) => cb(e.payload));
