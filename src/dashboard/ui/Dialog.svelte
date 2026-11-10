<!--
  Confirm dialog on a scrim (design: dialog). Focus starts on Cancel, Tab
  stays inside, Esc cancels, and focus returns where it was.
-->
<script lang="ts">
  import { tick } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import GButton from "../../lib/glass/GButton.svelte";
  import { dur, ease, gsap, reduced } from "../../lib/motion";
  import { store } from "../store.svelte";

  let scrim = $state<HTMLElement>();
  let box = $state<HTMLElement>();
  let back: HTMLElement | null = null;

  const buttons = () => Array.from(box?.querySelectorAll<HTMLButtonElement>("button") ?? []);

  $effect(() => {
    if (!store.dialog) return;
    back = document.activeElement as HTMLElement | null;
    tick().then(() => {
      if (!scrim || !box) return;
      gsap.fromTo(scrim, { autoAlpha: 0 }, { autoAlpha: 1, duration: dur("fast"), ease: ease() });
      gsap.fromTo(
        box,
        { scale: reduced() ? 1 : parseFloat(getComputedStyle(box).getPropertyValue("--press-scale")) || 1 },
        { scale: 1, duration: dur("fast"), ease: ease(), clearProps: "transform" },
      );
      // glisse renders its buttons a frame later.
      requestAnimationFrame(() => buttons()[0]?.focus());
    });
  });

  function close() {
    store.dialog = null;
    back?.focus();
  }

  async function confirmIt() {
    const d = store.dialog;
    close();
    await d?.run();
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      const b = buttons();
      if (!b.length) return;
      const i = b.indexOf(document.activeElement as HTMLButtonElement);
      e.preventDefault();
      b[(i + (e.shiftKey ? -1 : 1) + b.length) % b.length].focus();
    }
  }
</script>

{#if store.dialog}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" bind:this={scrim} onkeydown={keydown}>
    <div class="box" role="dialog" aria-modal="true" aria-labelledby="dlg-title" aria-describedby="dlg-body" bind:this={box}>
      <div class="head">
        <span class="ic" aria-hidden="true"><Icon name="alert" size={20} /></span>
        <span id="dlg-title">{store.dialog.title}</span>
      </div>
      <p id="dlg-body">{store.dialog.body}</p>
      <div class="actions">
        <GButton label="Cancel" onclick={close} />
        <GButton label={store.dialog.ok} danger onclick={confirmIt} />
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 10;
    background: var(--c-scrim);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-24);
  }
  .box {
    box-sizing: border-box;
    width: var(--size-dialog);
    max-width: 100%;
    padding: var(--sp-24);
    border-radius: var(--r-card);
    background: var(--c-card);
    box-shadow: var(--shadow-window);
    display: flex;
    flex-direction: column;
    gap: var(--sp-12);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    font-size: var(--fs-title);
    font-weight: var(--fw-strong);
  }
  .ic {
    display: flex;
    color: var(--c-err);
  }
  p {
    margin: 0;
    color: var(--c-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-8);
    margin-top: var(--sp-12);
  }
</style>
