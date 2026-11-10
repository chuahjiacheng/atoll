<!--
  Modules: switch each module on or off and drag rows (or use the arrow keys
  on the handle) to set the tab order. GSAP Flip slides the rows into place.
-->
<script lang="ts">
  import { tick } from "svelte";
  import Icon, { type IconName } from "../../lib/ui/Icon.svelte";
  import { dur, ease, Flip } from "../../lib/motion";
  import type { ModuleKey, Settings } from "../../lib/settings";
  import { change } from "../store.svelte";
  import Section from "../ui/Section.svelte";
  import Row from "../ui/Row.svelte";
  import Switch from "../ui/Switch.svelte";

  let { s }: { s: Settings } = $props();

  const INFO: Record<string, { name: string; icon: IconName; desc: string }> = {
    claude: { name: "AI agent", icon: "agent", desc: "Live activity and permission requests from Claude Code." },
    spotify: { name: "Spotify", icon: "music", desc: "Track, controls, devices and lyrics." },
    calendar: { name: "Calendar", icon: "calendar", desc: "Your Google calendars: today and the days ahead." },
    clipboard: { name: "Clipboard", icon: "clipboard", desc: "Recent copies." },
    whiteboard: { name: "Whiteboard", icon: "whiteboard", desc: "Cards of text, images and files." },
    fileTray: { name: "File tray", icon: "folder", desc: "Files you drop on the island." },
    note: { name: "Quick note", icon: "note", desc: "One short note." },
    focus: { name: "Focus timer", icon: "focus", desc: "Focus and break sessions that hold back alerts until the break." },
    mascot: { name: "Mascot", icon: "smile", desc: "Pip in the island." },
  };
  /** Modules that get a tab in the expanded island (Home is always the first). */
  const TABBED = new Set(["claude", "spotify", "calendar", "clipboard", "whiteboard", "fileTray", "note", "focus"]);

  const order = $derived(s.moduleOrder.filter((k) => INFO[k]));
  const metas = $derived.by(() => {
    let tab = 1;
    return order.map((k) => {
      const on = s.modules[k as ModuleKey];
      if (!on) return "Hidden";
      if (TABBED.has(k)) return `Tab ${++tab}`;
      return k === "mascot" ? "In the header" : "Not built yet";
    });
  });

  let list = $state<HTMLElement>();
  let dragging = $state("");

  async function moveTo(id: string, to: number, focus = false) {
    const from = order.indexOf(id);
    if (from < 0 || to < 0 || to >= order.length || to === from || !list) return;
    const state = Flip.getState(list.querySelectorAll(".mod"));
    const next = order.slice();
    next.splice(to, 0, next.splice(from, 1)[0]);
    change("moduleOrder", next);
    await tick();
    Flip.from(state, { duration: dur("fast"), ease: ease() });
    if (focus) list.querySelector<HTMLElement>(`[data-id="${id}"] .grip`)?.focus();
  }

  function startDrag(e: PointerEvent, id: string) {
    e.preventDefault();
    dragging = id;
    const move = (ev: PointerEvent) => {
      const rows = Array.from(list?.querySelectorAll<HTMLElement>(".mod") ?? []);
      const over = rows.findIndex((r) => {
        const b = r.getBoundingClientRect();
        return ev.clientY >= b.top && ev.clientY < b.bottom;
      });
      if (over >= 0) moveTo(dragging, over);
    };
    const up = () => {
      dragging = "";
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  }

  function key(e: KeyboardEvent, id: string, i: number) {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    moveTo(id, e.key === "ArrowUp" ? i - 1 : i + 1, true);
  }
</script>

<Section
  title="Modules"
  desc="Drag a row by its handle to change the tab order. Home is always the first tab. Hidden modules keep their place."
>
  <div class="list" bind:this={list}>
    {#each order as id, i (id)}
      {@const m = INFO[id]}
      {@const on = s.modules[id as ModuleKey]}
      <div class="mod" class:dragging={dragging === id} data-id={id} data-flip-id={id}>
        <Row label={m.name} help={m.desc} icon={m.icon} meta={metas[i]}>
          {#snippet lead()}
            <button
              class="grip"
              aria-label="Reorder {m.name}, position {i + 1} of {order.length}. Use the up and down arrow keys, or drag."
              onpointerdown={(e) => startDrag(e, id)}
              onkeydown={(e) => key(e, id, i)}
            >
              <Icon name="grip" size={18} />
            </button>
          {/snippet}
          <Switch checked={on} label="Show {m.name}" onchange={(v) => change(`modules.${id}`, v)} />
        </Row>
      </div>
    {/each}
  </div>
</Section>

<style>
  .list {
    display: flex;
    flex-direction: column;
  }
  .mod + .mod {
    border-top: 1px solid var(--c-line);
  }
  .mod:first-child {
    border-radius: var(--r-card) var(--r-card) 0 0;
  }
  .mod:last-child {
    border-radius: 0 0 var(--r-card) var(--r-card);
  }
  .mod.dragging {
    position: relative;
    z-index: 1;
    background: var(--c-sel);
    box-shadow: inset 0 0 0 var(--focus-width) var(--c-accent);
  }
  .grip {
    width: var(--size-control);
    height: var(--size-control);
    flex: none;
    margin: 0 calc(-1 * var(--sp-12)) 0 calc(-1 * var(--sp-8));
    border-radius: var(--r-nav);
    color: var(--c-muted);
    cursor: grab;
    touch-action: none;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .grip:hover {
    background: var(--c-hover);
  }
  .dragging .grip {
    cursor: grabbing;
  }
</style>
