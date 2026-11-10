---
name: Atoll
description: A floating, calm, all-in-one island for Claude Code and everyday tools on Windows.
colors:
  lagoon-ink: "#F4F4F7"
  mist: "#A9A9B5"
  periwinkle: "#8FA3FF"
  periwinkle-text: "#A9B8FF"
  periwinkle-ink: "#0E0E12"
  signal-amber: "#F5B947"
  signal-green: "#45D68E"
  signal-coral: "#FF7A7A"
  hull: "#101015"
  hull-raised: "rgba(255,255,255,0.07)"
  hull-track: "rgba(255,255,255,0.16)"
  hull-hairline: "rgba(255,255,255,0.10)"
  hull-dash: "rgba(255,255,255,0.28)"
  hull-button: "#2A2A33"
  hull-tab: "#33333D"
  light-ink: "#141419"
  light-mist: "#55555F"
  light-periwinkle: "#3A55D6"
  light-periwinkle-text: "#3048C2"
  light-amber: "#8A4B00"
  light-green: "#0B6638"
  light-coral: "#A82020"
  light-hull: "#FCFCFD"
  light-hull-raised: "rgba(20,20,30,0.05)"
  light-hull-track: "rgba(20,20,30,0.14)"
  light-hull-button: "#E6E6EC"
  settings-window: "#101015"
  settings-sidebar: "#0B0B0F"
  settings-card: "#18181F"
  settings-field: "#0B0B0F"
  settings-line: "rgba(255,255,255,0.08)"
  settings-control: "#7A7A86"
  settings-disabled: "#6E6E79"
  settings-control-disabled: "#3A3A44"
  settings-selected: "rgba(255,255,255,0.08)"
  settings-desk: "#1C1E2B"
  light-settings-window: "#F4F4F6"
  light-settings-sidebar: "#ECECF0"
  light-settings-card: "#FFFFFF"
  light-settings-line: "#E4E4EA"
  light-settings-control: "#84848E"
  light-settings-disabled: "#9A9AA3"
  light-settings-control-disabled: "#C9C9D1"
  light-settings-desk: "#DDE2EC"
  pip-periwinkle: "#8C9CFF"
  pip-periwinkle-edge: "#6F80F0"
  pip-face: "#171A26"
  pip-blush: "#FF9DB0"
  caption-close: "#C42B1C"
typography:
  display:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "40px"
    fontWeight: 600
    lineHeight: 1.1
    fontFeature: "'tnum'"
  headline:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.3
  title:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 600
    lineHeight: 1.4
  body:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.4
  body-small:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
  label:
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 600
    lineHeight: 1.4
rounded:
  pill: "9999px"
  panel: "32px"
  peek: "28px"
  card: "16px"
  control: "12px"
  tile: "10px"
  nav: "8px"
  keycap: "6px"
  art: "20px"
spacing:
  hair: "2px"
  xxs: "4px"
  xs: "8px"
  sm: "12px"
  md: "16px"
  lg: "24px"
  xl: "32px"
  xxl: "48px"
  xxxl: "64px"
components:
  island-pill:
    backgroundColor: "{colors.hull}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.pill}"
    height: "40px"
    padding: "0 16px 0 8px"
  island-panel:
    backgroundColor: "{colors.hull}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.panel}"
    width: "720px"
    height: "440px"
    padding: "16px"
  island-peek:
    backgroundColor: "{colors.hull}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.peek}"
    width: "440px"
    padding: "16px"
  button-primary:
    backgroundColor: "{colors.periwinkle}"
    textColor: "{colors.periwinkle-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    height: "44px"
    padding: "0 16px"
  button-secondary:
    backgroundColor: "{colors.hull-button}"
    textColor: "{colors.lagoon-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    height: "44px"
    padding: "0 16px"
  button-icon:
    backgroundColor: "{colors.hull-raised}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.pill}"
    size: "40px"
  tab:
    backgroundColor: "transparent"
    textColor: "{colors.mist}"
    typography: "{typography.body-small}"
    rounded: "{rounded.pill}"
    height: "36px"
  tab-active:
    backgroundColor: "{colors.hull-tab}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.pill}"
    height: "36px"
    padding: "0 16px 0 12px"
  card:
    backgroundColor: "{colors.hull-raised}"
    rounded: "{rounded.card}"
    padding: "16px"
  field:
    backgroundColor: "{colors.settings-field}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.control}"
    height: "40px"
    padding: "0 12px"
  settings-nav-item:
    backgroundColor: "transparent"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.nav}"
    height: "40px"
    padding: "0 12px 0 16px"
  settings-nav-item-active:
    backgroundColor: "{colors.settings-selected}"
    textColor: "{colors.lagoon-ink}"
    rounded: "{rounded.nav}"
    height: "40px"
  settings-card:
    backgroundColor: "{colors.settings-card}"
    rounded: "{rounded.card}"
    padding: "16px"
---

# Design System: Atoll

<!-- Source of truth: design/ (two clickable prototypes) and design/designtokens.md. Values not in the token list are taken from the prototypes and marked "from the prototype". -->

## Overview

**Creative North Star: "The Quiet Lagoon"**

Atoll is a calm, sheltered pool that floats just below the top edge of the screen. Its surface is smoked glass: a near-black hull (dark) or a near-white hull (light) at adjustable opacity, blurring whatever lies behind it. At rest it is a small, fully rounded pill with Pip the mascot and a single dot. It never touches the screen edge. When something happens, the water lights up: an amber, green or coral ring glows around the pill and one short line says what changed. When the user reaches for it, the pill grows smoothly into a rounded panel holding every module side by side.

The world is restrained. Colour is a signal, never decoration: periwinkle marks what is selected or actionable, amber means an agent is working, green means done, coral means something failed. Everything else is ink on glass. Density is comfortable rather than compact: 40 px rows, 44 px buttons, generous 16 px insets. It is a tool glanced at between other work, so it must read in a single look and then get out of the way.

The Settings window is the lagoon's shore: the same ink, accent and status colours on solid, opaque surfaces (no glass), with a sidebar of pages and a live miniature of the island at the top of Appearance.

**Key Characteristics:**
- Floating, fully rounded pill that grows into a 32 px-cornered panel; never attached to the screen edge (8 px gap from the top, from the prototype).
- Smoked-glass surfaces with blur(24px) saturate(140%), a 1 px inner hairline and a top highlight.
- Status shown by a coloured glow ring plus icon and word, never by colour alone.
- Inter at only two weights (400, 600) and six sizes.
- Pip appears in every collapsed state and reflects the agent's mood.
- Dark and light themes, plus Follow Windows.

## Colors

Ink on smoked glass, with one cool accent and three signal colours that only appear when something happens.

### Primary
- **Periwinkle** (`periwinkle`, light: `light-periwinkle`): the one accent. Selected tab, primary button, progress fills, sliders, switches when on, focus rings, the permission-request glow. Text-on-accent uses `periwinkle-ink` (dark) or white (light).
- **Periwinkle Text** (`periwinkle-text`, light: `light-periwinkle-text`): accent-coloured words on glass (for example "Permission needed"), lightened or darkened so they pass 4.5:1.

### Secondary (signals)
- **Signal Amber** (`signal-amber`, light: `light-amber`): an agent is busy. Glow ring, busy icon and the word "Busy".
- **Signal Green** (`signal-green`, light: `light-green`): done. Glow ring, check icon, "Done".
- **Signal Coral** (`signal-coral`, light: `light-coral`): error. Glow ring, alert icon, "Error"; also destructive buttons in Settings.

The light-theme signal colours are darker than the dark-theme ones (from the prototype; the token list says "the status colours above", but the prototype darkens them so they reach 4.5:1 on a light hull).

### Neutral
- **Lagoon Ink** (`lagoon-ink`, light: `light-ink`): all primary text and icons.
- **Mist** (`mist`, light: `light-mist`): secondary text, metadata, inactive tab icons, the idle dot.
- **Hull** (`hull`, light: `light-hull`): the island glass tint, applied at the user's opacity.
- **Hull Raised / Track / Hairline / Dash / Button / Tab**: cards inside the panel, progress tracks, the shell's inner hairline, dashed drop areas, secondary buttons and the active tab chip.
- **Settings Window / Sidebar / Card / Field / Line / Control / Desk**: the opaque Settings surfaces, card borders, control outlines, and the miniature desktop behind the island preview.

### Named Rules
**The Signal-Only Rule.** Amber, green and coral appear only to report a real state change. Never use them as decoration, illustration colour or brand colour.
**The Never-Colour-Alone Rule.** Every status colour is paired with an icon and a word ("Busy", "Done", "Error").
**The 4.5 Rule.** Text and status words must reach 4.5:1 on the hull at the current opacity. A custom colour below that is refused with an explanation, never silently applied.
**The Low-Tide Rule.** Below 85 % opacity the island adds tinted plates behind headers, cards and buttons, and secondary text switches from Mist to Ink, so legibility never depends on the wallpaper. A collapsed pill that shows text never drops below 85 % opacity.

## Typography

**Body Font:** Inter (with 'Segoe UI', system-ui, sans-serif), bundled locally.

**Character:** One workhorse face at two weights. Hierarchy comes from size and weight steps, never from extra families, italics or letter-spacing.

### Hierarchy
- **Display** (600, 40px): the focus timer's countdown only. Tabular numbers.
- **Headline** (600, 20px): Settings page titles, the player's track title, the Home focus card time.
- **Title** (600, 16px): peek titles (two lines, then "…"), connection names in Settings.
- **Body** (400 or 600, 14px, line-height 1.4): default text, buttons, nav items, field values. Notes use line-height 1.6.
- **Body Small** (400, 13px): pill text, list rows, help text, tab labels.
- **Label** (600, 12px): status words, section headers, metadata, times.

### Named Rules
**The Two-Weight Rule.** Only 400 and 600 exist. No 500, 700, light or italic.
**The Tabular Rule.** Every number that changes over time (times, percentages, counts, sizes) uses tabular figures.

## Layout

- **Island sizes** (logical px, from the token list): idle pill 128 × 40; status pill 384 × 40; timer pill 168 × 40; peek 440 wide; expanded panel 720 × 440. Settings → Appearance lets the user change the collapsed width (240–560, idle is one third), height (32–56) and the expanded size (560–960 × 320–640).
- **Position:** horizontally centred, floating 8 px below the top of the work area (from the prototype). It keeps 16 px from every screen edge and is never split across displays.
- **Expanded panel:** 16 px padding, 8 px between rows. Row 1 is the header (40 px). Row 2 is the tab bar (36 px tabs). Then the module body fills the rest. Two-column grids use 8 px gaps (Home) or 16 px (Agent, Files).
- **Settings window:** 32 px custom title bar, 256 px sidebar (232 px in the prototype, widened so "Clipboard and whiteboard" fits), content column up to 760 px wide with 24 / 32 / 48 px padding and 24 px between sections.
- **Spacing rhythm:** 8 / 16 / 24 / 32 / 48 / 64 from the token list. The prototype also uses 2, 4, 6 and 12 px for hairline gaps inside controls (4 px between icon and word, 12 px inside setting rows).
- **Truncation (from the prototype):** file paths are cut in the middle with "…/" and keep the file name; commands are cut at the end with "…"; the collapsed pill is one line; peek titles are two lines; folder paths keep the drive and last two folders; full text appears in a tooltip and in the Agent tab.

## Elevation & Depth

Glass that glows. The island shell and peeks are blurred, semi-transparent glass lifted by a soft drop shadow, a 1 px top highlight and a 1 px inner hairline. Everything inside the shell is flat tonal layering (raised cards at 7 % white). Coloured glow is never ambient: it appears only around the shell to signal a state.

### Shadow Vocabulary
- **Shell** (`0 16px 40px rgba(0,0,0,0.45), inset 0 1px 0 rgba(255,255,255,0.08), inset 0 0 0 1px rgba(255,255,255,0.10)`; light: `0 16px 40px rgba(20,20,40,0.18), inset 0 1px 0 rgba(255,255,255,0.9), inset 0 0 0 1px rgba(20,20,30,0.10)`): the pill, panel and peeks at rest.
- **Status glow** (`0 0 0 2px <signal>, 0 0 0 3.5px <hull>, 0 0 28px 4px <signal at 55 %>` plus the shell shadow): busy, done, error, and the permission request (periwinkle).
- **Glass** (`backdrop-filter: blur(24px) saturate(140%)`; the blur amount is a user setting, 0–48 px). On the desktop the island's blur is drawn by Windows (its host-backdrop blur, clipped to the island's rounded shape), because a transparent window's CSS filter can't see the desktop; there the setting turns it on (above 0) or off.
- **Settings window** (`0 0 0 1px` frame line, `0 16px 48px rgba(0,0,0,0.5)`): the window and dialogs. Toasts use `0 8px 24px rgba(0,0,0,0.3)`.

### Named Rules
**The Glow-Means-News Rule.** A glow ring is a message. If nothing changed, nothing glows.

## Shapes

Soft, continuous curves everywhere. The collapsed island is a true pill (radius = half its height). The panel uses 32 px corners and peeks use 28 px. Cards use 16 px, controls 12 px, file and clip tiles 10 px, Settings nav items 8 px, and keycaps 6 px. Tabs, segmented-control chips and round icon buttons are fully rounded. The Corner roundness setting (0–100 %) scales the island's radii only. There are no square corners and no sharp dividers on glass: separation comes from tone, not lines.

## Components

Soft and tactile: rounded, finger-sized, with gentle press feedback that feels physical but never bounces.

### Buttons
- **Shape:** gently rounded (12px); icon buttons are 40 px circles on the island.
- **Primary:** periwinkle fill, periwinkle-ink text, 44 px tall, 16 px side padding, 600 weight. One per view.
- **Secondary:** hull-button fill, ink text, same size.
- **Danger (Settings):** transparent with a 1 px coral ring and coral text.
- **Hover / Press / Focus:** hover adds a 7 % white wash (6 % ink in light); press scales to about 0.97 with a quick, damped return; focus shows a 2 px periwinkle outline offset by 2 px. Disabled buttons use the control-disabled tones and a not-allowed cursor.

### Tabs (island)
- **Style:** a rounded tab bar on a plate; each tab is a 36 px pill showing only its 16 px icon. The active tab grows to show its label on the hull-tab chip (white in light).
- **States:** inactive icons in Mist; active in Ink.

### Cards / Containers
- **Corner Style:** 16px.
- **Background:** hull-raised on glass; settings-card in Settings, with a 1 px settings-line ring.
- **Internal Padding:** 16px; list rows 8–16px.

### Inputs / Fields
- **Style:** 40 px tall, 12 px radius, field background, 1 px control-colour border.
- **Focus:** 2 px periwinkle outline.
- **Error:** 2 px coral border plus an alert icon and a one-line fix beneath ("Enter a value from 10 to 200.").
- **Switch:** 40 × 20 track with a 12 px knob; on = periwinkle fill with ink knob, off = outlined with a Mist knob; the words "On"/"Off" sit beside it.
- **Slider:** 4 px track filled in periwinkle up to the value, 20 px light thumb that grows to 1.15× on hover, plus a 72 px number box and unit.
- **Segmented control:** 1 px outlined group, 36 px chips; the selected chip is periwinkle with a check icon and periwinkle-ink text. Segments share one width, wide enough for the longest label with its check. Labels are centred by their capitals, the check on the same centre line.
- **Keycap:** 26 px tall chips in the keycap colour, 600 weight, used in the hotkey recorder.

### Navigation (Settings)
- 40 px rows with an 18 px icon and the page name. The active row has the selected wash, 600 weight and a 3 × 16 px periwinkle indicator on the left.

### Island Pill (signature)
- **Idle:** 128 × 40. Pip (24 px, idle) on the left, a Mist 8 px dot on the right.
- **Status:** 384 × 40. Pip in the matching mood, status icon + word in the signal colour, one line of text, and time on the right; the shell glows in the signal colour.
- **Music:** 384 × 40. Art tile, title (600) · artist (Mist), an equaliser icon, and a 2 px progress line along the bottom.
- **Timer:** 168 × 40. A 24 px progress ring, the time (600, tabular) and the word "Focus".

### Peek (signature)
- 440 px wide, 28 px corners, fully opaque hull. Pip (40 px), a coloured kicker line, a title, a detail line, then two 44 px buttons (Deny / Allow, or Dismiss / View answer) with their keys (Esc / Enter) shown beneath.

### Pip (signature)
- Built from body, sprout, highlight, cheeks, eyes and mouth so each part can move on its own. Five moods: idle, focused (agent working), curious (agent asks), happy (done), dizzy (poked repeatedly or error). Four body tones: periwinkle, mint, peach, lilac.

## Do's and Don'ts

### Do:
- **Do** keep the island floating, fully rounded, horizontally centred and 8 px below the top edge.
- **Do** pair every status colour with its icon and word.
- **Do** keep text at 4.5:1 or better at every opacity, using the Low-Tide plates below 85 %.
- **Do** use tabular figures for anything that counts or ticks.
- **Do** use exactly the durations in the motion tokens, scaled by the user's animation speed, and replace movement with short fades when Reduce motion is on.

### Don't:
- **Don't** attach the island to the screen edge or square off its top corners.
- **Don't** use amber, green or coral for decoration.
- **Don't** add weights other than 400 and 600, or fonts other than Inter.
- **Don't** use bounce or elastic easing; motion settles, it does not overshoot.
- **Don't** put glass inside glass: cards, rows and controls inside the shell stay flat.
