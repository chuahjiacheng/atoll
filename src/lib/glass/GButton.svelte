<!-- glisse Liquid Glass button. `variant="filled"` is the one primary action. -->
<script lang="ts">
  import { Button } from "glisse";
  import { icon as ricon, react } from "./bridge";
  import type { IconName } from "../ui/Icon.svelte";

  let {
    label = "",
    icon,
    variant = "glass",
    ariaLabel,
    title,
    disabled = false,
    destructive = false,
    danger = false,
    grow = false,
    onclick,
    type = "button",
    iconSize = 20,
  }: {
    label?: string;
    icon?: IconName;
    variant?: "glass" | "filled";
    ariaLabel?: string;
    title?: string;
    disabled?: boolean;
    destructive?: boolean;
    /** Filled in the error colour: confirming something that can't be undone. */
    danger?: boolean;
    /** Stretch to fill its grid/flex cell. */
    grow?: boolean;
    onclick?: (e: MouseEvent) => void;
    type?: "button" | "submit";
    iconSize?: number;
  } = $props();
</script>

<span
  class="g"
  class:grow
  use:react={{
    component: Button,
    props: {
      variant: danger ? "filled" : variant,
      tint: danger ? "var(--c-err)" : undefined,
      icon: icon ? ricon(icon, iconSize) : undefined,
      children: label || undefined,
      "aria-label": ariaLabel,
      title,
      disabled,
      destructive,
      type,
      className: [grow && "atoll-grow", danger && "atoll-danger"].filter(Boolean).join(" ") || undefined,
      onClick: (e: MouseEvent) => onclick?.(e),
    },
  }}
></span>

<style>
  .g { display: contents; }
</style>
