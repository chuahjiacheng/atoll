<!-- "Saved" confirmation, bottom right (design: toast). Errors stay longer. -->
<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import { dur, ease, gsap, reduced } from "../../lib/motion";
  import { store } from "../store.svelte";

  let el = $state<HTMLElement>();
  // Keep the last words on screen while it fades out.
  let text = $state("");
  let error = $state(false);

  $effect(() => {
    const show = !!store.toast.text;
    store.toast.at;
    if (show) {
      text = store.toast.text;
      error = store.toast.error;
    }
    if (!el) return;
    gsap.to(el, {
      autoAlpha: show ? 1 : 0,
      y: show || reduced() ? 0 : px(),
      duration: dur(show ? "fast" : "collapse"),
      ease: ease(),
      overwrite: true,
    });
  });

  const px = () => parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--sp-8")) || 0;
</script>

<div class="wrap" role="status" aria-live="polite">
  <div class="toast" bind:this={el} data-error={error}>
    <span class="ic" aria-hidden="true"><Icon name={error ? "alert" : "check"} size={16} /></span>
    <span>{text}</span>
  </div>
</div>

<style>
  .wrap {
    position: absolute;
    right: var(--sp-24);
    bottom: var(--sp-24);
    z-index: 5;
    pointer-events: none;
  }
  .toast {
    visibility: hidden;
    opacity: 0;
    box-sizing: border-box;
    min-height: var(--size-toast);
    max-width: var(--size-dialog);
    padding: var(--sp-8) var(--sp-16) var(--sp-8) var(--sp-12);
    border-radius: calc(var(--size-toast) / 2);
    background: var(--c-card);
    box-shadow: var(--shadow-toast);
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    font-size: var(--fs-small);
    font-weight: var(--fw-strong);
  }
  .ic {
    display: flex;
    flex: none;
    color: var(--c-ok);
  }
  [data-error="true"] .ic {
    color: var(--c-err);
  }
</style>
