<!-- Rules lab (dev only): the page inside the Settings window's content column, with its toast and dialog. -->
<script lang="ts">
  import RulesPage from "../pages/RulesPage.svelte";
  import { store } from "../store.svelte";
</script>

<div class="d-main lab">
  <div class="col">
    <h1>Permission rules</h1>
    <RulesPage />
  </div>
  {#if store.toast.text}<p class="toast" class:err={store.toast.error} role="status">{store.toast.text}</p>{/if}
  {#if store.dialog}
    <div class="dlg" role="dialog" aria-label={store.dialog.title}>
      <strong>{store.dialog.title}</strong>
      <p>{store.dialog.body}</p>
      <button onclick={() => (store.dialog = null)}>Cancel</button>
    </div>
  {/if}
</div>

<style>
  :global(body) { margin: 0; background: var(--c-win); color: var(--c-ink); font-family: var(--font); }
  .lab { padding: 32px 48px; min-height: 100vh; box-sizing: border-box; }
  .col { max-width: 760px; }
  h1 { font-size: var(--fs-headline); font-weight: var(--fw-strong); margin: 0 0 24px; }
  .toast { position: fixed; right: 24px; bottom: 24px; margin: 0; padding: 10px 16px; border-radius: 12px; background: var(--c-card); box-shadow: var(--shadow-toast); font-size: var(--fs-small); }
  .toast.err { color: var(--c-err); }
  .dlg { position: fixed; inset: 30% 30% auto; padding: 16px; background: var(--c-card); border-radius: 16px; box-shadow: var(--shadow-window); }
</style>
