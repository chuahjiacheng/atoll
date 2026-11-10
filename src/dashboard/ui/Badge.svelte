<!-- Status badge (design: connection and "what leaves this PC" badges). -->
<script lang="ts" module>
  import type { IconName } from "../../lib/ui/Icon.svelte";

  export const BADGES = {
    connected: { word: "Connected", icon: "check", tone: "ok" },
    notfound: { word: "Not found", icon: "alert", tone: "warn" },
    error: { word: "Error", icon: "x", tone: "err" },
    connecting: { word: "Connecting", icon: "busy", tone: "ink" },
    off: { word: "Not connected", icon: "dot", tone: "muted" },
    when: { word: "When connected", icon: "info", tone: "muted" },
    whenOn: { word: "When turned on", icon: "info", tone: "muted" },
    local: { word: "Stays on this PC", icon: "lock", tone: "ok" },
  } satisfies Record<string, { word: string; icon: IconName; tone: string }>;

  export type BadgeKind = keyof typeof BADGES;
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";

  let { kind }: { kind: BadgeKind } = $props();
  const b = $derived(BADGES[kind]);
</script>

<span class="d-badge" data-tone={b.tone}><Icon name={b.icon} size={14} />{b.word}</span>
