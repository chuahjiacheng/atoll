// Clipboard history, whiteboard, file tray and quick note: types + commands
// (Rust side: src-tauri/src/clipboard.rs, board.rs, tray.rs).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Clip {
  id: string;
  kind: "text" | "image" | "files";
  text: string | null;
  files: string[];
  image: string | null;
  thumb: string | null;
  width: number;
  height: number;
  source: string | null;
  at: number;
  pinned: boolean;
  /** "link" | "colour" | "json" | "code" */
  types?: string[];
  /** "jsonPretty" | "jsonMinify" | "stripTracking" */
  transforms?: string[];
  /** What secret the text holds; `text` is then masked. */
  secret?: string;
  /** When a secret item is forgotten (ms). */
  expiresAt?: number;
  /** Text found in an image. */
  ocr?: string;
}

export interface Card {
  id: string;
  kind: "text" | "image" | "file";
  x: number;
  y: number;
  w: number;
  text: string | null;
  path: string | null;
  thumb: string | null;
  name: string | null;
  missing: boolean;
  at: number;
}

export interface TrayFile {
  id: string;
  path: string;
  name: string;
  size: number;
  isDir: boolean;
  thumb: string | null;
  missing: boolean;
  at: number;
}

export interface CardPatch {
  x?: number;
  y?: number;
  w?: number;
  text?: string;
  raise?: boolean;
}

/** URL for an image Atoll saved under %LOCALAPPDATA%\Crest (asset protocol). */
export const imageUrl = (path: string | null) => (path ? convertFileSrc(path) : "");

// Encrypted clipboard thumbnails ("….enc") can't be read from disk by the page;
// Rust decrypts them and returns a data URL, kept here so each loads once.
const sealedThumbs = new Map<string, string>();

/** `<img use:clipThumb={clip}>` shows a clip's thumbnail, encrypted or not. */
export function clipThumb(img: HTMLImageElement, clip: Clip) {
  let current = "";
  const load = async (c: Clip) => {
    if (!c.thumb || c.thumb === current) return;
    current = c.thumb;
    if (!c.thumb.endsWith(".enc")) {
      img.src = imageUrl(c.thumb);
      return;
    }
    let url = sealedThumbs.get(c.thumb);
    if (!url) {
      url = await invoke<string>("clipboard_thumb", { id: c.id }).catch(() => "");
      if (url) sealedThumbs.set(c.thumb, url);
    }
    if (current === c.thumb && url) img.src = url;
  };
  load(clip);
  return { update: load };
}

export const clipApi = {
  list: () => invoke<Clip[]>("clipboard_list"),
  use: (id: string, paste: boolean) => invoke<void>("clipboard_use", { id, paste }),
  pin: (id: string, pinned: boolean) => invoke<void>("clipboard_pin", { id, pinned }),
  remove: (id: string) => invoke<void>("clipboard_delete", { id }),
  clear: () => invoke<void>("clipboard_clear"),
  drag: (id: string) => invoke<void>("clipboard_drag", { id }),
  send: (id: string, to: "whiteboard" | "fileTray") => invoke<void>("clipboard_send", { id, to }),
  openFile: (id: string, reveal: boolean) => invoke<void>("clipboard_open_file", { id, reveal }),
  /** Put a transformed copy on the clipboard ("jsonPretty", "jsonMinify", "stripTracking", "ocrText"). */
  transform: (id: string, op: string, paste: boolean) => invoke<void>("clipboard_transform", { id, op, paste }),
};

export type DropTarget = "whiteboard" | "fileTray" | "clipboard";
export interface DropIn {
  type: "enter" | "over" | "leave" | "drop";
  /** Client position, physical px. */
  x: number;
  y: number;
  what: "files" | "text" | "image" | null;
}
export const dropApi = {
  commit: (target: DropTarget, x?: number, y?: number) => invoke<void>("drop_commit", { target, x, y }),
};
export const onDropIn = (cb: (e: DropIn) => void): Promise<UnlistenFn> => listen<DropIn>("drop-in", (e) => cb(e.payload));

export const boardApi = {
  get: () => invoke<{ cards: Card[] }>("board_get"),
  addFiles: (paths: string[], x?: number, y?: number) => invoke<void>("board_add_files", { paths, x, y }),
  addText: (text: string, x: number, y: number) => invoke<Card>("board_add_text", { text, x, y }),
  addClipboard: (x?: number, y?: number) => invoke<void>("board_add_clipboard", { x, y }),
  pickFiles: () => invoke<void>("board_pick_files"),
  update: (id: string, patch: CardPatch) => invoke<void>("board_update", { id, patch }),
  remove: (id: string) => invoke<void>("board_remove", { id }),
  clear: () => invoke<void>("board_clear"),
  use: (id: string, paste: boolean) => invoke<void>("board_use", { id, paste }),
  drag: (id: string) => invoke<void>("board_drag", { id }),
  open: (id: string) => invoke<void>("board_open", { id }),
  reveal: (id: string) => invoke<void>("board_reveal", { id }),
  send: (id: string, to: "fileTray" | "clipboard") => invoke<void>("board_send", { id, to }),
  duplicate: (id: string) => invoke<void>("board_duplicate", { id }),
};

export const trayApi = {
  list: () => invoke<TrayFile[]>("tray_list"),
  add: (paths: string[]) => invoke<void>("tray_add", { paths }),
  pickFiles: () => invoke<void>("tray_pick_files"),
  remove: (id: string) => invoke<void>("tray_remove", { id }),
  clear: () => invoke<void>("tray_clear"),
  open: (id: string) => invoke<void>("tray_open", { id }),
  reveal: (id: string) => invoke<void>("tray_reveal", { id }),
  copy: (ids: string[]) => invoke<void>("tray_copy", { ids }),
  drag: (ids: string[]) => invoke<void>("tray_drag", { ids }),
  paste: (ids: string[]) => invoke<void>("tray_paste", { ids }),
  toBoard: (ids: string[]) => invoke<void>("tray_to_board", { ids }),
  addClipboard: () => invoke<void>("tray_add_clipboard"),
};

export const noteApi = {
  get: () => invoke<string>("note_get"),
  set: (text: string) => invoke<void>("note_set", { text }),
};

export const onClipboard = (cb: (c: Clip[]) => void): Promise<UnlistenFn> => listen<Clip[]>("clipboard", (e) => cb(e.payload));
export const onBoard = (cb: (b: { cards: Card[] }) => void): Promise<UnlistenFn> =>
  listen<{ cards: Card[] }>("board", (e) => cb(e.payload));
export const onTray = (cb: (t: TrayFile[]) => void): Promise<UnlistenFn> => listen<TrayFile[]>("tray", (e) => cb(e.payload));
export const onNote = (cb: (t: string) => void): Promise<UnlistenFn> => listen<string>("note", (e) => cb(e.payload));
export const onDragOutEnded = (cb: (dropped: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("drag-out-ended", (e) => cb(e.payload));

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

export function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1, i + 5).toUpperCase() : "";
}

/** Tiny message line shown under a view for a couple of seconds. */
export function toast() {
  let msg = $state({ text: "", error: false });
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    get text() {
      return msg.text;
    },
    get error() {
      return msg.error;
    },
    show(text: string, error = false) {
      clearTimeout(timer);
      msg = { text, error };
      timer = setTimeout(() => (msg = { text: "", error: false }), error ? 4000 : 1600);
    },
  };
}
