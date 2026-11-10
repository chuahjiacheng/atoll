<!--
  Files tab (design: Island → files). Drop area on the left, kept files on
  the right. Atoll only remembers where files are; nothing is copied or moved.
  Click = open · drag = drop it into another app · copy / show in folder / remove.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { extOf, fileSize, imageUrl, onDragOutEnded, onTray, toast, trayApi, type TrayFile } from "./tools.svelte";
  import GButton from "./glass/GButton.svelte";
  import Icon from "./ui/Icon.svelte";
  import { openMenu } from "./ui/menu.svelte";

  let files = $state<TrayFile[]>([]);
  let draggingOut = $state(false);
  const msg = toast();
  let press: { ids: string[]; sx: number; sy: number; started: boolean; open?: string } | null = null;

  onMount(() => {
    trayApi.list().then((f) => (files = f));
    const un = [onTray((f) => (files = f)), onDragOutEnded(() => (draggingOut = false))];
    return () => un.forEach((p) => p.then((f) => f()));
  });

  export function isDraggingOut() {
    return draggingOut;
  }

  async function run(p: Promise<void>, ok?: string) {
    try {
      await p;
      if (ok) msg.show(ok);
    } catch (e) {
      msg.show(String(e), true);
    }
  }

  // Drag out after a small move (hysteresis), otherwise a click opens the file.
  function down(e: PointerEvent, ids: string[], open?: string) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("[data-noclick]")) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    press = { ids, sx: e.clientX, sy: e.clientY, started: false, open };
  }
  function move(e: PointerEvent) {
    if (!press || press.started || Math.hypot(e.clientX - press.sx, e.clientY - press.sy) < 6) return;
    press.started = true;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    draggingOut = true;
    trayApi.drag(press.ids).catch((err) => {
      draggingOut = false;
      msg.show(String(err), true);
    });
  }
  function up() {
    const p = press;
    press = null;
    if (p && !p.started && p.open) run(trayApi.open(p.open));
  }

  function rowMenu(e: MouseEvent, f: TrayFile) {
    const ok = !f.missing;
    openMenu(e, [
      { label: "Open", icon: "external", disabled: !ok, run: () => run(trayApi.open(f.id)) },
      { label: "Paste into last app", icon: "send", disabled: !ok, run: () => run(trayApi.paste([f.id])) },
      { label: "Copy", icon: "clipboard", disabled: !ok, run: () => run(trayApi.copy([f.id]), "Copied") },
      { label: "Show in folder", icon: "folder", disabled: !ok, run: () => run(trayApi.reveal(f.id)) },
      null,
      { label: "Add to whiteboard", icon: "whiteboard", disabled: !ok, run: () => run(trayApi.toBoard([f.id]), "Added to the whiteboard") },
      null,
      { label: "Remove from tray", icon: "x", danger: true, run: () => run(trayApi.remove(f.id)) },
    ]);
  }

  function areaMenu(e: MouseEvent) {
    openMenu(e, [
      { label: "Paste from clipboard", icon: "clipboard", run: () => run(trayApi.addClipboard(), "Added from the clipboard") },
      { label: "Choose files…", icon: "upload", run: () => run(trayApi.pickFiles()) },
      files.length > 1 && null,
      files.length > 1 && { label: "Copy all", icon: "clipboard", run: () => run(trayApi.copy(files.map((f) => f.id)), "Copied all files") },
      files.length > 0 && { label: "Clear tray", icon: "trash", danger: true, run: () => run(trayApi.clear()) },
    ]);
  }

  function meta(f: TrayFile) {
    if (f.missing) return "Moved or deleted";
    if (f.isDir) return "Folder";
    const ext = extOf(f.name);
    return `${ext ? `${ext} · ` : ""}${fileSize(f.size)}`;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="files" oncontextmenu={areaMenu}>
  <div class="drop" class:wide={files.length === 0}>
    <span class="up"><Icon name="upload" size={24} /></span>
    <div class="title">{files.length ? "Drop files here" : "The tray is empty"}</div>
    <div class="label">Drag files, images or text onto the island</div>
    <div class="btns">
      <GButton label="Choose files" onclick={() => run(trayApi.pickFiles())} />
      <GButton icon="clipboard" iconSize={16} label="Paste" title="Add what's on the clipboard" onclick={() => run(trayApi.addClipboard(), "Added from the clipboard")} />
    </div>
  </div>

  {#if files.length}
    <div class="side">
      <div class="list" role="list" data-lenis-prevent>
        {#each files as f (f.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="row"
            role="listitem"
            class:missing={f.missing}
            onpointerdown={(e) => !f.missing && down(e, [f.id], f.id)}
            onpointermove={move}
            onpointerup={up}
            oncontextmenu={(e) => rowMenu(e, f)}
            title={f.missing ? `${f.path} (moved or deleted)` : `${f.path}\nClick to open · drag into another app · right-click for more`}
          >
            <span class="tile">
              {#if f.thumb}<img src={imageUrl(f.thumb)} alt="" draggable="false" />{:else}<Icon name={f.isDir ? "folder" : "file"} size={20} />{/if}
            </span>
            <span class="text"><span class="ellipsis">{f.name}</span><span class="label">{meta(f)}</span></span>
            <span class="tools" data-noclick>
              {#if !f.missing}
                <button class="tool" aria-label="Copy {f.name}" title="Copy (paste it in Explorer, chat apps…)" onclick={() => run(trayApi.copy([f.id]), "Copied")}><Icon name="clipboard" size={16} /></button>
                <button class="tool" aria-label="Show {f.name} in folder" title="Show in folder" onclick={() => run(trayApi.reveal(f.id))}><Icon name="folder" size={16} /></button>
              {/if}
              <button class="tool" aria-label="Remove {f.name}" title="Remove from tray (the file stays where it is)" onclick={() => run(trayApi.remove(f.id))}><Icon name="x" size={16} /></button>
            </span>
          </div>
        {/each}
      </div>
      <div class="foot">
        {#if files.length > 1}
          <GButton label="Copy all" onclick={() => run(trayApi.copy(files.map((f) => f.id)), "Copied all files")} />
        {/if}
        <span class="flex"></span>
        <GButton label="Clear" icon="trash" iconSize={16} title="Empty the tray (your files stay where they are)" onclick={() => run(trayApi.clear())} />
      </div>
    </div>
  {/if}
</div>
{#if msg.text}<p class="msg" class:error={msg.error} role="status">{msg.text}</p>{/if}

<style>
  .files { display: flex; gap: var(--sp-16); flex: 1; min-height: 0; }
  .drop {
    width: var(--size-drop);
    flex: none;
    box-sizing: border-box;
    border-radius: var(--r-card);
    border: 2px dashed var(--c-dash);
    background: var(--c-surface-plate);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-8);
    text-align: center;
    padding: var(--sp-16);
  }
  .drop.wide { flex: 1; width: auto; }
  .up { color: var(--c-accent-text); display: flex; }
  .btns { display: flex; gap: var(--sp-8); flex-wrap: wrap; justify-content: center; }
  .title { font-size: var(--fs-body); font-weight: var(--fw-strong); }
  .side { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-8); min-height: 0; }
  .list { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: var(--sp-8); }
  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    min-height: var(--size-row);
    padding: 0 var(--sp-8);
    border-radius: var(--r-control);
    background: var(--c-surface-card);
    min-width: 0;
    cursor: pointer;
    flex: none;
  }
  .row:hover { background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .row.missing { opacity: 0.55; cursor: default; }
  .row.missing .ellipsis { text-decoration: line-through; }
  .tile { width: var(--size-tile); height: var(--size-tile); border-radius: var(--r-tile); flex: none; display: grid; place-items: center; background: var(--c-button); overflow: hidden; }
  .tile img { width: 100%; height: 100%; object-fit: cover; pointer-events: none; }
  .text { min-width: 0; flex: 1; display: flex; flex-direction: column; font-size: var(--fs-small); }
  .tools { display: flex; opacity: 0; transition: opacity var(--dur-fast); }
  .row:hover .tools, .tools:focus-within { opacity: 1; }
  .tool { width: var(--size-control); height: var(--size-control); border-radius: var(--r-pill); display: grid; place-items: center; color: var(--c-ink); }
  .tool:hover { background: var(--c-hover); }
  .foot { display: flex; align-items: center; gap: var(--sp-8); }
  .flex { flex: 1; }
  .msg { margin: var(--sp-8) 0 0; color: var(--c-done); font-size: var(--fs-small); }
  .msg.error { color: var(--c-error); }
</style>
