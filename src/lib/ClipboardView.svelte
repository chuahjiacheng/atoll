<!--
  Clipboard tab (design: Island → clipboard). Recent copies, newest first,
  pinned on top. Click = paste into the app you were using (or copy only, per
  settings); Shift+click = copy only. Pause stops recording.
  Keyboard: type to search, ↑/↓ to choose, Enter to use, Esc to close.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { SearchField } from "glisse";
  import { ago, clipApi, clipThumb, onClipboard, onDragOutEnded, toast, type Clip } from "./tools.svelte";
  import { openMenu } from "./ui/menu.svelte";
  import { settingsApi } from "./settings";
  import GButton from "./glass/GButton.svelte";
  import { react } from "./glass/bridge";
  import Icon from "./ui/Icon.svelte";
  import Pip, { type Tone } from "./ui/Pip.svelte";

  let { clickPaste, paused, tone, onDone }: { clickPaste: boolean; paused: boolean; tone: Tone; onDone: () => void } = $props();

  let clips = $state<Clip[]>([]);
  let query = $state("");
  let selected = $state(0);
  let now = $state(Date.now());
  let searchHost = $state<HTMLDivElement>();
  const msg = toast();

  onMount(() => {
    clipApi.list().then((c) => (clips = c));
    const un = onClipboard((c) => (clips = c));
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      un.then((f) => f());
      clearInterval(t);
    };
  });

  // What a row is, for its icon, label and the filter chips.
  type Sort = "link" | "colour" | "code" | "json" | "image" | "files" | "secret" | "text";
  function sortOf(c: Clip): Sort {
    if (c.secret) return "secret";
    if (c.kind !== "text") return c.kind === "image" ? "image" : "files";
    return (c.types?.[0] as Sort) ?? "text";
  }
  const SORT_LABEL: Record<Sort, string> = {
    link: "Link",
    colour: "Colour",
    code: "Code",
    json: "JSON",
    image: "Image",
    files: "File",
    secret: "Secret",
    text: "Text",
  };
  const FILTERS: { key: Sort; label: string }[] = [
    { key: "link", label: "Links" },
    { key: "code", label: "Code" },
    { key: "json", label: "JSON" },
    { key: "colour", label: "Colours" },
    { key: "image", label: "Images" },
    { key: "secret", label: "Secrets" },
  ];
  let filter = $state<Sort | null>(null);
  // Only offer chips for sorts that are in the history.
  const chips = $derived(FILTERS.filter((f) => clips.some((c) => sortOf(c) === f.key)));
  $effect(() => {
    if (filter && !chips.some((f) => f.key === filter)) filter = null;
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const hay = (c: Clip) => [c.text ?? "", c.ocr ?? "", c.secret ?? "", ...c.files].join("\n").toLowerCase();
    const list = clips.filter((c) => (!filter || sortOf(c) === filter) && (!q || hay(c).includes(q)));
    return [...list.filter((c) => c.pinned), ...list.filter((c) => !c.pinned)];
  });

  const isColour = (c: Clip) => sortOf(c) === "colour" && !!c.text && CSS.supports("color", c.text.trim());

  /** "in 4 min" until a secret is forgotten. */
  function forgets(c: Clip) {
    if (!c.expiresAt) return "";
    const m = Math.max(1, Math.ceil((c.expiresAt - now) / 60_000));
    return ` · forgotten in ${m} min`;
  }

  const TRANSFORM_LABEL: Record<string, string> = {
    jsonPretty: "Copy as formatted JSON",
    jsonMinify: "Copy as one-line JSON",
    stripTracking: "Copy without tracking",
  };
  function runTransform(c: Clip, op: string, done: string) {
    run(clipApi.transform(c.id, op, false), done);
  }

  $effect(() => {
    if (selected >= shown.length) selected = Math.max(0, shown.length - 1);
  });

  export function focusSearch() {
    searchHost?.querySelector("input")?.focus();
  }

  async function use(c: Clip, shift = false) {
    const paste = clickPaste && !shift;
    await act(c, paste);
  }

  async function act(c: Clip, paste: boolean) {
    try {
      await clipApi.use(c.id, paste);
      if (paste) onDone();
      else msg.show("Copied");
    } catch (e) {
      msg.show(String(e), true);
    }
  }

  async function run(p: Promise<unknown>, ok?: string) {
    try {
      await p;
      if (ok) msg.show(ok);
    } catch (e) {
      msg.show(String(e), true);
    }
  }

  // Drag a row out of the island (text as text, images and files as files).
  let draggingOut = $state(false);
  let press: { id: string; sx: number; sy: number; started: boolean } | null = null;
  let dragged = false;
  onMount(() => {
    const un = onDragOutEnded(() => (draggingOut = false));
    return () => un.then((f) => f());
  });
  export function isDraggingOut() {
    return draggingOut;
  }
  function down(e: PointerEvent, c: Clip) {
    dragged = false;
    if (e.button !== 0) return;
    press = { id: c.id, sx: e.clientX, sy: e.clientY, started: false };
  }
  function move(e: PointerEvent) {
    if (!press || press.started || !(e.buttons & 1) || Math.hypot(e.clientX - press.sx, e.clientY - press.sy) < 6) return;
    press.started = true;
    dragged = true;
    draggingOut = true;
    clipApi.drag(press.id).catch((err) => {
      draggingOut = false;
      msg.show(String(err), true);
    });
  }
  function click(e: MouseEvent, c: Clip) {
    press = null;
    if (dragged) {
      dragged = false;
      return;
    }
    use(c, e.shiftKey);
  }

  function rowMenu(e: MouseEvent, c: Clip) {
    const file = c.kind === "files";
    const transforms = c.transforms ?? [];
    // Secrets stay out of the whiteboard and file tray, where they would be stored unmasked.
    const share = !c.secret;
    openMenu(e, [
      { label: "Paste", icon: "send", hint: clickPaste ? "Click" : undefined, run: () => act(c, true) },
      { label: "Copy", icon: "clipboard", hint: clickPaste ? "Shift+click" : "Click", run: () => act(c, false) },
      ...transforms.map((op) => ({ label: TRANSFORM_LABEL[op] ?? op, icon: "edit", run: () => runTransform(c, op, "Copied") })),
      c.ocr ? { label: "Copy text from image", icon: "text", run: () => runTransform(c, "ocrText", "Text copied") } : false,
      null,
      file && { label: "Open", icon: "external", run: () => run(clipApi.openFile(c.id, false)) },
      file && { label: "Show in folder", icon: "folder", run: () => run(clipApi.openFile(c.id, true)) },
      file && null,
      share && { label: "Add to whiteboard", icon: "whiteboard", run: () => run(clipApi.send(c.id, "whiteboard"), "Added to the whiteboard") },
      share && { label: c.kind === "files" ? "Keep in file tray" : "Save to file tray", icon: "folder", run: () => run(clipApi.send(c.id, "fileTray"), "Added to the file tray") },
      share && null,
      { label: c.pinned ? "Unpin" : "Pin", icon: "pin", run: () => clipApi.pin(c.id, !c.pinned) },
      { label: "Remove", icon: "x", danger: true, run: () => clipApi.remove(c.id) },
    ]);
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") selected = Math.min(shown.length - 1, selected + 1);
    else if (e.key === "ArrowUp") selected = Math.max(0, selected - 1);
    else if (e.key === "Enter" && shown[selected]) use(shown[selected], e.shiftKey);
    else return;
    e.preventDefault();
  }

  const name = (p: string) => p.split("\\").pop() ?? p;
  function title(c: Clip) {
    if (c.kind === "image") return `Image ${c.width} × ${c.height}`;
    if (c.kind === "files") return c.files.length === 1 ? name(c.files[0]) : `${name(c.files[0])} + ${c.files.length - 1} more`;
    return c.text ?? "";
  }
  function meta(c: Clip) {
    const kind = c.secret ?? (c.ocr ? "Image with text" : SORT_LABEL[sortOf(c)]);
    const from = c.source ? ` · ${appName(c.source)}` : "";
    return `${kind}${from} · ${ago(c.at, now)}${forgets(c)}`;
  }
  // Friendly names for common apps ("Code" would read like the Code type).
  const APPS: Record<string, string> = {
    code: "VS Code",
    winword: "Word",
    excel: "Excel",
    powerpnt: "PowerPoint",
    chrome: "Chrome",
    msedge: "Edge",
    firefox: "Firefox",
    explorer: "File Explorer",
    notepad: "Notepad",
    windowsterminal: "Terminal",
  };
  function appName(source: string) {
    const bare = source.replace(/\.exe$/i, "");
    return APPS[bare.toLowerCase()] ?? bare;
  }
  const ICON: Record<Sort, string> = {
    link: "external",
    colour: "palette",
    code: "terminal",
    json: "terminal",
    image: "image",
    files: "file",
    secret: "lock",
    text: "text",
  };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="clips" onkeydown={keydown}>
  <div class="head">
    <span class="title">Recent copies</span><span class="label tabular">{clips.length} {clips.length === 1 ? "item" : "items"}</span>
    <span class="flex"></span>
    <div
      class="search"
      bind:this={searchHost}
      use:react={{ component: SearchField, props: { value: query, onValueChange: (v: string) => (query = v), "aria-label": "Search copies" } }}
    ></div>
    <GButton
      icon={paused ? "play" : "pauseLine"}
      iconSize={16}
      label={paused ? "Resume" : "Pause"}
      title={paused ? "Start recording copies again" : "Stop recording new copies"}
      onclick={() => settingsApi.update({ clipboard: { paused: !paused } })}
    />
    <GButton icon="trash" iconSize={16} label="Clear" title="Remove everything except pinned items" disabled={!clips.some((c) => !c.pinned)} onclick={() => clipApi.clear()} />
  </div>

  {#if clips.length === 0}
    <div class="empty-state">
      <Pip mood="idle" {tone} size={64} />
      <div class="title">{paused ? "History is paused" : "Clipboard history is empty"}</div>
      <div class="hint">{paused ? "Press Resume to start saving copies again." : "Copy text, an image or a file, or drop one here, and it will appear here."}</div>
    </div>
  {:else}
    {#if chips.length > 1 || filter}
      <div class="chips" role="group" aria-label="Show only">
        <button class="chip" class:on={!filter} aria-pressed={!filter} onclick={() => (filter = null)}>
          {#if !filter}<Icon name="check" size={14} />{/if}All
        </button>
        {#each chips as f (f.key)}
          <button class="chip" class:on={filter === f.key} aria-pressed={filter === f.key} onclick={() => (filter = filter === f.key ? null : f.key)}>
            {#if filter === f.key}<Icon name="check" size={14} />{/if}{f.label}
          </button>
        {/each}
      </div>
    {/if}
    <div class="list" role="listbox" aria-label="Clipboard history" data-lenis-prevent>
      {#each shown as c, i (c.id)}
        <div
          class="row"
          class:sel={i === selected}
          role="option"
          tabindex="-1"
          aria-selected={i === selected}
          onmouseenter={() => (selected = i)}
          oncontextmenu={(e) => rowMenu(e, c)}
        >
          <button
            class="main"
            onpointerdown={(e) => down(e, c)}
            onpointermove={move}
            onclick={(e) => click(e, c)}
            title={clickPaste ? "Click to paste · Shift+click to copy only · drag into another app · right-click for more" : "Click to copy · drag into another app · right-click for more"}
          >
            <span class="tile" class:image={c.kind === "image"} class:secret={!!c.secret}>
              {#if c.kind === "image" && c.thumb}
                <img use:clipThumb={c} alt="" />
              {:else if isColour(c)}
                <span class="swatch" style:background={c.text?.trim()}></span>
              {:else}
                <Icon name={ICON[sortOf(c)]} size={20} />
              {/if}
            </span>
            <span class="text">
              <span class="ellipsis one" dir="auto">{title(c)}</span>
              <span class="label">{meta(c)}</span>
            </span>
          </button>
          <button class="tool" class:on={c.pinned} aria-label={c.pinned ? "Unpin" : "Pin"} title={c.pinned ? "Unpin" : "Pin (kept until you unpin it)"} onclick={() => clipApi.pin(c.id, !c.pinned)}>
            <Icon name="pin" size={16} />
          </button>
          <button class="tool" aria-label="Remove" title="Remove" onclick={() => clipApi.remove(c.id)}><Icon name="x" size={16} /></button>
        </div>
      {:else}
        <p class="label none">No copies match “{query}”.</p>
      {/each}
    </div>
  {/if}
  {#if msg.text}<p class="msg" class:error={msg.error} role="status">{msg.text}</p>{/if}
</div>

<style>
  .clips { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .head { display: flex; align-items: center; gap: var(--sp-8); margin-bottom: var(--sp-8); }
  .title { font-size: var(--fs-body); }
  .flex { flex: 1; }
  .search { width: var(--size-search); display: flex; }
  .list { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: var(--sp-8); padding-right: var(--sp-8); }
  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-height: var(--size-row);
    padding: var(--sp-8) var(--sp-8) var(--sp-8) var(--sp-8);
    box-sizing: border-box;
    border-radius: var(--r-control);
    background: var(--c-surface-card);
  }
  .row.sel { background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .main { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-16); text-align: left; }
  .tile {
    width: var(--size-tile);
    height: var(--size-tile);
    border-radius: var(--r-tile);
    flex: none;
    display: grid;
    place-items: center;
    background: var(--c-button);
    overflow: hidden;
  }
  .tile.image { background: var(--c-select-tile); }
  .tile img { width: 100%; height: 100%; object-fit: cover; }
  .tile.secret { color: var(--c-accent-text); }
  .swatch { width: 60%; height: 60%; border-radius: var(--r-pill); box-shadow: inset 0 0 0 1px var(--c-hairline); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-4); margin-bottom: var(--sp-8); }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    height: var(--size-tab);
    padding: 0 var(--sp-12);
    border-radius: var(--r-pill);
    font-size: var(--fs-small);
    color: var(--c-muted);
    background: var(--c-surface-card);
  }
  .chip:hover { color: var(--c-ink); background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .chip.on { color: var(--c-accent-ink); background: var(--c-accent); }
  .chip.on:hover { background-image: none; }
  .text { min-width: 0; display: flex; flex-direction: column; font-size: var(--fs-small); }
  .tool { width: var(--size-tab); height: var(--size-tab); border-radius: var(--r-pill); display: grid; place-items: center; color: var(--c-muted); opacity: 0; flex: none; }
  .row.sel .tool, .tool:focus-visible, .tool.on { opacity: 1; }
  .tool:hover { color: var(--c-ink); background: var(--c-hover); }
  .tool.on { color: var(--c-accent-text); }
  .none { margin: var(--sp-8) var(--sp-4); }
  .msg { margin: var(--sp-8) 0 0; color: var(--c-done); font-size: var(--fs-small); }
  .msg.error { color: var(--c-error); }
</style>
