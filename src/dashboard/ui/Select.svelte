<!-- Native select in the design's field shape, with its chevron. -->
<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";

  let {
    value,
    options,
    label,
    disabled = false,
    width = "",
    onchange,
  }: {
    value: string;
    options: { v: string; label: string }[];
    label: string;
    disabled?: boolean;
    /** A width token, e.g. "--size-select-sm". */
    width?: string;
    onchange: (v: string) => void;
  } = $props();
</script>

<span class="d-select">
  <select
    class="d-field"
    {value}
    {disabled}
    aria-label={label}
    style:min-width={width ? `var(${width})` : undefined}
    style:width={width ? `var(${width})` : undefined}
    onchange={(e) => onchange(e.currentTarget.value)}
  >
    {#each options as o (o.v)}
      <option value={o.v} selected={o.v === value}>{o.label}</option>
    {/each}
  </select>
  <span class="chev" aria-hidden="true"><Icon name="chevronDown" size={16} /></span>
</span>
