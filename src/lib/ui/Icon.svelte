<!--
  Atoll icon set, from the design (design/…/Icon.dc.html): 24-unit grid,
  1.75 stroke, round caps. A few transport icons are filled.
-->
<script lang="ts" module>
  export const ICONS = {
    home: "M4 11l8-7 8 7v9h-5v-6H9v6H4z",
    settings: "M4 7h9M17 7h3M4 17h3M11 17h9M15 5v4M9 15v4",
    power: "M12 3v8M7.05 6.05a7 7 0 1 0 9.9 0",
    chevronUp: "M6 15l6-6 6 6",
    chevronDown: "M6 9l6 6 6-6",
    chevronRight: "M9 6l6 6-6 6",
    chevronLeft: "M15 6l-6 6 6 6",
    agent: "M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9zM18.5 15.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z",
    music: "M9 18V6l10-2v12M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM19 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
    media: "M4 5h16v14H4zM10 9l5 3-5 3z",
    clipboard: "M9 3.5h6v3H9zM9 5H6v15h12V5h-3",
    whiteboard: "M3 4h18v12H3zM8 20l4-4 4 4",
    folder: "M3 6h6l2 2h10v11H3z",
    note: "M5 4h14v10l-6 6H5zM13 20v-6h6",
    focus: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z",
    check: "M5 12.5l4.5 4.5L19 7.5",
    x: "M6 6l12 12M18 6L6 18",
    alert: "M12 4l9 16H3zM12 10v4M12 17v.5",
    busy: "M12 3a9 9 0 1 1-9 9",
    file: "M6 3h8l4 4v14H6zM14 3v4h4",
    terminal: "M4 5h16v14H4zM7.5 10l3 2-3 2M12.5 15h4",
    edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
    read: "M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12zM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z",
    prev: "M6 5h2v14H6zM20 5v14L9 12z",
    next: "M16 5h2v14h-2zM4 5v14l11-7z",
    play: "M7 4v16l13-8z",
    pause: "M6 4h4v16H6zM14 4h4v16h-4z",
    pauseLine: "M9 6v12M15 6v12",
    phone: "M7 2.5h10v19H7zM11 18h2",
    laptop: "M5 5h14v10H5zM2 19h20",
    speaker: "M6 3h12v18H6zM12 7v.5M12 11a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
    image: "M3 5h18v14H3zM3 16l5-5 4 4 3-3 6 6M15.5 8.5v.5",
    text: "M4 6h16M4 10h16M4 14h10M4 18h12",
    trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
    reset: "M4 4v6h6M5.1 15a8 8 0 1 0 1.6-8.6L4 10",
    coffee: "M4 9h13v5a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5zM17 10h1.5a2.5 2.5 0 0 1 0 5H17M8 3v3M12 3v3",
    plus: "M12 5v14M5 12h14",
    send: "M4 12l16-8-6 16-2-7z",
    clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2",
    calendar: "M4 6h16v14H4zM4 10h16M8 3v4M16 3v4M8 14h.5M12 14h.5M16 14h.5M8 17h.5M12 17h.5",
    video: "M3 7h12v10H3zM15 10.5l6-3.5v10l-6-3.5",
    pinmap: "M12 21s-6-5.5-6-11a6 6 0 0 1 12 0c0 5.5-6 11-6 11zM12 8a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
    upload: "M12 16V4M7 9l5-5 5 5M4 16v4h16v-4",
    download: "M12 4v12M7 11l5 5 5-5M4 16v4h16v-4",
    cursor: "M5 3l14 7-6 2-2 6z",
    eq: "M5 20V12M10 20V5M15 20v-9M20 20v-5",
    monitor: "M3 4h18v12H3zM9 20h6M12 16v4",
    dot: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
    min: "M5 12h14",
    max: "M6 6h12v12H6z",
    grip: "M9 6v.5M15 6v.5M9 12v.5M15 12v.5M9 18v.5M15 18v.5",
    grid: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
    palette: "M12 3a9 9 0 1 0 0 18c1.5 0 2-1 2-2s-1-2 0-3 2 0 3 0a4 4 0 0 0 4-4c0-5-4-9-9-9zM7.5 11.5v.5M10 7.5v.5M15 7.5v.5",
    bell: "M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 20.5a2 2 0 0 0 4 0",
    keyboard: "M3 6h18v12H3zM7 10v.5M11 10v.5M15 10v.5M7 14h10",
    smile: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8.5 14.5q3.5 3 7 0M9 9.5v.5M15 9.5v.5",
    shield: "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z",
    info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v6M12 7.5v.5",
    external: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
    lock: "M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3",
    volume: "M4 9h4l5-4v14l-5-4H4zM16.5 8.5a5 5 0 0 1 0 7",
    volumeOff: "M4 9h4l5-4v14l-5-4H4zM16.5 9.5l5 5M21.5 9.5l-5 5",
    search: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM15.5 15.5L20 20",
    pin: "M9 3h6l-1 6 4 4H6l4-4zM12 13v8",
    lyrics: "M4 6h10M4 10h10M4 14h7M17 18V8l3-1M17 18a2 2 0 1 1-4 0 2 2 0 0 1 4 0z",
    // ---- ask-assistant: begin ----
    sparkle: "M11 4l1.8 5.2L18 11l-5.2 1.8L11 18l-1.8-5.2L4 11l5.2-1.8zM18.5 3v4M16.5 5h4",
    stop: "M7 7h10v10H7z",
    copy: "M9 9h11v11H9zM5 15H4V4h11v1",
    // ---- ask-assistant: end ----
  } as const;
  export type IconName = keyof typeof ICONS;
  const FILLED = new Set<IconName>(["prev", "next", "play", "pause", "cursor", "dot", "stop"]);
</script>

<script lang="ts">
  let { name, size = 24 }: { name: IconName; size?: number } = $props();
  const filled = $derived(FILLED.has(name));
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke={filled ? "none" : "currentColor"}
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class="icon"><path d={ICONS[name] ?? ICONS.dot} /></svg>

<style>
  .icon { display: block; flex: none; }
</style>
