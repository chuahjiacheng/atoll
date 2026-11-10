<script lang="ts">
  import Row, { type RowProps } from "./Row.svelte";
  import Switch from "./Switch.svelte";

  let {
    checked,
    onchange,
    swLabel,
    ...row
  }: RowProps & { checked: boolean; onchange: (v: boolean) => void; swLabel?: string } = $props();
</script>

<Row {...row}>
  <Switch {checked} label={swLabel ?? row.label ?? ""} disabled={row.disabled} {onchange} />
</Row>
