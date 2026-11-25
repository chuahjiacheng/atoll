// Every island module, its switch key and when it ships.
// `ready: false` modules keep their switch (the choice is remembered) but do nothing yet.
import type { ModuleKey } from "./settings";

export interface ModuleInfo {
  key: ModuleKey;
  name: string;
  description: string;
  milestone: number;
  ready: boolean;
  /** What the module's own global hotkey does (when it has one). */
  hotkeyAction?: string;
}

export const MODULES: ModuleInfo[] = [
  {
    key: "claude",
    name: "Claude Code",
    description: "Live activity per session and Allow / Deny for permission prompts.",
    milestone: 2,
    ready: true,
    hotkeyAction: "Open the island on your sessions",
  },
  { key: "ask", name: "Ask", description: "Ask Gemini anything from the island. Replies stream in, with code you can copy.", milestone: 4, ready: true, hotkeyAction: "Open Ask" }, // ask-assistant
  { key: "alerts", name: "Alerts", description: "Chime, colour flash and Windows notification when a session finishes or needs you.", milestone: 2, ready: true },
  { key: "usageRing", name: "Usage ring", description: "Your 5-hour and weekly Claude limits, read from Claude Code's status line.", milestone: 2, ready: true },
  { key: "chatWatcher", name: "Chat tab watcher", description: "Experimental: alert when a reply finishes in the Claude desktop Chat tab.", milestone: 2, ready: true },
  { key: "spotify", name: "Spotify", description: "Controls, progress line, playback devices and synced lyrics.", milestone: 3, ready: true, hotkeyAction: "Play / pause" },
  { key: "calendar", name: "Google Calendar", description: "Today's events and the days ahead, with one-click Join for calls.", milestone: 3, ready: true, hotkeyAction: "Open calendar" },
  { key: "clipboard", name: "Clipboard history", description: "Your recent copies: text, images and files. Click one to paste it.", milestone: 3, ready: true, hotkeyAction: "Open clipboard history" },
  { key: "whiteboard", name: "Whiteboard", description: "Freeform canvas of text, images and files. Click to paste, drag out to use.", milestone: 3, ready: true, hotkeyAction: "Open whiteboard" },
  { key: "fileTray", name: "File tray", description: "Drop files on the island to keep them handy, drag them back out.", milestone: 3, ready: true, hotkeyAction: "Open file tray" },
  { key: "note", name: "Quick note", description: "One always-there note, saved as you type.", milestone: 3, ready: true, hotkeyAction: "Open note" },
  { key: "focus", name: "Focus timer", description: "Focus and break sessions that hold back alerts and show them at the break.", milestone: 3, ready: true, hotkeyAction: "Start or pause the focus timer" },
  { key: "mascot", name: "Mascot", description: "Companion with moods and outfits that reacts to your work and music.", milestone: 4, ready: false },
  { key: "screenQuestion", name: "Ask about my screen", description: "Drag the mascot onto a window and ask Gemini about it.", milestone: 4, ready: false, hotkeyAction: "Ask about the active window" },
];
