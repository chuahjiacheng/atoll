<script lang="ts">
  import type { Settings } from "../../lib/settings";
  import { change, store } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import KeyRow from "../ui/KeyRow.svelte";

  let { s }: { s: Settings } = $props();

  // Defaults mirror Hotkeys::default and default_module_hotkeys in src-tauri/src/settings.rs.
  const ROWS: { key: string; label: string; def: string }[] = [
    { key: "toggleIsland", label: "Show or hide the island", def: "Ctrl+Alt+Space" },
    { key: "expandIsland", label: "Open or close the island", def: "Ctrl+Alt+Period" },
    { key: "openSettings", label: "Open settings", def: "Ctrl+Alt+KeyS" },
    { key: "module:clipboard", label: "Open clipboard history", def: "Ctrl+Alt+KeyV" },
    { key: "module:note", label: "Open the quick note", def: "Ctrl+Alt+KeyN" },
    { key: "module:whiteboard", label: "Open the whiteboard", def: "" },
    { key: "module:fileTray", label: "Open the file tray", def: "" },
    { key: "module:spotify", label: "Play or pause music", def: "Ctrl+Alt+KeyP" },
    { key: "module:allowPermission", label: "Allow a permission request", def: "Ctrl+Alt+KeyY" },
    { key: "module:denyPermission", label: "Deny a permission request", def: "Ctrl+Alt+KeyX" },
    { key: "module:dismissMessage", label: "Clear the island's message", def: "Ctrl+Alt+KeyD" },
    { key: "module:focus", label: "Start or pause the focus timer", def: "Ctrl+Alt+KeyF" },
    { key: "module:palette", label: "Search everything", def: "Ctrl+Alt+KeyK" }, // search-clipboard
    { key: "module:ask", label: "Open Ask", def: "Ctrl+Alt+KeyA" }, // ask-assistant
  ];

  const valueOf = (key: string) =>
    key.startsWith("module:") ? (s.hotkeys.modules[key.slice(7)] ?? "") : ((s.hotkeys as unknown as Record<string, string>)[key] ?? "");
  const pathOf = (key: string) => (key.startsWith("module:") ? `hotkeys.modules.${key.slice(7)}` : `hotkeys.${key}`);

  const conflicts = $derived.by(() => {
    const users = new Map<string, string[]>();
    for (const r of ROWS) {
      const v = valueOf(r.key).toLowerCase();
      if (v) users.set(v, [...(users.get(v) ?? []), r.label]);
    }
    return Object.fromEntries(
      ROWS.map((r) => [r.key, (users.get(valueOf(r.key).toLowerCase()) ?? []).filter((l) => l !== r.label)]),
    );
  });
</script>

<Section title="Shortcuts" desc="They work from anywhere in Windows. Select a shortcut, then press the new combination.">
  {#each ROWS as r (r.key)}
    <KeyRow
      label={r.label}
      value={valueOf(r.key)}
      def={r.def}
      conflictWith={valueOf(r.key) ? conflicts[r.key] : []}
      error={store.hotkeyStatus[r.key] ?? null}
      onchange={(v) => change(pathOf(r.key), v)}
    />
  {/each}
</Section>
