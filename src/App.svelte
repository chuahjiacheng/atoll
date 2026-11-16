<!--
  The island window (design: Island). A floating pill at the top centre that
  morphs into a peek (permission request, finished task) or the expanded
  panel (Home + one tab per module). Appearance comes from the settings
  (lib/theme.ts) on top of the design tokens (styles/tokens.css).
-->
<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    api,
    onAgentFeed,
    onHover,
    onPermissions,
    onSessions,
    onUsage,
    type AgentFeed,
    type Permission,
    type Session,
    type Status,
    type Usage,
  } from "./lib/api";
  import { clearable, elapsed, headline, STATUS_LOOK, summary } from "./lib/format";
  import { onSettings, settingsApi, type Settings } from "./lib/settings";
  import { onLyrics, onPlayer, onSpotifyStatus, positionMs, spotifyApi, type Lyrics, type Player, type SpotifyStatus } from "./lib/spotify";
  import { applyAppearance, watchSystem } from "./lib/theme";
  import { seconds } from "./lib/motion";
  import { clipApi, dropApi, onClipboard, onDropIn, type Clip, type DropTarget } from "./lib/tools.svelte";
  import ContextMenu from "./lib/ui/ContextMenu.svelte";
  import Shell, { type Signal } from "./lib/island/Shell.svelte";
  import Pill from "./lib/island/Pill.svelte";
  import Panel from "./lib/island/Panel.svelte";
  import PermissionPeek from "./lib/island/PermissionPeek.svelte";
  import QuestionPeek from "./lib/island/QuestionPeek.svelte";
  import FinishedPeek from "./lib/island/FinishedPeek.svelte";
  import DigestPeek from "./lib/island/DigestPeek.svelte";
  import DevOverlay from "./lib/island/DevOverlay.svelte";
  import { focusContract, idleContract, lyricLineWidth, lyricsContract, musicContract, statusContract, withLimits } from "./lib/island/contracts";
  import { lineAt, planLyrics, wakeIn, type LyricsMode } from "./lib/island/lyrics-width";
  import { DEFAULT_ORDER, createArbiter, pickPill, type Claim, type Decision, type Policy } from "./lib/island/arbiter";
  import { measurer } from "./lib/island/measure";
  import { exposeMetrics, metrics } from "./lib/island/metrics";
  import { onInvalidate, resolveWidth, settleWidth, withFixedSize, type IslandStateId } from "./lib/island/width";
  import { px } from "./lib/theme";
  import type { TabDef } from "./lib/island/Tabs.svelte";
  import FocusTab from "./lib/views/FocusView.svelte";
  import { focusActive, focusApi, focusClockMs, focusFraction, onFocus, spokenTime, timerText, type FocusView } from "./lib/focus";
  import HomeView from "./lib/views/HomeView.svelte";
  import AgentView from "./lib/views/AgentView.svelte";
  import MusicView from "./lib/views/MusicView.svelte";
  import ClipboardView from "./lib/ClipboardView.svelte";
  import BoardView from "./lib/BoardView.svelte";
  import TrayView from "./lib/TrayView.svelte";
  import NoteView from "./lib/NoteView.svelte";
  import CalendarView from "./lib/views/CalendarView.svelte";
  import { calendarApi, onAgenda, onCalendarStatus, soonEvent, timed, untilText, type Agenda, type CalendarStatus } from "./lib/calendar";

  // ---------- data ----------
  let settings = $state<Settings | null>(null);
  let sessions = $state<Session[]>([]);
  let permissions = $state<Permission[]>([]);
  let usage = $state<Usage | null>(null);
  let feed = $state<AgentFeed>({ log: [], history: [] });
  let player = $state<Player | null>(null);
  let spotify = $state<SpotifyStatus | null>(null);
  let calStatus = $state<CalendarStatus | null>(null);
  let agenda = $state<Agenda | null>(null);
  let lyrics = $state<Lyrics | null>(null);
  let lastClip = $state<Clip | null>(null);
  let focus = $state<FocusView | null>(null);

  // ---------- interaction ----------
  let hovering = $state(false);
  let pinned = $state(false);
  /** Opened by a click or hotkey: closes again when you click elsewhere. */
  let pinnedByUser = false;
  let previewExpanded = $state(false);
  /** The tab you last used; the island reopens on it (kept across restarts). */
  let chosenTab = $state(readTab());
  function readTab() {
    try {
      return localStorage.getItem("atoll.tab") || "home";
    } catch {
      return "home";
    }
  }
  $effect(() => {
    try {
      localStorage.setItem("atoll.tab", chosenTab);
    } catch {}
  });
  /** The panel is built once (shortly after start, or on first open) and then kept. */
  let panelMounted = $state(false);
  let finishedId = $state<string | null>(null);
  let now = $state(Date.now());
  let inputFocused = $state(false);
  let windowFocused = $state(document.hasFocus());
  let dropping = $state(false);
  let dropZone = $state<DropTarget | null>(null);

  let shellEl = $state<HTMLDivElement>();
  let boardView = $state<ReturnType<typeof BoardView>>();
  let trayView = $state<ReturnType<typeof TrayView>>();
  let clipboardView = $state<ReturnType<typeof ClipboardView>>();
  let noteView = $state<ReturnType<typeof NoteView>>();
  let permPeek = $state<ReturnType<typeof PermissionPeek>>();

  // ---------- modules ----------
  const on = (k: keyof Settings["modules"]) => settings?.modules[k] ?? false;
  const claudeOn = $derived(on("claude"));
  const musicOn = $derived(on("spotify"));
  const usageOn = $derived(claudeOn && on("usageRing"));
  const boardOn = $derived(on("whiteboard"));
  const trayOn = $derived(on("fileTray"));
  const clipOn = $derived(on("clipboard"));
  const tone = $derived(settings?.mascot.tone ?? "peri");
  const live = $derived(settings?.claude.liveActivity ?? true);

  // ---------- Claude Code ----------
  const visibleSessions = $derived(claudeOn ? sessions : []);
  const visiblePermissions = $derived(claudeOn ? permissions : []);
  const top = $derived(headline(visibleSessions));
  /** Status the pill talks about; a "done" only counts for a short while. */
  const pillStatus = $derived.by<Status | null>(() => {
    if (!top || !settings) return null;
    if (top.status === "done") return now - top.updatedAt < seconds("--dur-done-hold") * 1000 ? "done" : null;
    return top.status === "idle" ? null : top.status;
  });
  const overall = $derived<Status>(visiblePermissions.length ? "attention" : (top?.status ?? "idle"));

  // ---------- music ----------
  const track = $derived(musicOn && spotify?.connected ? (player?.track ?? null) : null);
  const playing = $derived(!!track && !!player?.isPlaying);
  // A paused track stays in the pill (equaliser still) for a while, then the pill goes idle.
  let pausedSince = $state<number | null>(null);
  $effect(() => {
    if (playing || !track) pausedSince = null;
    else if (pausedSince === null) pausedSince = Date.now();
  });
  const musicInPill = $derived(
    !!track &&
      (settings?.spotify.showInPill ?? true) &&
      (playing || (pausedSince !== null && now - pausedSince < seconds("--dur-paused-hold") * 1000)),
  );
  const progress = $derived(
    musicInPill && (settings?.spotify.progressLine ?? true) && player && track?.durationMs ? positionMs(player, now) / track.durationMs : null,
  );
  /** Settings → Spotify: the collapsed pill sings along instead of naming the track. */
  const lyricsInPill = $derived(settings?.spotify.pillText === "lyrics" && (settings?.spotify.lyrics ?? true));
  /** The synced line being sung, with its start time (so a repeated line still rolls in);
   *  null (so title · artist shows) before the first line or without synced lyrics. */
  const pillLyric = $derived.by<{ text: string; at: number } | null>(() => {
    if (!lyricLines || !player) return null;
    const i = lineAt(lyricLines, positionMs(player, lyricNow));
    return i < 0 ? null : { text: lyricLines[i].text || "♪", at: lyricLines[i].t };
  });
  /** The synced lines of the track that is playing (null: none, not loaded yet, or instrumental). */
  const lyricLines = $derived(
    lyricsInPill && track && lyrics?.trackId === track.id && !lyrics.instrumental && lyrics.synced.length ? lyrics.synced : null,
  );
  /** When the sung line was last worked out. Set by a timer aimed at each line
   *  (and at the moment its width change is due), not by the general tick, so
   *  a line lands on time instead of up to a tick late. */
  let lyricNow = $state(Date.now());
  $effect(() => {
    if (!lyricLines || !player || !playing || !musicInPill) return;
    const lines = lyricLines;
    const p = player;
    const lead = seconds("--dur-lyric-lead") * 1000;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onLine = () => {
      const t = Date.now();
      lyricNow = t;
      const wait = wakeIn(lines, positionMs(p, t), lead);
      // Nothing is scheduled after the last line, while paused, or with the pill showing something else.
      if (wait !== null) timer = setTimeout(onLine, Math.max(16, wait + 4));
    };
    onLine();
    return () => clearTimeout(timer);
  });

  // ---------- calendar ----------
  const calOn = $derived(on("calendar"));
  /** Settings → Calendar: the next event shows in the pill this many minutes before it starts. */
  const soon = $derived(
    calOn && calStatus?.connected && agenda ? soonEvent(timed(agenda.events, now, false), now, settings?.calendar.pillMinutes ?? 10) : null,
  );

  // ---------- focus timer ----------
  /** A session or break in progress (running or paused); null when ready or switched off. */
  const timer = $derived(on("focus") && focusActive(focus) ? focus : null);
  const inFocus = $derived(timer?.phase === "focus");
  /** Settings → Focus timer → Do not disturb, while a focus session is on. */
  const dnd = $derived(inFocus && (settings?.focus.dnd ?? true));
  /** A listed app came to the front: the pill says so for a few seconds. */
  const nudging = $derived(!!timer?.nudge && inFocus && now - timer.nudge.at < seconds("--dur-nudge") * 1000);
  /** The alerts held back during focus, shown when the break starts (until dismissed or timed out). */
  let digestHiddenFor = $state<number | null>(null);
  const digestKey = $derived(timer?.held.length ? timer.held[0].at : null);
  const digest = $derived(
    timer?.phase === "break" && timer.digestOpen && timer.held.length && digestHiddenFor !== digestKey ? timer : null,
  );
  let digestTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    if (!digest) return;
    const key = digestKey;
    clearTimeout(digestTimer);
    digestTimer = setTimeout(() => (digestHiddenFor = key), seconds("--dur-digest-hold") * 1000);
    return () => clearTimeout(digestTimer);
  });

  // ---------- what the island shows ----------
  const expanded = $derived(hovering || pinned || previewExpanded || dropping || (inputFocused && windowFocused));
  const perm = $derived(visiblePermissions[0] ?? null);
  const finished = $derived(finishedId ? (visibleSessions.find((s) => s.id === finishedId) ?? null) : null);
  // Claude ended its turn on a question with reply choices: it pops up (like a
  // permission request) until you answer it or choose Later.
  let askHidden = $state<string[]>([]);
  const askKey = (s: Session) => `${s.id}:${s.ask?.at ?? 0}`;
  const asking = $derived(visibleSessions.find((s) => s.ask && s.status === "attention" && !askHidden.includes(askKey(s))) ?? null);
  async function replyToAsk(s: Session, text: string) {
    await api.replyInClaude(s.id, text);
  }
  const mode = $derived<"pill" | "peek" | "panel">(perm || asking ? "peek" : expanded ? "panel" : digest || finished ? "peek" : "pill");
  // ---------- which state gets the pill (src-tauri/src/arbiter.rs) ----------
  // Every source only says what it has to offer; the arbiter decides. Needs
  // you and errors come first, then the focus timer, then agent status, an
  // event about to start, and music (Settings → Spotify can put music ahead).
  const musicFirst = $derived(settings?.spotify.pillPriority === "spotify");
  const claims = $derived.by<Claim[]>(() => {
    const c: Claim[] = [];
    if (perm) c.push({ key: perm.questions?.length ? "peek:question" : "peek:permission", token: String(perm.id) });
    if (asking) c.push({ key: "peek:ask", token: askKey(asking) });
    if (expanded) c.push({ key: "panel" });
    if (digest) c.push({ key: "peek:digest", token: String(digestKey) });
    if (finished) c.push({ key: "peek:finished", token: finished.id });
    if (pillStatus === "attention") c.push({ key: "needs-you" });
    else if (pillStatus === "error") c.push({ key: "error" });
    else if (pillStatus === "working") c.push({ key: "busy" });
    else if (pillStatus === "done") c.push({ key: "done" });
    if (timer) c.push({ key: "focus" });
    if (soon) c.push({ key: "event" });
    if (musicInPill) c.push({ key: "music" });
    return c;
  });
  const policy = $derived<Policy>({ musicFirst, focusDnd: dnd, errorsInFocus: settings?.focus.errors === "show" });
  let decision = $state<Decision | null>(null);
  const arbiter = createArbiter((d) => (decision = d));
  // Posted only when the set of claims changes, never on a tick.
  const claimSig = $derived(JSON.stringify([claims, policy]));
  $effect(() => {
    void claimSig;
    untrack(() => arbiter.post(claims, policy));
  });
  /** The first state in the arbiter's order that is still on offer: a state
   *  that ends is replaced at once, a new one waits for the arbiter's answer. */
  const pillKey = $derived(pickPill(decision?.order ?? DEFAULT_ORDER, new Set(claims.map((c) => c.key))));
  const pillKind = $derived<"idle" | "status" | "music" | "timer" | "event">(
    pillKey === "focus" ? "timer" : pillKey === "event" || pillKey === "music" || pillKey === "idle" ? pillKey : "status",
  );
  const timerNote = $derived(nudging && timer?.nudge ? `${timer.nudge.app} · back to focus?` : (timer?.intention ?? ""));
  /** Settings → Appearance size and roundness: the outline re-fits as soon as any of them change. */
  const sizeKey = $derived.by(() => {
    const a = settings?.appearance;
    return a ? [a.collapsedWidth, a.collapsedHeight, a.expandedWidth, a.expandedHeight, a.roundness].join() : "";
  });
  const signal = $derived<Signal>(
    mode === "peek"
      ? perm || asking
        ? "accent"
        : "done"
      : mode === "panel"
        ? null
        : pillKind === "status" && pillStatus
          ? STATUS_LOOK[pillStatus].signal
          : pillKind === "timer" && nudging
            ? "accent"
            : null,
  );

  const MODULE_TABS: Record<string, Omit<TabDef, "id">> = {
    claude: { label: "Agent", icon: "agent" },
    spotify: { label: "Spotify", icon: "music" },
    clipboard: { label: "Clips", icon: "clipboard" },
    whiteboard: { label: "Board", icon: "whiteboard" },
    fileTray: { label: "Files", icon: "folder" },
    note: { label: "Note", icon: "note" },
    calendar: { label: "Calendar", icon: "calendar" },
    focus: { label: "Focus", icon: "focus" },
  };
  const tabs = $derived<TabDef[]>([
    { id: "home", label: "Home", icon: "home" },
    ...(settings?.moduleOrder ?? [])
      .filter((k) => MODULE_TABS[k] && settings?.modules[k as keyof Settings["modules"]])
      .map((k) => ({ id: k, ...MODULE_TABS[k] })),
  ]);
  const tab = $derived(tabs.some((t) => t.id === chosenTab) ? chosenTab : "home");

  // ---------- helpers ----------
  function applySettings(s: Settings) {
    settings = s;
    applyAppearance(s.appearance);
    // Sizes and type may have changed: measure text afresh.
    measurer.clear();
    // Settings → Mascot → Show Pip off hides Pip everywhere in the island (app.css).
    document.documentElement.dataset.mascot = s.modules.mascot ? "on" : "off";
  }

  let collapseTimer: ReturnType<typeof setTimeout> | undefined;
  function setHover(inside: boolean) {
    clearTimeout(collapseTimer);
    if (inside) {
      hovering = true;
      finishedId = null; // the peek opens into the panel
    } else {
      collapseTimer = setTimeout(() => (hovering = false), seconds("--dur-hover-grace") * 1000);
    }
  }

  /** Close the island (after paste, Esc, chevron, clicking elsewhere). */
  function done() {
    finishedId = null;
    pinned = false;
    pinnedByUser = false;
    (document.activeElement as HTMLElement | null)?.blur?.();
  }

  function openPanel(t = chosenTab) {
    chosenTab = t;
    pinned = true;
    pinnedByUser = true;
    finishedId = null;
  }

  // Module hotkey: open on that tab; the same hotkey again closes it.
  async function openTab(t: string) {
    if (pinned && tab === t) {
      done();
      return;
    }
    openPanel(t);
    await tick();
    if (t === "note") noteView?.focus();
    if (t === "clipboard") clipboardView?.focusSearch();
  }

  function keydown(e: KeyboardEvent) {
    if (perm && !permPeek?.isTyping()) {
      if (e.key === "Escape") permPeek?.decide("deny");
      else if (e.key === "Enter" && !permPeek?.risky(perm)) permPeek?.decide("allow");
      else return;
      e.preventDefault();
      return;
    }
    if (e.key === "Escape") done();
  }

  // ---------- files dragged onto the island ----------
  // Files, images and text from any app (dnd.rs takes the drop natively).
  const ownDrag = () => !!(boardView?.isDraggingOut() || trayView?.isDraggingOut() || clipboardView?.isDraggingOut());
  const dropTabs: DropTarget[] = ["whiteboard", "fileTray", "clipboard"];
  const dropOn = (t: DropTarget) => (t === "whiteboard" ? boardOn : t === "fileTray" ? trayOn : clipOn);
  function zoneAt(x: number, y: number): DropTarget | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-zone]");
    return (el?.dataset.zone as DropTarget | undefined) ?? null;
  }
  async function onDrop(x: number, y: number) {
    let target: DropTarget | null;
    if (dropTabs.includes(tab as DropTarget) && dropOn(tab as DropTarget)) target = tab as DropTarget;
    else target = zoneAt(x, y) ?? dropTabs.find((t) => t !== "clipboard" && dropOn(t)) ?? (clipOn ? "clipboard" : null);
    if (!target) return;
    const at = target === "whiteboard" && tab === "whiteboard" ? boardView?.boardPoint(x, y) : null;
    await dropApi.commit(target, at?.x, at?.y);
    chosenTab = target;
  }

  // ---------- clearing messages ----------
  /**
   * Clear whatever message the island is showing (the hotkey, Ctrl+Alt+D by
   * default; the × on a status tag clears that one session directly). The session stays; it goes back to idle until Claude
   * Code says something new. A permission request is never dropped silently:
   * it keeps its own Allow / Deny / Answer in Claude.
   */
  function dismissMessage() {
    if (perm) return;
    if (digest) {
      focusApi.act("dismiss");
      return;
    }
    finishedId = null;
    const target = top && clearable(top.status) ? top : visibleSessions.find((s) => clearable(s.status));
    if (target) api.clearSession(target.id).catch((e) => console.error(e));
  }

  // Settings → AI agents → "Clear stuck messages": an error or "needs you" that
  // nothing has updated for that long clears itself (e.g. after a usage limit).
  $effect(() => {
    const mins = settings?.claude.clearAfterMin ?? 15;
    if (!mins) return;
    const asking = new Set(visiblePermissions.map((p) => p.sessionId));
    const stuck = visibleSessions.filter((s) => (s.status === "error" || s.status === "attention") && !asking.has(s.id));
    const timers = stuck.map((s) =>
      setTimeout(() => api.clearSession(s.id).catch(() => {}), Math.max(0, s.updatedAt + mins * 60_000 - Date.now())),
    );
    return () => timers.forEach(clearTimeout);
  });

  // ---------- finished-task peek ----------
  const lastStatus = new Map<string, Status>();
  let peekTimer: ReturnType<typeof setTimeout> | undefined;
  function watchFinishes(list: Session[]) {
    for (const s of list) {
      const before = lastStatus.get(s.id);
      // "Pop up when a task finishes" off: the pill just says Done. During focus,
      // Settings → Focus timer decides (held for the break, or silent).
      const focusQuiet = dnd && settings?.focus.finished !== "show";
      if (before && before !== "done" && s.status === "done" && !expanded && !focusQuiet && (settings?.claude.popupOnDone ?? true)) {
        finishedId = s.id;
        clearTimeout(peekTimer);
        peekTimer = setTimeout(() => (finishedId = null), seconds("--dur-peek-hold") * 1000);
      }
      lastStatus.set(s.id, s.status);
    }
  }

  onMount(() => {
    exposeMetrics();
    // Text is measured only once the fonts are in; until then states sit at their minimum width.
    const stopMeasure = [measurer.onChange(() => measured++), onInvalidate(() => measured++)];
    measurer.start();
    settingsApi.get().then(applySettings);
    api.getSessions().then((s) => {
      s.forEach((x) => lastStatus.set(x.id, x.status));
      sessions = s;
    });
    api.getPermissions().then((p) => (permissions = p));
    api.getUsage().then((u) => (usage = u));
    api.getAgentFeed().then((f) => (feed = f));
    spotifyApi.status().then((s) => (spotify = s));
    spotifyApi.player().then(async (p) => {
      player = p;
      if (p?.track) {
        const cached = await spotifyApi.lyrics(p.track.id);
        if (cached) lyrics = cached;
      }
    });
    clipApi.list().then((c) => (lastClip = c[0] ?? null));
    focusApi.get().then((f) => (focus = f));
    calendarApi.status().then((c) => (calStatus = c));
    calendarApi.agenda().then((a) => (agenda = a));

    const unlisten = [
      onFocus((f) => (focus = f)),
      onUsage((u) => (usage = u)),
      onSettings(applySettings),
      onSessions((s) => {
        watchFinishes(s);
        sessions = s;
      }),
      onAgentFeed((f) => (feed = f)),
      onPermissions((p) => (permissions = p)),
      onPlayer((p) => (player = p)),
      onSpotifyStatus((s) => (spotify = s)),
      onCalendarStatus((c) => (calStatus = c)),
      onAgenda((a) => (agenda = a)),
      onLyrics((l) => (lyrics = l)),
      onClipboard((c) => (lastClip = c[0] ?? null)),
      onHover(setHover),
      listen("toggle-pin", () => (pinned ? done() : openPanel(tab))),
      listen<boolean>("preview-expand", (e) => (previewExpanded = e.payload)),
      listen<string>("open-tab", (e) => openTab(e.payload)),
      listen("dismiss-message", () => dismissMessage()),
      onDropIn((p) => {
        if (p.type === "leave") {
          dropping = false;
          dropZone = null;
          return;
        }
        if (ownDrag() || !(boardOn || trayOn || clipOn)) return;
        const x = p.x / devicePixelRatio;
        const y = p.y / devicePixelRatio;
        if (p.type === "enter" || p.type === "over") {
          dropping = true;
          dropZone = zoneAt(x, y);
        } else if (p.type === "drop") {
          dropping = false;
          dropZone = null;
          onDrop(x, y).catch((err) => console.error(err));
        }
      }),
    ];
    const stopSystem = watchSystem(() => settings && applyAppearance(settings.appearance));
    // A focused box that disappears while Svelte is updating the page fires
    // focusout in the middle of that update, where state can't be changed (the
    // change was refused and the island stayed open). So check where focus
    // ended up a moment later instead, once the update has finished.
    const syncFocus = () =>
      queueMicrotask(() => (inputFocused = document.hasFocus() && !!document.activeElement?.matches("input, textarea")));
    const onFocusIn = syncFocus;
    const onFocusOut = syncFocus;
    const onWinFocus = () => (windowFocused = true);
    const onWinBlur = () => {
      windowFocused = false;
      if (pinnedByUser) done();
      // Don't leave a text box focused in the background: when Windows gives
      // the island focus again it returns to that box, which opens the panel.
      else (document.activeElement as HTMLElement | null)?.blur?.();
    };
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusOut);
    window.addEventListener("focus", onWinFocus);
    window.addEventListener("blur", onWinBlur);
    window.addEventListener("keydown", keydown);
    const pointerInput = () => (document.documentElement.dataset.input = "pointer");
    const keyInput = () => (document.documentElement.dataset.input = "keyboard");
    window.addEventListener("pointerdown", pointerInput, true);
    window.addEventListener("keydown", keyInput, true);
    // Build the panel in the background shortly after start, so the first open is smooth too.
    const premount = setTimeout(() => (panelMounted = true), 1500);
    return () => {
      clearTimeout(premount);
      arbiter.stop();
      stopMeasure.forEach((stop) => stop());
      window.removeEventListener("pointerdown", pointerInput, true);
      window.removeEventListener("keydown", keyInput, true);
      unlisten.forEach((p) => p.then((f) => f()));
      stopSystem();
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("focusout", onFocusOut);
      window.removeEventListener("focus", onWinFocus);
      window.removeEventListener("blur", onWinBlur);
      window.removeEventListener("keydown", keydown);
    };
  });

  $effect(() => {
    if (mode === "panel") panelMounted = true;
  });
  // The panel only needs fresh time while it is showing.
  let panelNow = $state(Date.now());
  $effect(() => {
    if (mode === "panel") panelNow = now;
  });

  // The clock only ticks while something on screen depends on time.
  // Idle and collapsed, nothing runs.
  const needsClock = $derived(expanded || !!perm || !!asking || !!finished || musicInPill || pillStatus === "working" || pillStatus === "done" || !!timer);
  // (Lyric lines have their own timer, aimed at each line: see lyricNow.)
  const tickMs = $derived(seconds("--dur-tick") * 1000);
  $effect(() => {
    if (!needsClock) return;
    now = Date.now();
    const id = setInterval(() => (now = Date.now()), tickMs);
    return () => clearInterval(id);
  });

  // The click-through area follows the island's shape natively (Shell.svelte
  // sends each move to shape.rs), so nothing is reported from here.

  /** "Ctrl+Alt+KeyY" -> "Ctrl+Alt+Y" for hints. */
  const keyName = (combo?: string) => (combo ? combo.replace(/Key([A-Z])/, "$1").replace(/Digit(\d)/, "$1") : "—");

  // Screen readers: announce arrivals and status changes (politely).
  let announce = $state("");
  let lastAnnounced = "";
  $effect(() => {
    let text = "";
    if (perm?.questions?.length) text = `${perm.project || "Claude"} asks: ${perm.questions[0].question}`;
    else if (asking?.ask) text = `${asking.project || "Claude"} asks: ${asking.ask.question}`;
    else if (perm) text = `Permission needed: ${perm.label} ${perm.detail} in ${perm.project || "Claude Code"}`;
    else if (digest) text = `Focus session over. While you focused: ${digest.held.length} ${digest.held.length === 1 ? "update" : "updates"}`;
    else if (finished) text = `${finished.project || "Claude Code"} finished`;
    else if (nudging && timer?.nudge) text = `${timer.nudge.app} is in front. Back to focus?`;
    else if (pillStatus && top && pillStatus !== "working" && pillKind === "status") text = `${STATUS_LOOK[pillStatus].word}: ${summary(top, live)}`;
    else if (timer) text = timer.phase === "break" ? "Break started" : timer.running ? "Focus started" : "Focus paused";
    if (text && text !== lastAnnounced) announce = text;
    lastAnnounced = text;
  });

  const pillLabel = $derived.by(() => {
    if (pillKind === "timer" && timer) {
      const t = spokenTime(focusClockMs(timer, now));
      const what = timer.phase === "break" ? "Break" : "Focus timer";
      const state = !timer.running ? `paused, ${t} left` : timer.lengthMs ? `${t} left` : `${t} so far`;
      return `${what}, ${state}${timerNote ? `. ${timerNote}` : ""}. Open Atoll`;
    }
    if (pillKind === "event" && soon) return `${soon.title}, ${untilText(soon.startMs, now) === "now" ? "starting now" : `starts ${untilText(soon.startMs, now)}`}. Open Atoll`;
    if (pillKind === "music" && track) return `Now playing ${track.name} by ${track.artists}. Open Atoll`;
    if (pillKind === "status" && top && pillStatus) return `Agent ${STATUS_LOOK[pillStatus].word}: ${summary(top, live)}. Open Atoll`;
    return "Atoll, idle. Open";
  });

  // ---------- adaptive width (docs/contracts/width-contract.md) ----------
  // The pill is as wide as what it shows: each state declares its width
  // contract, the island resolves one width from it and Shell springs there.
  const eventWord = $derived(soon ? (untilText(soon.startMs, now) === "now" ? "Now" : untilText(soon.startMs, now).replace(/^in /, "In ")) : "");
  const timerWord = $derived(timer ? (!timer.running ? "Paused" : timer.phase === "break" ? "Break" : "Focus") : "");
  const timerTime = $derived(timer ? timerText(focusClockMs(timer, now), !timer.lengthMs) : "");
  const statusText = $derived(top ? summary(top, live) : "");
  const statusMeta = $derived(pillStatus === "working" && top ? elapsed(now - top.startedAt) : "");
  const showPip = $derived(settings?.modules.mascot ?? true);
  const pillState = $derived<IslandStateId>(
    pillKind === "timer"
      ? "focus"
      : pillKind === "music"
        ? lyricsInPill
          ? "lyrics"
          : "music"
        : pillKind === "event"
          ? "event"
          : pillKind === "status" && pillStatus && pillStatus !== "idle"
            ? pillStatus === "working"
              ? "busy"
              : pillStatus
            : "idle",
  );
  /** Bumped when text may measure differently (fonts arrived, display scale changed, a state asked). */
  let measured = $state(0);

  // Lyrics: the pill keeps its fixed size (Collapsed width) and only grows for
  // a line too long to fit, then goes back (lyrics-width.ts).
  // Settings → Spotify will choose the mode; until that setting exists it is Follow.
  const lyricsMode = $derived<LyricsMode>((settings?.spotify as { lyricsWidth?: LyricsMode } | undefined)?.lyricsWidth ?? "follow");
  const lyricWidths = $derived.by(() => {
    void measured;
    return lyricLines && measurer.ready ? lyricLines.map((l) => lyricLineWidth(measurer, l.text || "♪")) : null;
  });
  const titleWidth = $derived.by(() => {
    void measured;
    return track && measurer.ready ? musicContract({ title: track.name, artist: track.artists }).measure(measurer) : 0;
  });
  const lyricPlan = $derived.by(() => {
    if (pillState !== "lyrics" || !player) return null;
    return planLyrics({
      mode: lyricsMode,
      lines: lyricLines ?? [],
      widths: lyricWidths ?? [],
      titleWidth,
      base: pillCeiling,
      pos: positionMs(player, lyricNow),
      lead: seconds("--dur-lyric-lead") * 1000,
      lookahead: seconds("--dur-lyric-lookahead") * 1000,
      step: px("--lyrics-step"),
    });
  });
  const pillContract = $derived.by(() => {
    void measured;
    const c =
      pillState === "focus" && timer
        ? focusContract({ time: timerTime, word: timerWord, strong: nudging, note: timerNote })
        : pillState === "lyrics"
          ? lyricsContract(lyricPlan?.width ?? pillCeiling)
          : pillState === "music" && track
            ? musicContract({ title: track.name, artist: track.artists })
            : pillState === "event" && soon
              ? statusContract("event", { word: eventWord, text: soon.title, showPip })
              : pillKind === "status" && top && pillStatus
                ? statusContract(pillState, { word: STATUS_LOOK[pillStatus].word, text: statusText, meta: statusMeta, showPip })
                : idleContract(showPip);
    return withLimits(c, measurer);
  });
  /** Settings → Appearance → Collapsed width is the pill's fixed size; idle is half of it. */
  const pillCeiling = $derived(settings?.appearance.collapsedWidth ?? 384);
  /** A long lyric line may take the pill past that, up to the width of the open panel. */
  const lyricsCeiling = $derived(Math.max(pillCeiling, settings?.appearance.expandedWidth ?? pillCeiling));
  const pillFit = $derived.by(() => {
    void measured;
    const fit = resolveWidth(pillContract, measurer, pillState === "lyrics" ? lyricsCeiling : pillCeiling);
    // Idle rests at half the fixed size; every other state has the fixed size
    // and only lyrics grow past it, for a line too long to fit.
    return withFixedSize(fit, pillState === "idle" ? pillCeiling / 2 : pillCeiling);
  });
  let pillW = $state(64);
  /** What the current width belongs to: the state, and for music the track too
   *  (a new song takes its own width at once, like a new state). */
  const widthKey = $derived(pillState === "lyrics" || pillState === "music" ? `${pillState}|${track?.id ?? ""}` : pillState);
  let widthFor: string | null = null;
  $effect(() => {
    const fit = pillFit;
    const key = widthKey;
    // Lyrics → Follow each line: every line takes its own width, both ways, at once.
    const everyLine = pillState === "lyrics" && lyricsMode === "follow";
    const band = () => px(pillState === "lyrics" ? "--lyrics-deadband" : "--width-deadband");
    untrack(() => {
      // A new state takes its own width at once. Within one state the pill
      // grows at once (or its text would be cut) but is slow to shrink: small
      // differences are ignored, and a real one must last a few seconds first.
      // Otherwise a busy agent's step names would make the pill breathe.
      const next = key === widthFor && !everyLine ? settleWidth(pillW, fit.width, band()) : fit.width;
      if (key !== widthFor || everyLine || next >= pillW) {
        clearTimeout(shrinkTimer);
        shrinkTimer = undefined;
        pillW = next;
        widthFor = key;
      } else if (!shrinkTimer) {
        shrinkTimer = setTimeout(() => {
          shrinkTimer = undefined;
          if (widthKey === widthFor) pillW = settleWidth(pillW, pillFit.width, band());
        }, seconds("--dur-width-hold") * 1000);
      }
    });
  });
  let shrinkTimer: ReturnType<typeof setTimeout> | undefined;

  // What the island is showing and why, for the dev overlay and the test harness.
  const islandState = $derived(
    mode === "panel"
      ? "panel"
      : mode === "peek"
        ? perm?.questions?.length
          ? "peek:question"
          : perm
            ? "peek:permission"
            : asking
              ? "peek:ask"
              : digest
                ? "peek:digest"
                : "peek:finished"
        : pillState,
  );
  const islandReason = $derived.by(() => {
    if (mode === "panel") return hovering ? "pointer is over the island" : pinned ? "opened by click or hotkey" : dropping ? "something is being dragged in" : previewExpanded ? "shown open by Settings" : "a text box has focus";
    if (mode === "peek") return perm ? "a permission request or question is waiting" : asking ? "Claude asked a question" : digest ? "break started: what was held during focus" : "a task just finished";
    // The pill: the arbiter's own reason (until its answer for a state that just ended arrives, the next in its order).
    return decision && decision.pill === pillKey && !decision.surface ? decision.reason : pillKey === "idle" ? "nothing to show" : `next in the arbiter's order: ${pillKey}`;
  });
  $effect(() => {
    metrics.setState(islandState, islandReason);
    metrics.setLimits(Math.max(pillContract.min, pillState === "idle" ? pillCeiling / 2 : pillCeiling), Math.max(pillContract.min, Math.min(pillContract.max, pillState === "lyrics" ? lyricsCeiling : pillCeiling)));
    metrics.lyrics = lyricPlan
      ? {
          mode: lyricsMode,
          line: lyricPlan.line,
          nextChangeIn: lyricPlan.wakeIn ?? -1,
          lookaheadTarget: Number.isFinite(lyricPlan.width) ? lyricPlan.width : pillCeiling,
        }
      : null;
  });

  // Dev overlay (debug builds): Ctrl + Shift + click the island.
  let devOverlay = $state(import.meta.env.DEV && readFlag("atoll.devOverlay"));
  function readFlag(key: string) {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  }
  function devClick(e: MouseEvent) {
    if (!import.meta.env.DEV || !e.ctrlKey || !e.shiftKey) return;
    e.stopPropagation();
    e.preventDefault();
    devOverlay = !devOverlay;
    try {
      localStorage.setItem("atoll.devOverlay", devOverlay ? "1" : "0");
    } catch {}
  }
</script>

<div class="sr-only" aria-live="polite">{announce}</div>
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="stage" onclickcapture={devClick}>
  {#if settings}
    <Shell
      {mode}
      {pillW}
      quiet={pillState === "idle"}
      {sizeKey}
      {signal}
      progress={mode === "pill" && pillKind === "music" ? progress : null}
      bind:el={shellEl}
      peek={perm || asking || digest || finished ? peekLayer : undefined}
      panel={panelMounted ? panelLayer : undefined}
    >
      {#snippet pill()}
        <!-- One block per state: when the state changes, the old content
             blurs out at its old width while the new one arrives at its own. -->
        {#key pillState}
        {#if pillKind === "timer" && timer}
          <Pill
            kind="timer"
            {tone}
            tint={nudging ? "accent" : timer.phase === "break" ? "done" : "busy"}
            ring={timer.lengthMs ? focusFraction(timer, now) : 0}
            time={timerTime}
            word={timerWord}
            text={timerNote}
            width={pillW}
            label={pillLabel}
            onclick={() => openPanel("focus")}
          />
        {:else if pillKind === "music" && track}
          <Pill
            kind="music"
            {tone}
            title={track.name}
            artist={track.artists}
            art={track.image}
            lyric={pillLyric?.text ?? null}
            lyricAt={pillLyric?.at ?? null}
            roll={lyricsInPill}
            {playing}
            width={pillW}
            label={pillLabel}
            onclick={() => openPanel(chosenTab)}
          />
        {:else if pillKind === "event" && soon}
          <Pill
            kind="status"
            {tone}
            mood="idle"
            icon="calendar"
            word={eventWord}
            tint="accent"
            text={soon.title}
            width={pillW}
            label={pillLabel}
            onclick={() => openPanel(tabs.some((t) => t.id === "calendar") ? "calendar" : chosenTab)}
          />
        {:else if pillKind === "status" && top && pillStatus}
          {@const look = STATUS_LOOK[pillStatus]}
          <Pill
            kind="status"
            {tone}
            mood={look.mood}
            icon={look.icon}
            word={look.word}
            tint={pillStatus === "working" ? "busy" : pillStatus === "done" ? "done" : pillStatus === "error" ? "error" : "accent"}
            text={statusText}
            meta={statusMeta}
            bob={playing && settings!.mascot.reactMusic}
            width={pillW}
            label={pillLabel}
            onclick={() => openPanel(chosenTab)}
          />
        {:else}
          <Pill kind="idle" {tone} bob={playing && settings!.mascot.reactMusic} width={pillW} label={pillLabel} onclick={() => openPanel(chosenTab)} />
        {/if}
        {/key}
      {/snippet}
    </Shell>
    {#if devOverlay}
      <div class="dev"><DevOverlay {decision} /></div>
    {/if}

    {#snippet peekLayer()}
      {#if perm?.questions?.length}
        {#key perm.id}
          <QuestionPeek kind="question" request={perm} project={perm.project} {now} {tone} onLater={() => {}} />
        {/key}
      {:else if perm}
        {#key perm.id}
          <PermissionPeek
            bind:this={permPeek}
            request={perm}
            folder={visibleSessions.find((s) => s.id === perm.sessionId)?.cwd ?? ""}
            queued={visiblePermissions.length}
            {now}
            {tone}
            focused={windowFocused}
            keys={{ allow: keyName(settings!.hotkeys.modules.allowPermission), deny: keyName(settings!.hotkeys.modules.denyPermission) }}
          />
        {/key}
      {:else if asking?.ask}
        {#key askKey(asking)}
          <QuestionPeek
            kind="ask"
            ask={asking.ask}
            project={asking.project}
            {tone}
            onPick={(label) => replyToAsk(asking, label)}
            onLater={() => (askHidden = [...askHidden, askKey(asking)])}
          />
        {/key}
      {:else if digest}
        <DigestPeek
          held={digest.held}
          breakLabel={digest.longBreak ? "Long break" : "Break"}
          {tone}
          onDismiss={() => focusApi.act("dismiss")}
          onView={() => {
            digestHiddenFor = digestKey;
            openPanel("focus");
          }}
        />
      {:else if finished}
        <FinishedPeek session={finished} {tone} onDismiss={() => (finishedId = null)} onView={() => openPanel("claude")} />
      {/if}
    {/snippet}

    {#snippet panelLayer()}
      <Panel
        {tabs}
        active={tab}
        status={overall}
        {tone}
        onselect={(id) => (chosenTab = id)}
        onSettings={() => settingsApi.openDashboard()}
        onCollapse={done}
        onQuit={() => api.quit()}
      >
        {#snippet pane(id: string)}
          {#if id === "home"}
            <HomeView
              {live}
              session={top}
              sessionCount={visibleSessions.length}
              player={track ? player : null}
              connected={!!spotify?.connected}
              {usage}
              {lastClip}
              focus={on("focus") ? focus : null}
              now={panelNow}
              open={(t) => (chosenTab = tabs.some((x) => x.id === t) ? t : "home")}
              onClear={(id) => api.clearSession(id).catch((e) => console.error(e))}
              clearKey={keyName(settings!.hotkeys.modules.dismissMessage) === "—" ? "" : keyName(settings!.hotkeys.modules.dismissMessage)}
            />
          {:else if id === "claude"}
            <AgentView sessions={visibleSessions} onReply={(s, text) => replyToAsk(s, text)} onClear={(id) => api.clearSession(id)} {live} {feed} {usage} window={settings!.usage.ringWindow} {usageOn} now={panelNow} {tone} />
          {:else if id === "spotify"}
            <MusicView {player} status={spotify} {lyrics} lyricsOn={settings!.spotify.lyrics} now={panelNow} {tone} />
          {:else if id === "clipboard"}
            <ClipboardView bind:this={clipboardView} clickPaste={settings!.clipboard.clickPaste} paused={settings!.clipboard.paused} {tone} onDone={done} />
          {:else if id === "whiteboard"}
            <BoardView bind:this={boardView} clickPaste={settings!.whiteboard.clickPaste} onDone={done} />
          {:else if id === "fileTray"}
            <TrayView bind:this={trayView} />
          {:else if id === "note"}
            <NoteView bind:this={noteView} />
          {:else if id === "calendar"}
            <CalendarView
              status={calStatus}
              {agenda}
              daysAhead={settings!.calendar.daysAhead}
              showDeclined={settings!.calendar.showDeclined}
              now={panelNow}
              {tone}
            />
          {:else if id === "focus" && focus}
            <FocusTab view={focus} settings={settings!.focus} now={panelNow} />
          {/if}
        {/snippet}
      </Panel>
      {#if dropping && !(dropTabs.includes(tab as DropTarget) && dropOn(tab as DropTarget))}
        <div class="drop" aria-hidden="true">
          {#if boardOn}<div class="zone" class:hot={dropZone === "whiteboard"} data-zone="whiteboard">Add to whiteboard</div>{/if}
          {#if trayOn}<div class="zone" class:hot={dropZone === "fileTray"} data-zone="fileTray">Keep in file tray</div>{/if}
          {#if clipOn}<div class="zone" class:hot={dropZone === "clipboard"} data-zone="clipboard">Add to clipboard</div>{/if}
        </div>
      {/if}
    {/snippet}
  {/if}
  <ContextMenu />
</div>

<style>
  .stage {
    display: flex;
    justify-content: center;
    align-items: flex-start;
    height: 100%;
    padding-top: var(--stage-top);
    box-sizing: border-box;
    position: relative;
  }
  /* Dev overlay: just under the island, kept inside the window when the panel is open. */
  .dev {
    position: absolute;
    left: 50%;
    top: min(calc(var(--stage-top) + var(--island-h, var(--pill-h)) + var(--sp-8)), calc(100% - 10.5em));
    transform: translateX(-50%);
    z-index: 10;
    pointer-events: none;
  }
  .drop {
    position: absolute;
    inset: 0;
    z-index: 5;
    padding: var(--sp-16);
    display: flex;
    gap: var(--sp-16);
    background: rgba(var(--c-hull-rgb), var(--hover-alpha));
    border-radius: inherit;
  }
  .zone {
    flex: 1;
    display: grid;
    place-items: center;
    border: 2px dashed var(--c-dash);
    border-radius: var(--r-card);
    color: var(--c-muted);
    font-weight: var(--fw-strong);
    transition: border-color var(--dur-fast), color var(--dur-fast), background-color var(--dur-fast);
  }
  .zone.hot {
    border-color: var(--c-accent);
    color: var(--c-ink);
    background: color-mix(in srgb, var(--c-accent) 12%, transparent);
  }
</style>
