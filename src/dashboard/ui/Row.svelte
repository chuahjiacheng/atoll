<!--
  One settings row (design: the row in Settings.dc.html): optional lead
  (drag handle), icon tile, label and help, meta text, the control on the
  right, content below, and a note (help, warning or error) at the bottom.
-->
<script lang="ts" module>
  import type { Snippet } from "svelte";
  import type { IconName } from "../../lib/ui/Icon.svelte";

  export type Tone = "muted" | "err" | "ok" | "ink";

  export interface RowProps {
    label?: string;
    help?: string;
    icon?: IconName;
    /** "big" = 16 px semibold (connection headers), "huge" = 20 px (About). */
    size?: "normal" | "big" | "huge";
    disabled?: boolean;
    meta?: string;
    note?: string;
    noteIcon?: IconName;
    noteTone?: Tone;
    /** Line the note up with the label when the row has an icon tile. */
    noteIndent?: boolean;
    /** false = no label line (the row is only its `below` content). */
    line?: boolean;
    lead?: Snippet;
    /** Rich label (e.g. with a counting number) instead of `label`. */
    title?: Snippet;
    children?: Snippet;
    below?: Snippet;
    labelId?: string;
  }
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";

  let {
    label = "",
    help = "",
    icon,
    size = "normal",
    disabled = false,
    meta = "",
    note = "",
    noteIcon = "info",
    noteTone = "muted",
    noteIndent = false,
    line = true,
    lead,
    title,
    children,
    below,
    labelId,
  }: RowProps = $props();
</script>

<div class="d-row" data-disabled={disabled}>
  {#if line}
    <div class="d-line">
      {@render lead?.()}
      {#if icon}<span class="d-tile" aria-hidden="true"><Icon name={icon} size={18} /></span>{/if}
      <div class="d-text">
        <span class="d-label" class:big={size === "big"} class:huge={size === "huge"} id={labelId}>{#if title}{@render title()}{:else}{label}{/if}</span>
        {#if help}<span class="d-help">{help}</span>{/if}
      </div>
      {#if meta}<span class="d-meta">{meta}</span>{/if}
      {#if children}<div class="d-controls">{@render children()}</div>{/if}
    </div>
  {/if}
  {@render below?.()}
  {#if note}
    <div class="d-note" class:indent={noteIndent} data-tone={noteTone} role={noteTone === "err" ? "alert" : undefined}>
      <span aria-hidden="true"><Icon name={noteIcon} size={16} /></span><span>{note}</span>
    </div>
  {/if}
</div>
