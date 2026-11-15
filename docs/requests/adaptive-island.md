# Requests from adaptive-island

Changes this branch needs in files it does not own. Each has a stub in place, so nothing is blocked.

## 1. DESIGN.md: island sizes (owner of DESIGN.md)

DESIGN.md still lists fixed pill sizes ("idle pill 128 × 40; status pill 384 × 40; timer pill 168 × 40", and the Island Pill component section). After step 1 the pill is content-sized:

- idle: 72 px by default (limits 64 to 96);
- status, music, event: content, at least 160;
- timer: content, at least 120;
- the Collapsed width setting (384 by default) is the widest the pill gets, not its fixed width.

Please update those lines, or tell me to. Until then DESIGN.md and the island disagree.

## 2. docs/contracts on main (whoever merges)

`docs/contracts/width-contract.md` is at version 0.2 on this branch (changelog at the bottom of the file). `main` still has 0.1. It needs to reach `main` so the other sessions read the current one.

## 3. Settings: Collapsed width wording (owner of the Appearance settings page)

The "Collapsed width" slider now sets the widest the pill can get. Its label or help text should say so (suggested help text: "The widest the collapsed island gets. It is only as wide as what it shows.").

## 4. src/lib/theme.ts (owner of theme.ts)

`--idle-w` and `--timer-w` are no longer read by the island and can be removed. `--status-w` is still read (as the widest pill). No action needed now.

## 5. Coming in step 3 (not needed yet)

A `spotify.lyricsWidth` setting: `"follow" | "smooth" | "fixed"`, default `"smooth"`, in `settings.rs`, `src/lib/settings.ts` and the Spotify settings page. Until it exists the island behaves as `"smooth"`.
