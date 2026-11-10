<!--
  The island's right-click menu (opened with openMenu from menu.svelte.ts).
  Kept inside the expanded panel, since clicks outside the island's shape
  fall through to the desktop. Esc, a click elsewhere or leaving the island
  closes it; ↑/↓ + Enter work too.
-->
<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { closeMenu, menu, type MenuItem } from "./menu.svelte";

  let el = $state<HTMLDivElement>();
  let pos = $state({ x: 0, y: 0 });

  $effect(() => {
    if (!menu.open) return;
    // Measure, then keep the menu inside the panel (or the window).
    pos = { x: menu.x, y: menu.y };
    tick().then(() => {
      if (!el) return;
      const bounds = document.querySelector(".panel")?.getBoundingClientRect() ?? new DOMRect(0, 0, innerWidth, innerHeight);
      const r = el.getBoundingClientRect();
      const pad = 8;
      let x = menu.x;
      let y = menu.y;
      if (x + r.width > bounds.right - pad) x = Math.max(bounds.left + pad, x - r.width);
      if (y + r.height > bounds.bottom - pad) y = Math.max(bounds.top + pad, bounds.bottom - pad - r.height);
      pos = { x, y };
      el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
    });
    const away = (e: PointerEvent) => {
      if (!el?.contains(e.target as Node)) closeMenu();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeMenu();
      }
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("wheel", closeMenu, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("wheel", closeMenu);
    };
  });

  const items = $derived(
    // Drop empty entries and stray / doubled dividers.
    menu.items
      .filter((i) => i !== false && i !== undefined)
      .filter((i, n, all) => i !== null || (n > 0 && n < all.length - 1 && all[n - 1] !== null)) as (MenuItem | null)[],
  );

  async function choose(i: MenuItem) {
    closeMenu();
    await i.run();
  }

  function nav(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const all = [...(el?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    all[(at + (e.key === "ArrowDown" ? 1 : all.length - 1)) % all.length]?.focus();
  }
</script>

{#if menu.open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="menu" role="menu" tabindex="-1" bind:this={el} style="left: {pos.x}px; top: {pos.y}px" onkeydown={nav} oncontextmenu={(e) => e.preventDefault()}>
    {#each items as i, n (n)}
      {#if i === null}
        <div class="sep" role="separator"></div>
      {:else}
        <button role="menuitem" class:danger={i.danger} disabled={i.disabled} onclick={() => choose(i)}>
          <span class="ic">{#if i.icon}<Icon name={i.icon} size={16} />{/if}</span>
          <span class="label">{i.label}</span>
          {#if i.hint}<span class="hint">{i.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 196px;
    max-width: 280px;
    padding: var(--sp-4);
    border-radius: var(--r-control);
    background: var(--c-card);
    color: var(--c-ink);
    box-shadow: var(--shadow-toast);
    display: flex;
    flex-direction: column;
    font-size: var(--fs-small);
    outline: none;
  }
  button {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    min-height: 32px;
    padding: 0 var(--sp-8);
    border-radius: calc(var(--r-control) - var(--sp-4));
    text-align: left;
    color: inherit;
  }
  button:hover:not(:disabled),
  button:focus-visible { background: var(--c-hover); outline: none; }
  button:disabled { opacity: 0.45; }
  .danger { color: var(--c-error); }
  .ic { width: 16px; display: grid; place-items: center; color: var(--c-muted); flex: none; }
  .danger .ic { color: inherit; }
  .label { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .hint { color: var(--c-muted); font-size: var(--fs-label); }
  .sep { height: 1px; margin: var(--sp-4) var(--sp-8); background: var(--c-hairline); }
</style>
