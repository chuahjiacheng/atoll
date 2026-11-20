# Atoll (formerly Crest)

A personal "island" overlay for Windows 10/11. It sits at the top centre of the screen and
follows your Claude Code sessions: what is being read or edited, which command is running,
permission prompts you can answer in place, and when a session finishes or needs you. More
modules (Spotify, clipboard, whiteboard, focus...) plug into the same shell.

Private, single-user tool. No licensing, telemetry, accounts or auto-update.

## Status

| Milestone | Scope | State |
|---|---|---|
| 1 | Shell: settings dashboard with live preview, appearance, module switches, saved settings, global hotkeys, tray, start with Windows | ✅ |
| 2 | Claude Code: live activity, Allow/Deny, alerts (chime, toast, flash), usage ring, Chat tab watcher (experimental) | ✅ |
| 3 | Spotify + synced lyrics ✅ (part 1); clipboard history, whiteboard, file tray, quick note ✅ (part 2); focus timer ✅ (part 3) | built, waiting for your approval |
| 4 | Mascot and "ask about my screen" | planned |

## Design

The look comes from `design/` (two clickable prototypes + `design/designtokens.md`), recorded in
[PRODUCT.md](PRODUCT.md) and [DESIGN.md](DESIGN.md) (Impeccable). Implementation rules:

- **One set of variables:** every colour, size, radius, spacing and timing lives in
  `src/styles/tokens.css` (dark default, `data-theme="light"` override). Settings-driven values
  (sizes, opacity, blur, chosen colours, animation speed) are applied by `src/lib/theme.ts`.
- **Motion:** GSAP (`src/lib/motion.ts`) with the design's durations (expand 280 ms, peek 260 ms,
  collapse 200 ms, state changes 160/240 ms) and one settling curve, divided by the animation
  speed setting; Reduce motion (setting or Windows) swaps movement for 120 ms fades. Nothing
  animates while the island is idle and collapsed (measured idle CPU ≈ 0.3 % of one core).
- **Lenis** glides the expanded panel between modules (the modules sit side by side in a strip).
- **glisse** (Apple Liquid Glass, React) renders the island's glass body, buttons, tab bar,
  segmented control, slider and inputs through a small React bridge (`src/lib/glass/`). Its
  colours are mapped onto the tokens in `src/styles/glass.css`.
- **Pip** (`src/lib/ui/Pip.svelte`) has five moods; it bobs to music and gets dizzy when poked.
- Island states: idle pill (128 × 40), status pill (384 × 40) with a status glow, music pill
  with progress line, permission and "task finished" peeks (440 wide), expanded panel (720 × 440)
  with Home, Agent, Spotify, Clipboard, Whiteboard, Files and Note.
- **Size and position** (Settings → Appearance): scale 25–100 % (the whole island zooms, text
  stays sharp; default 85 %), collapsed and expanded width and height, horizontal position
  (0–100 % across the screen) and distance from the top. The island always keeps room to open
  fully on screen.
- The island reopens on the tab you used last (also after a restart).
- Smoothness: the pill, peek and panel are fixed-size layers; opening only resizes the outline,
  the panel is built once in the background and kept, and each tab is built on its first visit.
  Only the visible tab is drawn. Measured (release build, fresh start): opens, closes and tab
  switches stay at full frame rate (about 9 ms per frame at 120 Hz); idle uses under 1 % of one
  core. Music animations step a few times a second instead of redrawing at the screen rate.
- Permission requests can be answered from anywhere with **Ctrl+Alt+Y** (allow) and
  **Ctrl+Alt+X** (deny). Commands that can delete or overwrite work are flagged, and Deny becomes
  the main button.

## Run (development)

Requirements: Windows 10/11 with WebView2, Rust (MSVC toolchain), Node.js 20+, Claude Code
installed natively on Windows.

```bash
npm install
npm run app
```

`npm run app` builds `crest-bridge.exe`, then starts Vite and the Tauri app. The first build
takes a few minutes. Only one copy of Crest runs at a time; starting it again opens Settings.

### Install it (use this day to day)

The dev build (`npm run app`) runs unoptimised code through a live-reload server and is
noticeably slower. For daily use, build the installer:

```bash
npm run installer
```

It produces `target\releaseundle
sis\Atoll_<version>_x64-setup.exe`. Run it: Atoll is
installed for your user (no admin rights) into `%LOCALAPPDATA%\Atoll`, gets a Start menu entry
and shows up in **Settings → Apps → Installed apps** (and "Add or remove programs"), where you
can uninstall it. Running a newer setup upgrades in place and keeps your hooks and settings.
After installing, press **Settings → Claude Code → Reinstall** once so the hooks use the
installed bridge, and turn on **Start with Windows** there.

`npm run app:release` still builds a bare `target\release\crest.exe` without an installer.

## The settings dashboard

Open it with **Ctrl+Alt+,** (comma), a left click on the tray icon, the gear in the expanded
island, or by starting Crest a second time.

| Section | What it controls |
|---|---|
| General | Start with Windows, show/hide the island, reset everything |
| Appearance | Collapsed/expanded size, corner roundness, font size, outline, shadow, background colour, transparency, text colour (auto or custom), busy/done/needs-you/idle colours, which screen, gap from the top edge |
| Modules | One switch per module (modules not built yet remember your choice) |
| Alerts | Chime (volume), Windows notification, island flash; which events alert; usage warning threshold; test buttons |
| Claude Code | Follow Claude Code on/off, install/remove hooks, usage ring options and latest data, Chat tab watcher |
| Music | one-click Connect, optional account link for device switching, what-shows-in-the-pill, progress line, lyrics |
| Focus timer | Session, break and long-break lengths, no-time-limit sessions, end sound, Do not disturb rules, distraction nudge, music with focus, history (today, streak, last 7 days) |
| Hotkeys | Global hotkeys for the island and one per module |
| About | Version, file locations, privacy |

**Live preview:** every change is applied immediately to the real island *and* to the mock
island at the top of Appearance (switch its backdrop between wallpaper/dark/light to judge
transparency). Turn on **Keep the real island expanded while I edit** to tune the open look.

Settings are saved to `%APPDATA%\Crest\settings.json` (written at most every 0.4 s and on
exit). Values are clamped to safe ranges; an unreadable file is renamed to
`settings.json.broken` and defaults are used.

The dashboard's look is deliberately neutral; all its colours and radii are tokens at the top of
`src/dashboard/dashboard.css`, ready to be replaced by a design system.

## Default hotkeys

| Action | Default |
|---|---|
| Show / hide the island | Ctrl+Alt+Space |
| Open / close (pin) the island | Ctrl+Alt+. |
| Open settings | Ctrl+Alt+, |
| Per module | not set |

Change them in **Settings → Hotkeys** (click *Change*, press the combination; Esc cancels,
Backspace clears). At least one modifier is required. A combination already used by another
Crest hotkey or by another app is rejected and shown in red.

## Tray icon

Left click: open Settings. Right click: show/hide island, Settings…, Start / Pause / Resume focus
(the item follows the timer), Quit.

## How Claude Code is followed

```
Claude Code ──hook event (stdin JSON)──▶ crest-bridge.exe ──HTTP POST + token──▶ Crest (127.0.0.1:random)
            ◀──── decision JSON (stdout) ──────────────  ◀──────── response ─────────
```

- Claude Code [hooks](https://code.claude.com/docs/en/hooks) run `crest-bridge.exe` on
  `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`,
  `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop` and `StopFailure`.
- For `PermissionRequest` the listener holds the request open until you click in the island,
  then answers with the documented decision JSON (`hookSpecificOutput.decision.behavior`).
- The bridge reads `%LOCALAPPDATA%\Crest\runtime.json` (port + 256-bit token, regenerated every
  launch, deleted on exit).
- The listener binds to **127.0.0.1 only**, rejects requests without the token, and rejects any
  request carrying a browser `Origin` header.
- **Fail-open:** if Crest is not running (or Claude Code is switched off in Settings), Claude
  Code behaves exactly as without hooks.

### Permission prompts

When Claude Code would ask you for permission, the island opens by itself with the request
(full command, or the edit as `-`/`+` lines) and three buttons:

| Button | Effect |
|---|---|
| **Allow** | Claude Code runs the tool. Your deny/ask rules in settings still apply. |
| **Deny** | Claude Code skips the tool and is told you denied it. |
| **Answer in Claude** | Crest steps aside; Claude Code asks you in its own prompt. |

Several requests queue up ("1 of 3"). A request goes back to Claude Code when nobody answers
within **120 s**, Claude Code cancels the hook, the session moves on, Crest closes, or Claude
Code is switched off in Settings. Nothing appears in `bypassPermissions` / `dontAsk` mode or for
tools your rules already allow.

### Alerts

| Event | Alert | Notes |
|---|---|---|
| `Stop` | **Done** (rising chime, done colour) | Shows the start of Claude's last message |
| `PermissionRequest`, `Notification` | **Needs you** (two taps, amber) | Claude Code's own "waiting for permission" reminder ~6 s later is merged into the same alert; the 60 s "idle" reminder is ignored |
| `StopFailure` | **Error** (falling chime, red) | Rate limit, sign-in, API errors |
| Usage ≥ threshold | **Usage** | Once per 5-hour window (default 90%) |

The same alert for the same session is not repeated within 30 s. Each part (chime, Windows
notification, island flash) and each trigger has its own switch in **Settings → Alerts**, with
test buttons. Chimes are generated by Crest (no audio files). Notifications appear as "Crest":
Crest registers its own notification identity per user under
`HKCU\Software\Classes\AppUserModelId\local.crest.island`. If Windows **Do Not Disturb** is on,
notifications go straight to the Notification Centre without a banner.

### Usage ring

Claude Code's [status line](https://code.claude.com/docs/en/statusline) receives your
`rate_limits` (5-hour and weekly `used_percentage` + `resets_at`). **Install hooks / Reinstall**
also sets Crest's status line (`crest-bridge.exe statusline`, refreshed every 60 s), which:
- forwards the limits to the island (ring in the pill, details at the bottom of the open island;
  amber from 70%, red from 90%),
- prints `Opus · ctx 34% · 5h 23% · wk 41%` at the bottom of Claude Code (works even when Crest is
  closed).

If you already have your own status line, Crest leaves it alone and the ring stays empty.
Limits are only reported for Pro, Max and Ultra plans, only after the first reply of a session,
and only while a Claude Code session is running; windows disappear once their reset time passes.

### Chat tab watcher (experimental, off by default)

The Claude desktop **Chat** tab has no hooks, and its page content is not visible to Windows UI
Automation (Electron only exposes it while a screen reader runs). Crest therefore listens for the
public Windows "taskbar button flashing" signal (`RegisterShellHookWindow` / `HSHELL_FLASH`) from
`claude.exe` and alerts "Claude replied". Signals within 10 s of a Code tab hook event are ignored
(hooks already covered those). **[Unverified]** whether the Claude app flashes its taskbar button
when a chat reply finishes: check **Settings → Claude Code → Last signal from the Claude app**
after one chat with the Claude window in the background. If it stays "never", this signal is not
available and the watcher cannot work.

## Music (Spotify)

**Connect is one click** (island → Spotify tab, or Settings → Spotify). No sign-in, no developer
app, Free or Premium. Atoll reads and controls the **Spotify app on this PC** through Windows'
media session ([GlobalSystemMediaTransportControlsSessionManager](https://learn.microsoft.com/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager)),
the same source the volume flyout and keyboard media keys use. Nothing is sent anywhere for this.

- Shows title, artist, album, cover, play state and position; play/pause, previous, next and
  click-to-seek go straight to the Spotify app. Checked against the real app: Windows reports
  `can seek = true` for Spotify, and the position tracks playback to within a few ms.
- Works with the installer build and the Microsoft Store build. The Spotify website in a browser
  isn't picked up (it isn't the Spotify app).
- Reading is local, once a second while playing (2 s otherwise), on its own thread; the island
  only gets an update when something changes (new track, play/pause, a seek).
- The island: what's playing shows in the collapsed pill, with a thin progress line along the
  bottom. Settings → Spotify chooses what the pill says (**Song · artist** or **Lyrics**: the
  synced line being sung, falling back to the song name between lines and when there are no
  synced lyrics) and who gets the pill **while Claude Code works** (Claude Code by default, or
  Spotify). "Needs you" and errors always take the pill, and the permission and finished-task
  pop-ups still appear. The **Music** tab has album art, play/pause/previous/next,
  click-to-seek, the device menu and **synced lyrics** that follow the song.
- Lyrics come from [LRCLIB](https://lrclib.net/docs) (free, no key). It receives title, artist,
  album and length only; lyrics are kept in memory, never written to disk. [Unverified] LRCLIB
  documents no licence for its data, so this is for private use.
- The Spotify module hotkey (Settings → Hotkeys) toggles play/pause.

### Other devices (optional)

Moving playback to your phone, a speaker or another computer is the one thing Windows can't do.
Spotify allows it only through its Web API with a developer app of your own, and since March 2026
development-mode apps need Premium and allow 5 users
([Spotify's announcement](https://developer.spotify.com/blog/2026-02-06-update-on-developer-access-and-platform-security)).
Products that connect in one click with device switching use their own Spotify app approved for
"extended quota", which Spotify now gives only to registered businesses with 250,000+ monthly users.

If you want it: Settings → Spotify → **Other devices** → follow the steps (create an app at
[developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), redirect URI
`http://127.0.0.1:43117/spotify/callback`, tick Web API, paste the Client ID) → **Link account**.
- Sign-in is Authorization Code with **PKCE** (no client secret). Atoll opens port 43117 on
  127.0.0.1 only while you sign in (max 10 minutes); the callback is only accepted with the
  single-use random `state` Atoll generated.
- The refresh token is stored in **Windows Credential Manager** (generic credential "Crest" /
  `spotify-refresh-token`); access tokens stay in memory.
- The Web API is called only when you open the Music tab (device list) or pick a device, never on
  a timer, so the old rate-limit waits (HTTP 429) no longer block the island. If a playback command
  fails locally (Spotify closed here), it is retried through the linked account.

## Clipboard, whiteboard, file tray, quick note

Settings live in **Settings → Clipboard & files**. Each has a hotkey slot in Settings → Hotkeys:
pressing it opens the island on that tab, pressing it again (or Esc, or clicking elsewhere)
closes it. Clipboard and Note take keyboard focus so you can type straight away.

**Clipboard history** (`Clipboard` tab)
- Crest listens with `AddClipboardFormatListener` and keeps text, images (saved as PNG) and
  copied files (paths only), newest first. Search, pin, remove, clear.
- Click an item to **paste** it into the app you were using (Crest remembers the window you were
  in when the cursor reached the island, brings it back and sends Ctrl+V). Shift+click copies
  only. "Click to paste" can be switched off.
- Copies that password managers mark as private (`ExcludeClipboardContentFromMonitorProcessing`,
  `CanIncludeInClipboardHistory = 0`, `Clipboard Viewer Ignore`) are never recorded.
- Off by default: history is wiped when Crest starts. Pinned items are always kept. "Keep after
  restart" saves history (unencrypted) in `%LOCALAPPDATA%\Crest\clipboard`.

**Whiteboard** (`Board` tab), a freeform canvas saved in `%LOCALAPPDATA%\Crest\board`
- Add: drop files from Explorer onto the island, **Paste** (what's on the clipboard),
  **Add files…** (file picker), **+ Text** or double-click the canvas, or turn on
  "Add every copy to the board" so each Ctrl+C lands there.
- Images are copied into the board (they survive the original moving). Other files are linked,
  never copied or moved; the card says "missing" if the file is gone.
- Click a card to paste it (Shift+click: copy only). Drag to move, corner to resize, double-click
  text to edit. **Drag a card past the canvas edge** to drop it into another app as a real file
  (text cards arrive as a `.txt`). Drag empty space or scroll to pan.

**File tray** (`Files` tab): drop files on the island, then click to open, drag back out into
any app, copy (paste into Explorer, chat apps…), show in folder, or remove. Only paths are kept
(`%LOCALAPPDATA%\Crest\tray.json`).

**Quick note** (`Note` tab): one plain-text note, saved as you type to
`%LOCALAPPDATA%\Crest\note.txt`.

Dropping files on the island while another tab is open shows two zones: whiteboard or file tray.

## Focus timer

Start a session from the island's **Focus** tab, with **Ctrl+Alt+F** (start, pause, resume) or
from the tray. Settings live in **Settings → Focus timer**.

- **Sessions:** presets 25 / 5, 50 / 10 and 90 / 15 (focus / break minutes), your own lengths,
  or **No limit** (the timer counts up until you press *Take a break*). After every 4th session
  (your choice, 2 to 8) the break is a long one. An optional one-line intention ("finish the
  checkout tests") shows in the collapsed island while you work.
- **Collapsed island:** a small ring, the time left and "Focus" (or "Break", "Paused"). The timer
  takes the collapsed island over music and Claude's Busy / Done; "Needs you" always wins, and
  errors win unless you hold or silence them.
- **Do not disturb** (on by default) decides what Atoll's own alerts do during a focus session:
  - *Finished tasks* (also chat replies and usage warnings): **hold for the break** (default),
    show, or silent. Held ones are not lost: when the break starts, a "While you focused" pop-up
    lists them, and the Focus tab keeps them until you dismiss them.
  - *Errors*: show (default), hold or silent.
  - *Permission requests* always appear (Claude can't go on without you): **quietly** (no sound
    or notification, default) or with sound.
  - Apps can't switch Windows' own Do not disturb, so the page has a button that opens its
    settings for other apps' notifications.
- **End of a session:** the sound from Settings → Focus timer (the same as "Timer end" in
  Alerts), then the break starts with its own countdown. *Skip break* or *Start* goes straight to
  the next round.
- **Distraction nudge** (off by default): list programs or words in window titles (YouTube,
  Discord…). When one comes to the front during focus, the collapsed island says
  "YouTube · back to focus?" for a few seconds and the session counts it. It checks the window in
  front every 1.5 s, on this PC only, and never blocks anything.
- **Music with focus** (off by default): plays Spotify on this PC when a session starts and
  pauses it at the break (needs Spotify connected).
- **History:** today's focus time and sessions, your streak (days with at least one session of
  5 minutes or more) and a 7-day chart, kept in `%LOCALAPPDATA%\Crest\focus\history.json` on
  this PC only. Clear it in Settings → Focus timer, or with Clear all data.

## Project layout

```
bridge/                    crest-bridge.exe (hook command; tiny, no TLS)
src-tauri/src/
  lib.rs                   startup wiring, shared state, commands for the UI, tray
  settings.rs              settings model, clamping, load/save (unit tested)
  windows.rs               island placement, settings window
  backdrop.rs              rounded Windows blur under the island (unit tested, probe test)
  shortcuts.rs             global hotkeys (unit tested)
  server.rs                127.0.0.1 listener + token check
  sessions.rs              hook events -> per-session status/activity (unit tested)
  permissions.rs           pending Allow/Deny requests (unit tested)
  alerts.rs                chime (synthesised WAV), toast, flash, dedupe (unit tested)
  usage.rs                 5-hour / weekly limits from the status line (unit tested)
  chatwatch.rs             experimental Chat tab watcher (taskbar-flash signal)
  spotify.rs               Spotify controls, devices, optional account link (PKCE) (unit tested)
  spotify_local.rs         Spotify on this PC via the Windows media session (unit tested)
  lyrics.rs                LRCLIB synced lyrics + LRC parser (unit tested)
  hooks.rs                 install/remove hooks in ~/.claude/settings.json (unit tested)
  clipboard.rs             clipboard listener + history store
  winclip.rs               Win32 clipboard read/write: text, files, PNG/DIB (unit tested)
  paste.rs                 remember the previous window, paste into it (SendInput Ctrl+V)
  board.rs                 whiteboard cards
  tray.rs                  file tray + quick note
  focus.rs                 focus / break timer, held alerts, distraction nudge, history (unit tested)
  media.rs                 images, thumbnails, JSON stores, drag-out (drag crate)
  paths.rs                 file locations
src/                       island page (App.svelte) + shared lib/
  lib/theme.ts             appearance settings -> CSS variables (island and preview)
  lib/IslandFrame.svelte   the island shape, shared by island and preview
src/dashboard/             settings window (Dashboard.svelte, sections/, controls/)
index.html, dashboard.html the two pages
```

Run the Rust unit tests with `cargo test -p crest`.

## Troubleshooting

Send a fake event through the bridge by hand (PowerShell, with Crest running):

```powershell
$env:CREST_DEBUG = "1"
'{"session_id":"t1","cwd":"C:\\work\\demo","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"C:\\work\\demo\\a.rs"}}' | & "$env:LOCALAPPDATA\Crest\bin\crest-bridge.exe"
```

The island should show `demo · Reading a.rs`. With `CREST_DEBUG=1` the bridge prints why it gave
up (island not running, bad JSON, connection refused...). Without it, it is always silent. Use
`claude --debug` to see Claude Code's own hook log. The dev console prints
`[crest] permission #N: …` for every decision and hotkey registration problems.

## Uninstall

Settings → Apps → Installed apps → **Atoll** → Uninstall. The uninstaller removes Atoll's hooks
and status line from `%USERPROFILE%\.claude\settings.json` (other entries are kept, and a
backup `settings.json.crest-backup-<timestamp>` is written next to it), Start with Windows, the
toast registration and the installed bridge. Tick **Delete the application data** to also remove
`%LOCALAPPDATA%\Crest` and `%APPDATA%\Crest` (settings, clipboard, whiteboard, history). Saved
sign-ins (Spotify, Google) stay in Windows Credential Manager under "Crest".

Without the installer: Settings → Claude Code → **Remove**, turn off Start with Windows, then
delete `%LOCALAPPDATA%\Crest`, `%APPDATA%\Crest` and the registry key
`HKCU\Software\Classes\AppUserModelId\local.crest.island`.

## Known limits

- Hooks load when a session starts: restart running Claude Code sessions after installing.
  **After upgrading, press Settings → Claude Code → Reinstall** (Milestone 2 adds `StopFailure`
  and the status line).
- [Unverified] Whether the status line runs inside the Claude desktop Code tab; the usage ring is
  fed by terminal sessions for sure.
- Clicking a Crest notification does nothing yet.
- In a transparent window, CSS backdrop blur (and glisse's refraction) only sample Atoll's own
  page, never the desktop. So Windows draws the blur (`backdrop.rs`): a click-through helper
  window under the island holds one visual with Windows' host-backdrop brush (the blurred
  desktop behind it), clipped to an anti-aliased rounded rectangle that follows the island as
  it opens and closes. (A first try cut a window to the island's shape instead; Windows
  ignores a window's shape once an accent blur is on, so that showed as a square box.) It
  needs Windows 11, shows while Appearance → Blur is above 0, the island isn't 100 % opaque
  and Windows' Transparency effects are on. It blurs and tints what's behind; it can't bend
  (refract) it. Peeks are always fully opaque. `probe_rounded_blur` (an ignored test) checks
  it on a PC.
- The Settings window still has the old look; its redesign is the next stage.
- "Click to paste" types Ctrl+V into the window you were in before reaching the island. Apps
  running as administrator ignore input from a normal app (Windows rule), so paste by hand there.
- Dragging *text* out of the whiteboard gives a `.txt` file (the drag library only drags files).
  Use click-to-paste for plain text.
- Dragging text or links from a browser *onto* the island isn't supported (only files); use Ctrl+C
  then the board's Paste button.
- Spotify: after moving playback to another device, the island still shows the Spotify app on
  this PC (paused). It can't show what plays on other devices without polling Spotify's servers.
- A focus session or break in progress isn't kept if Atoll quits or restarts (the history is).
- Do not disturb covers Atoll's own sounds, notifications and pop-ups only; Windows has no API
  that lets an app switch its own Do not disturb.
- Claude Code holds back hooks until you accept the workspace-trust dialog for a folder.
- [Unverified] Whether Claude Code also shows its own prompt *while* the island is waiting.
- Questions Claude asks you (multiple choice) can't be answered from the island; it only alerts.
- The island is always top centre; it can be moved to another screen in Settings.
- Start with Windows registers the exe you are running. From a dev build that is
  `target\debug\crest.exe` (the dashboard warns about this).
- A Settings window opened from a dev build can take a few seconds to appear (Vite compiles the
  page on first load).
- The project folder is inside OneDrive; `target/` and `node_modules/` are large. Consider
  excluding them from sync.
