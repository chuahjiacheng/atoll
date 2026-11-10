<!-- "Task finished" peek (design: Island → finished). Dismiss, or open the Agent tab. -->
<script lang="ts">
  import type { Session } from "../api";
  import GButton from "../glass/GButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import Pip, { type Tone } from "../ui/Pip.svelte";

  let {
    session,
    tone,
    onDismiss,
    onView,
  }: { session: Session; tone: Tone; onDismiss: () => void; onView: () => void } = $props();
</script>

<div class="peek" role="dialog" aria-label="Task finished in {session.project}">
  <div class="top">
    <Pip mood="happy" {tone} size={40} />
    <div class="body">
      <div class="kicker"><Icon name="check" size={16} /><span>Done</span></div>
      <div class="title ellipsis">{session.project ? `${session.project} finished` : "Task finished"}</div>
      <div class="detail">{session.message || session.prompt || "Claude Code finished its turn."}</div>
    </div>
  </div>
  <div class="actions">
    <GButton label="Dismiss" grow onclick={onDismiss} />
    <GButton label="View answer" variant="filled" grow onclick={onView} />
  </div>
</div>

<style>
  .peek { padding: var(--sp-16); display: flex; flex-direction: column; gap: var(--sp-16); }
  .top { display: flex; gap: var(--sp-16); align-items: flex-start; }
  .body { display: flex; flex-direction: column; gap: var(--sp-8); min-width: 0; flex: 1; }
  .kicker { display: flex; align-items: center; gap: var(--sp-4); font-size: var(--fs-label); font-weight: var(--fw-strong); color: var(--c-done); }
  .title { font-size: var(--fs-title); font-weight: var(--fw-strong); }
  .detail {
    font-size: var(--fs-small);
    color: var(--c-muted);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-8); }
</style>
