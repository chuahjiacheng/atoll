// Settings presets (mirrors src-tauri/src/presets.rs). Stored on this PC in
// %APPDATA%\Crest\presets.json.
import { invoke } from "@tauri-apps/api/core";

export interface PresetInfo {
  id: string;
  name: string;
  /** ms since epoch */
  savedAt: number;
}

export interface PresetList {
  /** The preset last switched to or saved. */
  active: string | null;
  /** Current settings differ from the active preset. */
  modified: boolean;
  presets: PresetInfo[];
}

export const MAX_NAME = 40;

export const presetsApi = {
  list: () => invoke<PresetList>("list_presets"),
  save: (name: string) => invoke<PresetList>("save_preset", { name }),
  overwrite: (id: string) => invoke<PresetList>("overwrite_preset", { id }),
  apply: (id: string) => invoke<PresetList>("apply_preset", { id }),
  rename: (id: string, name: string) => invoke<PresetList>("rename_preset", { id, name }),
  remove: (id: string) => invoke<PresetList>("delete_preset", { id }),
};

// ---------- routines (src-tauri/src/routines.rs) ----------

export type When =
  | { kind: "time"; /** "HH:MM" */ at: string; /** 0 = Sunday … 6 = Saturday */ days: number[] }
  | { kind: "displays"; count: number; orMore: boolean };

export interface Routine {
  /** "" = new */
  id: string;
  enabled: boolean;
  presetId: string;
  when: When;
}

export interface RoutineList {
  options: { paused: boolean; notify: boolean };
  routines: (Routine & { label: string })[];
  /** Screen routines that could fire on the same change. */
  overlaps: string[];
  /** The last automatic switch, while it can still be undone. */
  lastSwitch: { presetId: string; at: number; reason: string } | null;
  /** Screens connected now. */
  displays: number;
}

export const routinesApi = {
  list: () => invoke<RoutineList>("list_routines"),
  save: (routine: Routine) => invoke<RoutineList>("save_routine", { routine }),
  remove: (id: string) => invoke<RoutineList>("delete_routine", { id }),
  options: (o: { paused?: boolean; notify?: boolean }) =>
    invoke<RoutineList>("set_routine_options", { paused: o.paused ?? null, notify: o.notify ?? null }),
  undo: () => invoke<RoutineList>("undo_routine_switch"),
};
