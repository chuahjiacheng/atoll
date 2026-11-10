<!-- glisse switch with the design's On / Off word in front of it. -->
<script lang="ts">
  import { Switch } from "glisse";
  import { react } from "../../lib/glass/bridge";

  let {
    checked,
    label,
    disabled = false,
    onchange,
  }: { checked: boolean; label: string; disabled?: boolean; onchange: (v: boolean) => void } = $props();
</script>

<span class="sw" class:dis={disabled}>
  <span class="word" aria-hidden="true">{checked ? "On" : "Off"}</span>
  <span
    class="host"
    use:react={{
      component: Switch,
      props: { checked, disabled, "aria-label": label, onCheckedChange: (v: boolean) => onchange(v) },
    }}
  ></span>
</span>

<style>
  .sw {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-12);
  }
  .word {
    min-width: var(--size-state-word);
    font-size: var(--fs-small);
    color: var(--c-muted);
    text-align: right;
  }
  .dis .word {
    color: var(--c-dis);
  }
  .host {
    display: contents;
  }
</style>
