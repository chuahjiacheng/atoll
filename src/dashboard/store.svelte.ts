// Settings window state.
//
// Controls call `change("appearance.collapsedWidth", 320)`: the local copy
// updates instantly (sliders stay smooth), the change goes to Rust once per
// animation frame, Rust clamps, saves and broadcasts, and the real island
// follows the broadcast (that is the live preview on the real island).
import { onHotkeyStatus, onSettings, settingsApi, type ResetSection, type Settings } from "../lib/settings";
import { seconds } from "../lib/motion";

const ms = (name: string) => seconds(name) * 1000;

export const store = $state({
  settings: null as Settings | null,
  hotkeyStatus: {} as Record<string, string | null>,
  /** Bottom-right confirmation ("Saved"). */
  toast: { text: "", at: 0, error: false },
  /** Confirm dialog for destructive or page-wide actions. */
  dialog: null as null | { title: string; body: string; ok: string; run: () => Promise<void> | void },
});

let pending: Record<string, any> = {};
let frame = 0;
let lastLocalChange = 0;
let resyncTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;

function setPath(target: Record<string, any>, path: string[], value: unknown) {
  let obj = target;
  for (const key of path.slice(0, -1)) {
    if (typeof obj[key] !== "object" || obj[key] === null) obj[key] = {};
    obj = obj[key];
  }
  obj[path[path.length - 1]] = value;
}

export function toast(text: string, error = false) {
  store.toast = { text, at: Date.now(), error };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (store.toast = { text: "", at: 0, error: false }), ms(error ? "--dur-toast-error" : "--dur-toast"));
}

/** Change one setting by dotted path, e.g. "appearance.busy.preset". */
export function change(path: string, value: unknown) {
  if (!store.settings) return;
  const parts = path.split(".");
  setPath(store.settings as any, parts, value);
  setPath(pending, parts, value);
  lastLocalChange = Date.now();
  if (!frame) frame = requestAnimationFrame(flush);
}

async function flush() {
  frame = 0;
  const patch = pending;
  pending = {};
  try {
    await settingsApi.update(patch);
    toast("Saved");
  } catch (e) {
    toast(String(e), true);
  }
  // Once the user pauses, pull the stored values (they may have been clamped).
  clearTimeout(resyncTimer);
  resyncTimer = setTimeout(async () => (store.settings = await settingsApi.get()), ms("--dur-resync"));
}

/** Run an action that saves on its own (export, clear, …) and confirm it. */
export async function act(run: () => Promise<unknown>, done: string) {
  try {
    await run();
    if (done) toast(done);
  } catch (e) {
    toast(String(e), true);
  }
}

export async function reset(section: ResetSection, label: string) {
  store.settings = await settingsApi.reset(section);
  toast(`${label} reset to defaults`);
}

/** Ask before doing something that can't be undone. */
export function confirm(title: string, body: string, ok: string, run: () => Promise<void> | void) {
  store.dialog = { title, body, ok, run };
}

export async function init() {
  store.settings = await settingsApi.get();
  store.hotkeyStatus = await settingsApi.hotkeyStatus();
  onSettings((s) => {
    // Ignore echoes of our own in-flight edits; accept outside changes.
    if (Date.now() - lastLocalChange > ms("--dur-resync")) store.settings = s;
  });
  onHotkeyStatus((s) => (store.hotkeyStatus = s));
}
