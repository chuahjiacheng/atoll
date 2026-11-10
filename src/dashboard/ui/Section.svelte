<!-- A titled group of rows on one card. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let { title = "", desc = "", aria = "", children }: { title?: string; desc?: string; aria?: string; children: Snippet } = $props();
</script>

<section class="d-section" aria-label={aria || title || undefined}>
  {#if title}<h2>{title}</h2>{/if}
  {#if desc}<p>{desc}</p>{/if}
  <div class="d-card">{@render children()}</div>
</section>
