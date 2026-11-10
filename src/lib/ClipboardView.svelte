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

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? clips.filter((c) => (c.text ?? "").toLowerCase().includes(q) || c.files.some((f) => f.toLowerCase().includes(q)))
      : clips;
    return [...list.filter((c) => c.pinned), ...list.filter((c) => !c.pinned)];
  });

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
    openMenu(e, [
      { label: "Paste", icon: "send", hint: clickPaste ? "Click" : undefined, run: () => act(c, true) },
      { label: "Copy", icon: "clipboard", hint: clickPaste ? "Shift+click" : "Click", run: () => act(c, false) },
      null,
      file && { label: "Open", icon: "external", run: () => run(clipApi.openFile(c.id, false)) },
      file && { label: "Show in folder", icon: "folder", run: () => run(clipApi.openFile(c.id, true)) },
      file && null,
      { label: "Add to whiteboard", icon: "whiteboard", run: () => run(clipApi.send(c.id, "whiteboard"), "Added to the whiteboard") },
      { label: c.kind === "files" ? "Keep in file tray" : "Save to file tray", icon: "folder", run: () => run(clipApi.send(c.id, "fileTray"), "Added to the file tray") },
      null,
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
    const kind = c.kind === "image" ? "Image" : c.kind === "files" ? "File" : "Text";
    const from = c.source ? ` · ${c.source.replace(/\.exe$/, "")}` : "";
    return `${kind}${from} · ${ago(c.at, now)}`;
  }
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
            <span class="tile" class:image={c.kind === "image"}>
              {#if c.kind === "image" && c.thumb}
                <img use:clipThumb={c} alt="" />
              {:else}
                <Icon name={c.kind === "files" ? "file" : c.kind === "image" ? "image" : "text"} size={20} />
              {/if}
            </span>
            <span class="text">
              <span class="ellipsis one">{title(c)}</span>
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
  .title { font-size: var(--fs-body); font-weight: var(--fw-strong); }
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
  .text { min-width: 0; display: flex; flex-direction: column; font-size: var(--fs-small); }
  .tool { width: var(--size-tab); height: var(--size-tab); border-radius: var(--r-pill); display: grid; place-items: center; color: var(--c-muted); opacity: 0; flex: none; }
  .row.sel .tool, .tool:focus-visible, .tool.on { opacity: 1; }
  .tool:hover { color: var(--c-ink); background: var(--c-hover); }
  .tool.on { color: var(--c-accent-text); }
  .none { margin: var(--sp-8) var(--sp-4); }
  .msg { margin: var(--sp-8) 0 0; color: var(--c-done); font-size: var(--fs-small); }
  .msg.error { color: var(--c-error); }
</style>
