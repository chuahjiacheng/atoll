<!--
  Whiteboard tab: a freeform canvas of text, image and file cards.
  - Click a card: copy it (and paste into the app you were using, if on). Shift+click: copy only.
  - Drag a card: move it. Drag it past the canvas edge: it leaves the island as a
    real file you can drop into any app (text cards become a .txt file).
  - Double-click empty space or "+ Text": new text card. Double-click a text card: edit.
  - Drag empty space or scroll: pan. Drop files, images or text from any app onto the canvas: add them.
  - Right-click a card or empty space for Paste / Copy / send to another tab.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { boardApi, extOf, imageUrl, onBoard, onDragOutEnded, toast, type Card } from "./tools.svelte";
  import GButton from "./glass/GButton.svelte";
  import Icon from "./ui/Icon.svelte";
  import { openMenu } from "./ui/menu.svelte";

  let { clickPaste, onDone }: { clickPaste: boolean; onDone: () => void } = $props();

  let cards = $state<Card[]>([]);
  let pan = $state({ x: 0, y: 0 });
  let editing = $state<string | null>(null);
  let draft = $state("");
  let canvas = $state<HTMLDivElement>();
  let draggingOut = $state(false);
  const msg = toast();

  type Gesture =
    | { kind: "move"; id: string; sx: number; sy: number; ox: number; oy: number; moved: boolean; shift: boolean }
    | { kind: "resize"; id: string; sx: number; ow: number }
    | { kind: "pan"; sx: number; sy: number; ox: number; oy: number };
  let gesture: Gesture | null = null;

  onMount(() => {
    boardApi.get().then((b) => (cards = b.cards));
    try {
      const saved = JSON.parse(localStorage.getItem("crest.boardPan") ?? "null");
      if (saved && typeof saved.x === "number") pan = saved;
    } catch {}
    const un = [onBoard((b) => (cards = b.cards)), onDragOutEnded(() => (draggingOut = false))];
    return () => un.forEach((p) => p.then((f) => f()));
  });

  function savePan() {
    try {
      localStorage.setItem("crest.boardPan", JSON.stringify(pan));
    } catch {}
  }

  /** Board coordinates for a point in the window, or null when outside the canvas. */
  export function boardPoint(clientX: number, clientY: number): { x: number; y: number } | null {
    if (!canvas) return null;
    const r = canvas.getBoundingClientRect();
    if (clientX < r.left || clientX > r.right || clientY < r.top || clientY > r.bottom) return null;
    return { x: Math.round(clientX - r.left - pan.x), y: Math.round(clientY - r.top - pan.y) };
  }

  /** True while a card is being dragged out (so the island ignores its own drop). */
  export function isDraggingOut() {
    return draggingOut;
  }

  function center() {
    const r = canvas?.getBoundingClientRect();
    return { x: Math.round((r?.width ?? 400) / 2 - 90 - pan.x), y: Math.round((r?.height ?? 300) / 2 - 30 - pan.y) };
  }

  async function newText(at = center()) {
    const c = await boardApi.addText("", at.x, at.y);
    if (!cards.some((k) => k.id === c.id)) cards = [...cards, c];
    startEdit(c);
  }

  async function startEdit(c: Card) {
    editing = c.id;
    draft = c.text ?? "";
    await tick();
    canvas?.querySelector<HTMLTextAreaElement>(`[data-edit="${c.id}"]`)?.focus();
  }

  function finishEdit(c: Card) {
    if (editing !== c.id) return;
    editing = null;
    if (!draft.trim()) boardApi.remove(c.id);
    else if (draft !== c.text) boardApi.update(c.id, { text: draft });
  }

  async function use(c: Card, shift: boolean) {
    const paste = clickPaste && !shift;
    try {
      await boardApi.use(c.id, paste);
      if (paste) onDone();
      else msg.show("Copied");
    } catch (e) {
      msg.show(String(e), true);
    }
  }

  function cardDown(e: PointerEvent, c: Card) {
    if (e.button !== 0 || editing === c.id || (e.target as HTMLElement).closest("[data-noclick]")) return;
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = { kind: "move", id: c.id, sx: e.clientX, sy: e.clientY, ox: c.x, oy: c.y, moved: false, shift: e.shiftKey };
  }

  function resizeDown(e: PointerEvent, c: Card) {
    if (e.button !== 0) return;
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = { kind: "resize", id: c.id, sx: e.clientX, ow: c.w };
  }

  function canvasDown(e: PointerEvent) {
    if (e.button !== 0 || e.target !== canvas) return;
    canvas!.setPointerCapture(e.pointerId);
    gesture = { kind: "pan", sx: e.clientX, sy: e.clientY, ox: pan.x, oy: pan.y };
  }

  function move(e: PointerEvent) {
    const g = gesture;
    if (!g) return;
    if (g.kind === "pan") {
      pan = { x: g.ox + e.clientX - g.sx, y: g.oy + e.clientY - g.sy };
      return;
    }
    const c = cards.find((k) => k.id === g.id);
    if (!c) return;
    if (g.kind === "resize") {
      c.w = Math.max(90, Math.min(520, g.ow + e.clientX - g.sx));
      return;
    }
    const dx = e.clientX - g.sx;
    const dy = e.clientY - g.sy;
    if (!g.moved && Math.hypot(dx, dy) < 4) return;
    g.moved = true;
    c.x = g.ox + dx;
    c.y = g.oy + dy;
    // Past the canvas edge: hand the card to Windows as a real drag.
    if (!boardPoint(e.clientX, e.clientY)) {
      c.x = g.ox;
      c.y = g.oy;
      gesture = null;
      (e.target as HTMLElement).releasePointerCapture?.(e.pointerId);
      draggingOut = true;
      boardApi.drag(c.id).catch((err) => {
        draggingOut = false;
        msg.show(String(err), true);
      });
    }
  }

  function up() {
    const g = gesture;
    gesture = null;
    if (!g) return;
    if (g.kind === "pan") {
      savePan();
      return;
    }
    const c = cards.find((k) => k.id === g.id);
    if (!c) return;
    if (g.kind === "resize") boardApi.update(c.id, { w: c.w });
    else if (g.moved) boardApi.update(c.id, { x: Math.round(c.x), y: Math.round(c.y), raise: true });
    else use(c, g.shift);
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    pan = e.shiftKey ? { x: pan.x - e.deltaY, y: pan.y } : { x: pan.x - e.deltaX, y: pan.y - e.deltaY };
    savePan();
  }

  async function paste() {
    try {
      await boardApi.addClipboard();
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

  async function act(c: Card, paste: boolean) {
    try {
      await boardApi.use(c.id, paste);
      if (paste) onDone();
      else msg.show("Copied");
    } catch (e) {
      msg.show(String(e), true);
    }
  }

  function cardMenu(e: MouseEvent, c: Card) {
    const ok = !c.missing;
    openMenu(e, [
      { label: "Paste", icon: "send", hint: clickPaste ? "Click" : undefined, disabled: !ok, run: () => act(c, true) },
      { label: "Copy", icon: "clipboard", hint: clickPaste ? "Shift+click" : "Click", disabled: !ok, run: () => act(c, false) },
      c.kind === "text" && { label: "Edit", icon: "edit", hint: "Double-click", run: () => startEdit(c) },
      c.kind === "file" && { label: "Open", icon: "external", disabled: !ok, run: () => run(boardApi.open(c.id)) },
      c.kind !== "text" && { label: "Show in folder", icon: "folder", disabled: !ok, run: () => run(boardApi.reveal(c.id)) },
      null,
      { label: "Add to clipboard history", icon: "clipboard", disabled: !ok, run: () => run(boardApi.send(c.id, "clipboard"), "Added to clipboard history") },
      { label: c.kind === "file" ? "Keep in file tray" : "Save to file tray", icon: "folder", disabled: !ok, run: () => run(boardApi.send(c.id, "fileTray"), "Added to the file tray") },
      null,
      { label: "Duplicate", icon: "plus", run: () => run(boardApi.duplicate(c.id)) },
      { label: "Bring to front", icon: "chevronUp", run: () => boardApi.update(c.id, { raise: true }) },
      { label: "Remove", icon: "x", danger: true, run: () => boardApi.remove(c.id) },
    ]);
  }

  function canvasMenu(e: MouseEvent) {
    if (e.target !== canvas) return;
    const at = boardPoint(e.clientX, e.clientY) ?? center();
    openMenu(e, [
      { label: "Paste here", icon: "clipboard", run: () => run(boardApi.addClipboard(at.x, at.y)) },
      { label: "New text here", icon: "text", run: () => newText(at) },
      { label: "Add files…", icon: "upload", run: () => run(boardApi.pickFiles()) },
      (pan.x !== 0 || pan.y !== 0) && null,
      (pan.x !== 0 || pan.y !== 0) && { label: "Recentre", icon: "focus", run: () => ((pan = { x: 0, y: 0 }), savePan()) },
      cards.length > 0 && null,
      cards.length > 0 && { label: "Clear whiteboard", icon: "trash", danger: true, run: confirmClear },
    ]);
  }

  async function confirmClear() {
    if (confirm("Remove every card from the whiteboard? Linked files themselves are not touched.")) await boardApi.clear();
  }
</script>

<div class="board">
  <div class="tools">
    <GButton icon="plus" iconSize={16} label="Text" onclick={() => newText()} />
    <GButton icon="clipboard" iconSize={16} label="Paste" title="Add what's on the clipboard" onclick={paste} />
    <GButton icon="upload" iconSize={16} label="Add files" onclick={() => boardApi.pickFiles().catch((e) => msg.show(String(e), true))} />
    <span class="flex"></span>
    {#if pan.x !== 0 || pan.y !== 0}
      <GButton icon="focus" iconSize={16} label="Recentre" onclick={() => ((pan = { x: 0, y: 0 }), savePan())} />
    {/if}
    {#if cards.length}
      <GButton icon="trash" iconSize={16} label="Clear" onclick={confirmClear} />
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="canvas"
    bind:this={canvas}
    onpointerdown={canvasDown}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
    ondblclick={(e) => e.target === canvas && newText(boardPoint(e.clientX, e.clientY) ?? center())}
    oncontextmenu={canvasMenu}
    onwheel={wheel}
    style="background-position: {pan.x}px {pan.y}px"
  >
    {#if !cards.length}
      <p class="hint">
        Drop files here, press <strong>Paste</strong>, or double-click to write.<br />
        Click a card to {clickPaste ? "paste it where you were typing" : "copy it"}; drag it out of the island to drop it into another app.
      </p>
    {/if}
    {#each cards as c (c.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="bcard {c.kind}"
        class:missing={c.missing}
        class:editing={editing === c.id}
        style="left: {c.x + pan.x}px; top: {c.y + pan.y}px; width: {c.w}px"
        onpointerdown={(e) => cardDown(e, c)}
        ondblclick={() => c.kind === "text" && startEdit(c)}
        oncontextmenu={(e) => editing !== c.id && cardMenu(e, c)}
        title={c.kind === "file" ? c.path : clickPaste ? "Click to paste · Shift+click to copy · drag out to drop elsewhere · right-click for more" : "Click to copy · drag out to drop elsewhere · right-click for more"}
      >
        {#if c.kind === "text"}
          {#if editing === c.id}
            <textarea
              data-edit={c.id}
              data-noclick
              bind:value={draft}
              onblur={() => finishEdit(c)}
              onkeydown={(e) => {
                if (e.key === "Escape" || (e.key === "Enter" && e.ctrlKey)) (e.currentTarget as HTMLTextAreaElement).blur();
              }}
              placeholder="Write something…"
            ></textarea>
          {:else}
            <div class="text">{c.text}</div>
          {/if}
        {:else if c.kind === "image"}
          <img src={imageUrl(c.thumb)} alt={c.name ?? "Image"} draggable="false" />
        {:else}
          <div class="file">
            <span class="ext">{extOf(c.name ?? "") || "FILE"}</span>
            <span class="name">{c.name}</span>
            {#if c.missing}<small>missing</small>{/if}
          </div>
        {/if}
        <div class="acts" data-noclick>
          {#if c.kind === "file" && !c.missing}
            <button title="Open" aria-label="Open" onclick={() => boardApi.open(c.id).catch((e) => msg.show(String(e), true))}>
              <Icon name="external" size={12} />
            </button>
          {/if}
          <button title="Remove from board" aria-label="Remove" onclick={() => boardApi.remove(c.id)}>
            <Icon name="x" size={12} />
          </button>
        </div>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="grip" data-noclick onpointerdown={(e) => resizeDown(e, c)} title="Resize"></div>
      </div>
    {/each}
  </div>
  {#if msg.text}<p class="msg" class:error={msg.error}>{msg.text}</p>{/if}
</div>

<style>
  .board { display: flex; flex-direction: column; gap: var(--sp-8); flex: 1; min-height: 0; }
  .tools { display: flex; gap: var(--sp-8); align-items: center; }
  .flex { flex: 1; }
  .canvas {
    position: relative;
    overflow: hidden;
    flex: 1;
    min-height: 0;
    border-radius: var(--r-card);
    border: 2px dashed var(--c-dash);
    background-color: var(--c-surface-plate);
    background-image: radial-gradient(var(--c-hairline) 1px, transparent 1px);
    background-size: var(--sp-16) var(--sp-16);
    cursor: grab;
    touch-action: none;
  }
  .canvas:active { cursor: grabbing; }
  .hint {
    position: absolute;
    inset: 0;
    margin: auto;
    height: fit-content;
    text-align: center;
    color: var(--c-muted);
    font-size: var(--fs-small);
    line-height: var(--lh-loose);
    padding: 0 var(--sp-32);
    pointer-events: none;
  }
  .bcard {
    position: absolute;
    background: var(--c-card);
    color: var(--c-ink);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-preview), inset 0 0 0 1px var(--c-hairline);
    cursor: pointer;
    box-sizing: border-box;
    font-size: var(--fs-small);
  }
  .bcard:hover { box-shadow: var(--shadow-preview), inset 0 0 0 1px var(--c-mist); }
  .bcard.editing { cursor: text; box-shadow: var(--shadow-preview), inset 0 0 0 var(--focus-width) var(--c-accent); }
  .text { padding: var(--sp-12) var(--sp-16); white-space: pre-wrap; word-break: break-word; max-height: var(--size-card-text); overflow: hidden; line-height: var(--lh); }
  textarea {
    display: block;
    width: 100%;
    min-height: var(--size-card-edit);
    box-sizing: border-box;
    resize: vertical;
    border: 0;
    background: transparent;
    color: var(--c-ink);
    font: inherit;
    padding: var(--sp-12) var(--sp-16);
    outline: none;
  }
  .image img { display: block; width: 100%; border-radius: var(--r-card); pointer-events: none; }
  .file { display: flex; align-items: center; gap: var(--sp-8); padding: var(--sp-12) var(--sp-16); min-width: 0; }
  .ext {
    flex: none;
    font-size: var(--fs-label);
    font-weight: var(--fw-strong);
    padding: var(--sp-2) var(--sp-6);
    border-radius: var(--r-keycap);
    background: var(--c-button);
    color: var(--c-accent-text);
  }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .missing { opacity: 0.55; }
  .missing .name { text-decoration: line-through; }
  .file small { color: var(--c-error); flex: none; }
  .acts { position: absolute; top: calc(var(--sp-8) * -1); right: calc(var(--sp-8) * -1); display: none; gap: var(--sp-4); }
  .bcard:hover .acts { display: flex; }
  .acts button {
    width: var(--sp-24);
    height: var(--sp-24);
    border-radius: var(--r-pill);
    background: var(--c-button);
    color: var(--c-ink);
    display: grid;
    place-items: center;
    box-shadow: inset 0 0 0 1px var(--c-hairline);
  }
  .acts button:hover { background-image: linear-gradient(var(--c-hover), var(--c-hover)); }
  .grip { position: absolute; right: 0; bottom: 0; width: var(--sp-12); height: var(--sp-12); cursor: nwse-resize; display: none; }
  .grip::after {
    content: "";
    position: absolute;
    right: var(--sp-4);
    bottom: var(--sp-4);
    width: var(--sp-6);
    height: var(--sp-6);
    border-right: var(--sp-2) solid var(--c-muted);
    border-bottom: var(--sp-2) solid var(--c-muted);
  }
  .bcard:hover .grip { display: block; }
  .msg { margin: 0; color: var(--c-done); font-size: var(--fs-small); }
  .msg.error { color: var(--c-error); }
</style>
