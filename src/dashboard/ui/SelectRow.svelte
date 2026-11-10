<script lang="ts">
  import type { Snippet } from "svelte";
  import Row, { type RowProps } from "./Row.svelte";
  import Select from "./Select.svelte";

  let {
    value,
    options,
    width = "",
    onchange,
    after,
    ...row
  }: RowProps & {
    value: string;
    options: { v: string; label: string }[];
    width?: string;
    onchange: (v: string) => void;
    /** Extra control after the select (e.g. a preview button). */
    after?: Snippet;
  } = $props();
</script>

<Row {...row}>
  <Select {value} {options} {width} label={row.label ?? ""} disabled={row.disabled} {onchange} />
  {@render after?.()}
</Row>
