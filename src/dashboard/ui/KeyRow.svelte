<!--
  Hotkey recorder (design: keys row). Select the field, press a combination
  with Ctrl, Alt, Shift or Win. Esc cancels, Backspace clears. While
  recording, Atoll releases its own hotkeys so they can be pressed here.
  Stored as "Ctrl+Alt+KeyK" (KeyboardEvent.code), the format Rust parses.
-->
<script lang="ts" module>
  /** "Ctrl+Alt+KeyK" -> ["Ctrl", "Alt", "K"] */
  export function caps(combo: string): string[] {
    if (!combo) return [];
    const names: Record<string, string> = {
      Super: "Win",
      Period: ".",
      Comma: ",",
      Slash: "/",
      Backslash: "\\",
      Semicolon: ";",
      Quote: "'",
      BracketLeft: "[",
      BracketRight: "]",
      Minus: "-",
      Equal: "=",
      Backquote: "`",
      ArrowUp: "Up",
      ArrowDown: "Down",
      ArrowLeft: "Left",
      ArrowRight: "Right",
    };
    return combo.split("+").map((p) => names[p] ?? p.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Numpad/, "Num "));
  }
  export const pretty = (combo: string) => caps(combo).join(" + ");
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import { settingsApi } from "../../lib/settings";
  import Row from "./Row.svelte";

  let {
    label,
    value,
    def,
    conflictWith = [],
    error = null,
    onchange,
  }: {
    label: string;
    value: string;
    /** Default combination (mirrors src-tauri/src/settings.rs). */
    def: string;
    /** Labels of other shortcuts using the same combination. */
    conflictWith?: string[];
    /** Why Windows refused it (taken by another app), if it did. */
    error?: string | null;
    onchange: (combo: string) => void;
  } = $props();

  let recording = $state(false);
  let invalid = $state("");
  const MODS = ["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"];

  function setRecording(on: boolean) {
    if (on === recording) return;
    recording = on;
    invalid = "";
    settingsApi.pauseHotkeys(on).catch(() => {});
  }

  async function keydown(e: KeyboardEvent) {
    if (!recording || e.key === "Tab") return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape") return setRecording(false);
    if (e.code === "Backspace" || e.code === "Delete") {
      setRecording(false);
      onchange("");
      return;
    }
    if (MODS.includes(e.code)) return; // wait for the real key
    const parts: string[] = [];
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.altKey) parts.push("Alt");
    if (e.shiftKey) parts.push("Shift");
    if (e.metaKey) parts.push("Super");
    if (!parts.length) {
      invalid = "Add Ctrl, Alt, Shift or Win to the key.";
      return;
    }
    parts.push(e.code);
    const combo = parts.join("+");
    try {
      await settingsApi.validateHotkey(combo);
      setRecording(false);
      onchange(combo);
    } catch (err) {
      invalid = String(err);
    }
  }

  const conflict = $derived(!recording && conflictWith.length > 0);
  const note = $derived(
    recording
      ? invalid || "Press the new combination with Ctrl, Alt, Shift or Win. Esc cancels, Backspace clears."
      : conflict
        ? `${pretty(value)} is also used by "${conflictWith.join('", "')}". Choose another combination.`
        : error
          ? `Windows would not register ${pretty(value)}: ${error}`
          : "",
  );
  const tone = $derived(recording ? (invalid ? "err" : "muted") : "err");
</script>

<Row {label} {note} noteIcon={recording && !invalid ? "info" : "alert"} noteTone={tone}>
  <button
    class="field"
    class:recording
    class:bad={conflict || !!error}
    aria-label="Shortcut for {label}: {value ? pretty(value) : 'not set'}{conflict ? ', conflict' : ''}. Select to change."
    onclick={() => setRecording(!recording)}
    onkeydown={keydown}
    onblur={() => setRecording(false)}
  >
    {#if recording}
      <span class="prompt">Press a key combination</span>
    {:else if value}
      {#each caps(value) as cap, i (i)}<kbd>{cap}</kbd>{/each}
    {:else}
      <span class="unset">Not set</span>
    {/if}
    <span class="grow"></span>
    {#if conflict || error}<span class="warn" aria-hidden="true"><Icon name="alert" size={16} /></span>{/if}
  </button>
  <button
    class="icon-btn bare"
    aria-label="Reset the shortcut for {label}"
    title="Reset to {pretty(def) || 'not set'}"
    disabled={value === def}
    onclick={() => onchange(def)}
  >
    <Icon name="reset" size={18} />
  </button>
</Row>

<style>
  .field {
    box-sizing: border-box;
    width: var(--size-key-field);
    height: var(--size-control);
    padding: 0 var(--sp-10) 0 var(--sp-8);
    border-radius: var(--r-control);
    border: 1px solid var(--c-ctrl);
    background: var(--c-field);
    display: flex;
    align-items: center;
    gap: var(--sp-4);
  }
  .field:hover {
    background-image: linear-gradient(var(--c-hover), var(--c-hover));
  }
  .field.bad {
    border: var(--focus-width) solid var(--c-err);
  }
  .field.recording {
    border: var(--focus-width) solid var(--c-accent);
  }
  kbd {
    box-sizing: border-box;
    min-width: var(--size-keycap-w);
    height: var(--size-keycap-h);
    padding: 0 var(--sp-8);
    border-radius: var(--r-keycap);
    background: var(--c-kbd);
    box-shadow: inset 0 -1px 0 var(--c-ctrl);
    font: inherit;
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .prompt {
    font-size: var(--fs-small);
    color: var(--c-accent-text);
  }
  .unset {
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .grow {
    flex: 1;
  }
  .warn {
    display: flex;
    color: var(--c-err);
  }
  .icon-btn:disabled {
    color: var(--c-dis);
    opacity: 1;
  }
</style>
