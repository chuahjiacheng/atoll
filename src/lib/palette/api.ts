// Universal search: types + commands (Rust side: src-tauri/src/search).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Source = "clipboard" | "whiteboard" | "note" | "fileTray" | "calendar" | "agent";
export type Action = "paste" | "copy" | "open" | "reveal";

export interface SearchResult {
  key: string;
  source: Source;
  title: string;
  sub: string;
  at: number;
  pinned: boolean;
  kind: string;
  score: number;
  /** The first is what Enter does. */
  actions: Action[];
}

export const searchApi = {
  query: (query: string, source: Source | null, limit = 30) => invoke<SearchResult[]>("search_query", { query, source, limit }),
  act: (key: string, action: Action) => invoke<void>("search_act", { key, action }),
  stats: () => invoke<{ documents: number; bytes: number }>("search_stats"),
  ready: () => invoke<void>("palette_ready"),
  hide: () => invoke<void>("palette_hide"),
};

export const onPaletteOpen = (cb: () => void): Promise<UnlistenFn> => listen("palette-open", () => cb());

export const SOURCES: { key: Source | null; label: string; icon: string }[] = [
  { key: null, label: "All", icon: "search" },
  { key: "clipboard", label: "Clipboard", icon: "clipboard" },
  { key: "whiteboard", label: "Whiteboard", icon: "whiteboard" },
  { key: "note", label: "Note", icon: "note" },
  { key: "fileTray", label: "Files", icon: "folder" },
  { key: "calendar", label: "Calendar", icon: "calendar" },
  { key: "agent", label: "Agent", icon: "agent" },
];

export const ACTION_LABEL: Record<Action, string> = {
  paste: "Paste",
  copy: "Copy",
  open: "Open",
  reveal: "Show in folder",
};

/**
 * Width contract (docs/contracts/width-contract.md): the palette is a peek.
 * Requested as island state "peek:palette" in docs/requests/search-clipboard.md;
 * mirrored by WIDTH in src-tauri/src/search/palette.rs.
 */
export const PALETTE_WIDTH = { id: "peek:palette", min: 560, preferred: 560, max: 560, overflow: "ellipsis", height: "content" } as const;
