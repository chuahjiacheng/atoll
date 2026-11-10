<!--
  Slider + number box + unit (design: slider row). The box accepts typing;
  out-of-range text shows an error and is not saved.
-->
<script lang="ts">
  import { Slider } from "glisse";
  import { react } from "../../lib/glass/bridge";
  import Row from "./Row.svelte";

  let {
    label,
    help = "",
    value,
    min,
    max,
    step = 1,
    unit,
    unitWord = unit,
    disabled = false,
    onchange,
  }: {
    label: string;
    help?: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    unit: string;
    /** Spoken unit for the number box ("pixels"). */
    unitWord?: string;
    disabled?: boolean;
    onchange: (v: number) => void;
  } = $props();

  /** Text being typed in the number box, while it differs from the saved value. */
  let raw = $state<string | null>(null);
  const valid = (t: string) => t.trim() !== "" && Number.isFinite(Number(t)) && Number(t) >= min && Number(t) <= max;
  const bad = $derived(raw !== null && !valid(raw));

  function typed(t: string) {
    raw = t;
    if (valid(t)) onchange(Number(t));
  }
</script>

<Row {label} {help} {disabled} note={bad ? `Enter a value from ${min} to ${max}.` : ""} noteIcon="alert" noteTone="err">
  <span
    class="host"
    use:react={{
      component: Slider,
      props: {
        value,
        min,
        max,
        step,
        disabled,
        "aria-label": label,
        onValueChange: (v: number) => {
          raw = null;
          onchange(v);
        },
      },
    }}
  ></span>
  <input
    class="d-field"
    type="number"
    {min}
    {max}
    {step}
    value={raw ?? String(value)}
    {disabled}
    aria-label="{label} in {unitWord}"
    aria-invalid={bad}
    oninput={(e) => typed(e.currentTarget.value)}
    onblur={() => raw !== null && valid(raw) && (raw = null)}
  />
  <span class="unit" class:dis={disabled}>{unit}</span>
</Row>

<style>
  .host {
    display: contents;
  }
  .unit {
    width: var(--size-unit);
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .unit.dis {
    color: var(--c-dis);
  }
</style>
