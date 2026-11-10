<script lang="ts">
  import Row, { type RowProps } from "./Row.svelte";
  import Seg from "./Seg.svelte";

  let {
    items,
    value,
    wide = false,
    onchange,
    ...row
  }: RowProps & { items: { id: string; label: string }[]; value: string; wide?: boolean; onchange: (v: string) => void } = $props();
</script>

<Row {...row}>
  <Seg {items} {value} {wide} label={row.label ?? ""} disabled={row.disabled} {onchange} />
</Row>
