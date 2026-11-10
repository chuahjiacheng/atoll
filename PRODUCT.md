# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(A Tauri 2 desktop app for Windows 10/11. Both windows are web views rendered with Svelte 5, so the design language is web, shaped to sit naturally on the Windows desktop.)

## Users

The owner: one person on their own Windows PC who runs Claude Code sessions (terminal and the Claude desktop Code tab) while studying and working in other apps. They want to know what Claude is doing, answer it, and reach their music, copies, files, notes and focus timer without switching windows.

It is private for now and may be shared with friends or classmates later, so nothing may assume a single hard-coded user, folder or account.

## Product Purpose

Atoll is a floating island at the top centre of the screen: a calm, all-in-one hub that is glanced at and dismissed. Collapsed, it is a small pill that shows the one thing that matters right now. Expanded, it holds every module side by side.

Success: the user keeps working in another app and still never misses a permission request or finished task, and reaches any module in one hover or one hotkey.

## Positioning

One quiet place for an AI coding agent and the small everyday tools around it. Agent state, permission requests (answered from the island, including a typed reply), music, clipboard, whiteboard, files, notes and focus all live in the same island, with agent-aware behaviour across them: for example, focus holds back "done" alerts but still lets permission requests through.

## Operating Context

- Lives above every other window, top centre, as a floating pill, and never steals focus unless the user opens it.
- Hover or hotkey expands it; leaving collapses it. Peeks drop in for permission requests and finished tasks.
- A separate Settings window (sidebar of pages, live island preview) controls everything; changes apply live.
- Claude Code is followed through its hooks and status line; the experimental Chat tab watcher uses Windows taskbar-flash signals.
- Runs from the tray, starts with Windows if asked.

## Capabilities and Constraints

- Modules, each with its own switch and tab: Home, Agent (Claude Code only, no Codex), Spotify (Premium; controls, progress line, devices, synced lyrics), Clipboard history, Whiteboard, File tray, Quick note, Focus timer, Mascot (Pip).
- Not included: Codex, a general media-player module, other languages (English only).
- Island position: top centre only. Floating pill shape, not attached to the screen edge.
- Settings also cover: pause and encrypt clipboard history; export, import and clear all data; per-event sounds; hotkeys; quit and log folder.
- Private use: no accounts, licence keys, payment, telemetry or auto-update.
- Secrets (Spotify refresh token, future Gemini key) live only in Windows Credential Manager.
- Local listeners bind 127.0.0.1 only; anything that approves an action needs a random token.
- Unverifiable claims in the UI or docs are labelled [Inference] or [Unverified].
- Original code, name and artwork only; nothing copied from the reference app or other projects.

## Brand Commitments

- Name: **Atoll**. Mascot: **Pip** (moods: idle, focused, curious, happy, dizzy).
- The design in `design/` (two clickable prototypes and `design/designtokens.md`) is the binding visual source.
- Voice: plain, short, calm English. Say what happened and what to do next.

## Evidence on Hand

- `design/Clickable prototype-html/`: island states, desktop scenes, mascot moods, icon set.
- `design/Clickable prototype · 960 × 640-html/`: the Settings window, 12 pages.
- `design/designtokens.md`: colours, type, radii, spacing, sizes, motion.
- No testimonials, users or metrics exist; none may be invented.

## Product Principles

1. **Calm by default.** The island stays small and quiet; it grows only when something needs the user or they reach for it.
2. **Equal tools, one place.** Every module is a first-class part of the hub; none is buried.
3. **Never lose the agent.** Whatever else is showing, a waiting permission request or failed task always gets through.
4. **Glance, act, dismiss.** Every common action finishes in one or two clicks or one hotkey, without leaving the current app.
5. **The user's data stays on the PC** unless a feature they turned on needs to send it (Spotify, lyrics lookup, Gemini).

## Accessibility & Inclusion

Follow the design's own rules: text at least 4.5:1 contrast (custom colours below that are refused), full keyboard use with visible focus rings, a Reduce motion setting that swaps movement for short fades, and an animation speed setting.
