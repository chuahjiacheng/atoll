<!--
  Atoll Settings (design: Settings.dc.html). Custom title bar, sidebar with a
  GSAP-moved indicator, and the page area: Lenis smooth scrolling (driven by
  GSAP's clock only while it glides), GSAP page transitions, a toast and a
  confirm dialog.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Lenis from "lenis";
  import Icon, { type IconName } from "../lib/ui/Icon.svelte";
  import Pip from "../lib/ui/Pip.svelte";
  import GButton from "../lib/glass/GButton.svelte";
  import { dur, ease, enter, gsap, reduced } from "../lib/motion";
  import { applyAppearance, px, watchSystem } from "../lib/theme";
  import type { ResetSection } from "../lib/settings";
  import { confirm, init, reset, store } from "./store.svelte";
  import Toast from "./ui/Toast.svelte";
  import Dialog from "./ui/Dialog.svelte";
  import GeneralPage from "./pages/GeneralPage.svelte";
  import AppearancePage from "./pages/AppearancePage.svelte";
  import ModulesPage from "./pages/ModulesPage.svelte";
  import AgentsPage from "./pages/AgentsPage.svelte";
  import SpotifyPage from "./pages/SpotifyPage.svelte";
  import CalendarPage from "./pages/CalendarPage.svelte";
  import AskPage from "./pages/AskPage.svelte"; // ask-assistant
  import ClipboardPage from "./pages/ClipboardPage.svelte";
  import FocusPage from "./pages/FocusPage.svelte";
  import AlertsPage from "./pages/AlertsPage.svelte";
  import HotkeysPage from "./pages/HotkeysPage.svelte";
  import MascotPage from "./pages/MascotPage.svelte";
  import PrivacyPage from "./pages/PrivacyPage.svelte";
  import AboutPage from "./pages/AboutPage.svelte";
  // ---- permission-intel: begin ----
  import RulesPage from "./pages/RulesPage.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  // ---- permission-intel: end ----

  type Page = { id: string; title: string; icon: IconName; desc: string; reset?: ResetSection };
  const PAGES: Page[] = [
    { id: "general", title: "General", icon: "settings", desc: "Startup, language and where the island sits on screen.", reset: "general" },
    { id: "appearance", title: "Appearance", icon: "palette", desc: "Size, shape, colours and motion of the island. Changes show in the preview straight away.", reset: "appearance" },
    { id: "modules", title: "Modules", icon: "grid", desc: "Turn modules on or off and set their tab order in the expanded island.", reset: "modules" },
    { id: "agents", title: "AI agents", icon: "agent", desc: "Connect Claude Code and choose what the island shows.", reset: "agents" },
    { id: "rules", title: "Permission rules", icon: "shield", desc: "Let Atoll notice, and if you say so answer, the permission requests you keep allowing." }, // permission-intel
    { id: "spotify", title: "Spotify", icon: "music", desc: "Show and control the Spotify app on this PC. One click, no sign-in.", reset: "spotify" },
    { id: "calendar", title: "Calendar", icon: "calendar", desc: "Connect your Google account and choose which calendars the island shows.", reset: "calendar" },
    { id: "ask", title: "Ask", icon: "sparkle", desc: "Your Gemini key, the model Ask uses, and what happens to the chat.", reset: "ask" }, // ask-assistant
    { id: "clipboard", title: "Clipboard and whiteboard", icon: "clipboard", desc: "Clipboard history, privacy and whiteboard options.", reset: "clipboard" },
    { id: "focus", title: "Focus timer", icon: "focus", desc: "Session lengths, breaks, sound, and what alerts do while you focus.", reset: "focus" },
    { id: "alerts", title: "Alerts and sounds", icon: "bell", desc: "Sounds and Windows notifications for each event.", reset: "alerts" },
    { id: "hotkeys", title: "Hotkeys", icon: "keyboard", desc: "Keyboard shortcuts that work from anywhere in Windows.", reset: "hotkeys" },
    { id: "mascot", title: "Mascot", icon: "smile", desc: "How Pip looks and reacts.", reset: "mascot" },
    { id: "privacy", title: "Privacy and data", icon: "shield", desc: "Where your data is kept and what leaves this PC." },
    { id: "about", title: "About", icon: "info", desc: "Version, logs and quitting the app." },
  ];
  const KEY = "atoll.settings.page";

  function firstPage(): string {
    try {
      const saved = localStorage.getItem(KEY);
      if (saved && PAGES.some((p) => p.id === saved)) return saved;
    } catch {}
    return "general";
  }

  let page = $state(firstPage());
  const cur = $derived(PAGES.find((p) => p.id === page)!);
  let loadError = $state("");
  let maximised = $state(false);
  const win = getCurrentWindow();

  let navEl = $state<HTMLElement>();
  let markEl = $state<HTMLElement>();
  let mainEl = $state<HTMLElement>();
  let pageEl = $state<HTMLElement>();
  let contentEl = $state<HTMLElement>();

  // ---------- theme: the window follows the island's appearance ----------
  let systemTick = $state(0);
  $effect(() => {
    systemTick;
    const s = store.settings;
    if (!s) return;
    applyAppearance(s.appearance);
    // The Low-Tide rule is for text over the desktop; this window is opaque.
    document.documentElement.dataset.lowTide = "false";
    if (lenis) lenis.options.smoothWheel = !reduced();
  });

  // ---------- sidebar indicator ----------
  function placeMark(animate: boolean) {
    const btn = navEl?.querySelector<HTMLElement>(`[data-page="${page}"]`);
    if (!btn || !markEl) return;
    const y = btn.offsetTop + (btn.offsetHeight - markEl.offsetHeight) / 2;
    if (!animate || reduced()) gsap.set(markEl, { y });
    else gsap.to(markEl, { y, duration: dur("fast"), ease: ease(), overwrite: true });
  }

  async function go(id: string) {
    if (id === page) return;
    lenis?.scrollTo(0, { immediate: true, force: true });
    if (mainEl) mainEl.scrollTop = 0;
    page = id;
    try {
      localStorage.setItem(KEY, id);
    } catch {}
    placeMark(true);
    await tick();
    if (contentEl) enter(contentEl);
  }

  function navKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const items = Array.from(navEl?.querySelectorAll<HTMLButtonElement>(".d-nav-item") ?? []);
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus();
  }

  function askReset() {
    const p = cur;
    if (!p.reset) return;
    confirm(
      `Reset ${p.title} to defaults?`,
      "Every setting on this page goes back to its default. Other pages are not changed.",
      "Reset page",
      () => reset(p.reset!, p.title),
    );
  }

  // ---------- smooth scrolling: Lenis on GSAP's clock, only while gliding ----------
  let lenis: Lenis | null = null;
  let still = 0;
  const raf = (time: number) => {
    if (!lenis) return;
    lenis.raf(time * 1000);
    // Stop the clock a couple of frames after the glide ends (idle costs nothing).
    still = lenis.isScrolling ? 0 : still + 1;
    if (still > 2) ticking(false);
  };
  let running = false;
  function ticking(on: boolean) {
    if (on === running) return;
    running = on;
    still = 0;
    if (on) gsap.ticker.add(raf);
    else gsap.ticker.remove(raf);
  }

  // ---- permission-intel: begin ----
  // The peek's "Make a rule…" opens this window on the rules page with a draft.
  let rulesPage = $state<{ draftArrived: () => void }>();
  async function toRules() {
    if (page === "rules") rulesPage?.draftArrived();
    else await go("rules");
  }
  onMount(() => {
    invoke<boolean>("rules_draft_waiting").then((waiting) => {
      if (waiting) toRules();
    }).catch(() => {});
    const un = listen<string>("settings:goto", (e) => e.payload === "rules" && toRules());
    return () => un.then((f) => f());
  });
  // ---- permission-intel: end ----

  // A "Connect …" button in the island opens this window on its page
  // (Spotify, Calendar, AI agents) rather than the page it was left on.
  const toPage = (id: string | null) => {
    if (id && id !== "rules" && PAGES.some((p) => p.id === id)) go(id);
  };
  onMount(() => {
    invoke<string | null>("dashboard_take_page").then(toPage).catch(() => {});
    const un = listen<string>("settings:goto", (e) => {
      if (e.payload === "rules") return;
      invoke("dashboard_take_page").catch(() => {});
      toPage(e.payload);
    });
    return () => un.then((f) => f());
  });

  onMount(() => {
    init().catch((e) => (loadError = String(e)));
    const unSystem = watchSystem(() => systemTick++);

    // Focus rings for keyboard use only.
    const root = document.documentElement;
    const byPointer = () => (root.dataset.input = "pointer");
    const byKey = (e: KeyboardEvent) => e.key === "Tab" && (root.dataset.input = "keyboard");
    window.addEventListener("pointerdown", byPointer, true);
    window.addEventListener("keydown", byKey, true);

    const unResize = win.onResized(async () => {
      maximised = await win.isMaximized();
      placeMark(false);
    });
    win.isMaximized().then((m) => (maximised = m));

    return () => {
      unSystem();
      window.removeEventListener("pointerdown", byPointer, true);
      window.removeEventListener("keydown", byKey, true);
      unResize.then((f) => f());
      ticking(false);
      lenis?.destroy();
    };
  });

  // Lenis starts once the page area exists (after settings load).
  $effect(() => {
    if (!mainEl || !pageEl || lenis) return;
    lenis = new Lenis({ wrapper: mainEl, content: pageEl, autoRaf: false, smoothWheel: !reduced() });
    mainEl.addEventListener("wheel", () => ticking(true), { capture: true, passive: true });
  });

  $effect(() => {
    if (store.settings && navEl && markEl) placeMark(false);
  });
</script>

<div class="app">
  <header class="title">
    <div class="name" data-tauri-drag-region>
      <span class="nopoint"><Pip mood="idle" tone={store.settings?.mascot.tone ?? "peri"} size={px("--size-icon-sm")} /></span>
      <span class="nopoint">Atoll Settings</span>
    </div>
    <button class="cap" aria-label="Minimise" onclick={() => win.minimize()}><Icon name="min" size={16} /></button>
    <button class="cap" aria-label={maximised ? "Restore" : "Maximise"} onclick={() => win.toggleMaximize()}><Icon name="max" size={14} /></button>
    <button class="cap close" aria-label="Close" onclick={() => win.close()}><Icon name="x" size={16} /></button>
  </header>

  <div class="body">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <nav class="nav" aria-label="Settings pages" bind:this={navEl} onkeydown={navKey}>
      <span class="mark" bind:this={markEl} aria-hidden="true"></span>
      {#each PAGES as p (p.id)}
        <button class="d-nav-item" data-page={p.id} aria-current={page === p.id ? "page" : undefined} onclick={() => go(p.id)}>
          <Icon name={p.icon} size={18} />
          <span class="ellipsis">{p.title}</span>
        </button>
      {/each}
    </nav>

    <main class="d-main" bind:this={mainEl}>
      <div class="page" bind:this={pageEl}>
        <div class="head">
          <div class="htext">
            <h1>{cur.title}</h1>
            <p>{cur.desc}</p>
          </div>
          {#if cur.reset}<GButton label="Reset to defaults" icon="reset" iconSize={16} onclick={askReset} />{/if}
        </div>

        {#if loadError}
          <p class="err" role="alert">Could not load settings: {loadError}</p>
        {:else if store.settings}
          {@const s = store.settings}
          <div class="content" bind:this={contentEl}>
            {#if page === "general"}<GeneralPage {s} />
            {:else if page === "appearance"}<AppearancePage {s} scroller={mainEl!} />
            {:else if page === "modules"}<ModulesPage {s} />
            {:else if page === "agents"}<AgentsPage {s} />
            {:else if page === "rules"}<RulesPage bind:this={rulesPage} /><!-- permission-intel -->
            {:else if page === "spotify"}<SpotifyPage {s} />
            {:else if page === "calendar"}<CalendarPage {s} />
            {:else if page === "ask"}<AskPage {s} /><!-- ask-assistant -->
            {:else if page === "clipboard"}<ClipboardPage {s} />
            {:else if page === "focus"}<FocusPage {s} />
            {:else if page === "alerts"}<AlertsPage {s} />
            {:else if page === "hotkeys"}<HotkeysPage {s} />
            {:else if page === "mascot"}<MascotPage {s} />
            {:else if page === "privacy"}<PrivacyPage {s} />
            {:else}<AboutPage {s} />{/if}
          </div>
        {/if}
      </div>
    </main>
  </div>

  <Toast />
  <Dialog />
</div>

<style>
  .app {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--c-win);
  }
  .title {
    height: var(--size-titlebar);
    flex: none;
    display: flex;
    align-items: center;
    background: var(--c-side);
  }
  .name {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    gap: var(--sp-8);
    padding: 0 var(--sp-12);
    font-size: var(--fs-label);
  }
  .nopoint {
    display: flex;
    pointer-events: none;
  }
  .cap {
    width: var(--size-caption);
    height: var(--size-titlebar);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--c-ink);
  }
  .cap:hover {
    background: var(--c-hover);
  }
  .cap.close:hover {
    background: var(--c-caption-close);
    color: var(--c-caption-close-ink);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .nav {
    position: relative;
    width: var(--size-sidebar);
    flex: none;
    box-sizing: border-box;
    padding: var(--sp-8);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    background: var(--c-side);
    overflow-y: auto;
  }
  .mark {
    position: absolute;
    top: 0;
    left: calc(var(--sp-8) + var(--sp-4));
    width: var(--size-nav-mark-w);
    height: var(--size-nav-mark-h);
    border-radius: var(--r-mark);
    background: var(--c-accent);
    z-index: 1;
  }
  .d-nav-item {
    position: relative;
    flex: none;
    box-sizing: border-box;
    height: var(--size-nav-item);
    padding: 0 var(--sp-12) 0 var(--sp-16);
    border-radius: var(--r-nav);
    font-size: var(--fs-body);
    text-align: left;
    display: flex;
    align-items: center;
    gap: var(--sp-12);
  }
  .d-nav-item:hover {
    background: var(--c-hover);
  }
  .d-nav-item[aria-current="page"] {
    background: var(--c-sel);
    font-weight: var(--fw-strong);
  }
  .d-main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    position: relative;
  }
  .page {
    box-sizing: border-box;
    max-width: var(--size-main-max);
    padding: var(--sp-24) var(--sp-32) var(--sp-48);
    display: flex;
    flex-direction: column;
    gap: var(--sp-24);
  }
  .content {
    display: flex;
    flex-direction: column;
    gap: var(--sp-24);
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-16);
  }
  .htext {
    flex: 1;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-size: var(--fs-headline);
    font-weight: var(--fw-strong);
    line-height: var(--lh-tight);
  }
  .htext p {
    margin: var(--sp-4) 0 0;
    font-size: var(--fs-small);
    color: var(--c-muted);
  }
  .err {
    color: var(--c-err);
  }
</style>
