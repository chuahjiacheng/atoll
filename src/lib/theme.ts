// Applies the Appearance settings on top of the design tokens (src/styles/tokens.css).
// Used by the island and by the Settings window (its live preview), so both
// always match. No colour or size values live here: presets are CSS variables,
// and the only literal colours are the user's own custom hex values.
import type { Appearance, ColorChoice } from "./settings";

export type Theme = "dark" | "light";
export type ColorKey = "accent" | "busy" | "done" | "error";

/** Text under 85 % opacity gets backing plates (DESIGN.md "Low-Tide Rule"). */
export const LOW_TIDE = 0.85;
/** A collapsed pill that shows text never drops below this opacity. */
export const PILL_MIN_ALPHA = 0.85;
/** WCAG AA for normal text. */
export const MIN_CONTRAST = 4.5;

const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");
const motionQuery = () => window.matchMedia("(prefers-reduced-motion: reduce)");

export function resolveTheme(a: Pick<Appearance, "theme">): Theme {
  if (a.theme === "dark" || a.theme === "light") return a.theme;
  return darkQuery().matches ? "dark" : "light";
}

export function reducedMotion(a: Pick<Appearance, "reduceMotion">): boolean {
  return a.reduceMotion || motionQuery().matches;
}

/** Call `cb` when Windows' light/dark or reduced-motion setting changes. */
export function watchSystem(cb: () => void): () => void {
  const q = [darkQuery(), motionQuery()];
  q.forEach((m) => m.addEventListener("change", cb));
  return () => q.forEach((m) => m.removeEventListener("change", cb));
}

// ---------- colour maths (WCAG) ----------

export const isHex = (v: string) => /^#[0-9a-fA-F]{6}$/.test(v.trim());

function channel(c: number) {
  const s = c / 255;
  return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

export function luminance(hex: string): number {
  const n = parseInt(hex.trim().slice(1), 16);
  return 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
}

export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** Read a token's value (e.g. "--c-hull") from an element. */
export function token(name: string, el: Element = document.documentElement): string {
  return getComputedStyle(el).getPropertyValue(name).trim();
}

/** Numeric value of a token ("40px" -> 40, "0.92" -> 0.92). */
export function px(name: string, el?: Element): number {
  return parseFloat(token(name, el)) || 0;
}

/** "#rrggbb" of the hull for a theme, read from the tokens. */
export function hullHex(theme: Theme): string {
  // Theme tokens live on :root, so switch it for one synchronous read.
  const root = document.documentElement;
  const before = root.dataset.theme;
  root.dataset.theme = theme;
  const hex = token("--c-hull");
  if (before === undefined) delete root.dataset.theme;
  else root.dataset.theme = before;
  return hex;
}

/** Is a custom colour usable on the island hull of this theme? */
export function customColorStatus(c: ColorChoice, theme: Theme): { valid: boolean; ratio: number; ok: boolean } {
  const valid = isHex(c.custom);
  const ratio = valid ? contrast(c.custom, hullHex(theme)) : 0;
  return { valid, ratio, ok: c.customOn && valid && ratio >= MIN_CONTRAST };
}

/** CSS value for an accent/status colour: the custom hex if it passes, else the preset variable. */
export function colorValue(key: ColorKey, c: ColorChoice, theme: Theme): string {
  if (c.customOn && customColorStatus(c, theme).ok) return c.custom.trim().toLowerCase();
  return `var(--preset-${key}-${c.preset})`;
}

/** Backing-plate alpha for the Low-Tide rule (0 when not needed). */
export function plateAlpha(alpha: number): number {
  return alpha < LOW_TIDE ? Math.min(1, 1 - 0.15 / (1 - alpha)) : 0;
}

/** Every runtime variable the island needs, from the settings. */
export function appearanceVars(a: Appearance, theme: Theme): Record<string, string> {
  const alpha = a.opacity / 100;
  const plate = plateAlpha(alpha);
  const vars: Record<string, string> = {
    "--pill-h": `${a.collapsedHeight}px`,
    "--status-w": `${a.collapsedWidth}px`,
    "--idle-w": `${Math.round(a.collapsedWidth / 3)}px`,
    "--timer-w": `${Math.round((px("--size-timer-w") * a.collapsedWidth) / px("--size-status-w"))}px`,
    "--panel-w": `${a.expandedWidth}px`,
    "--panel-h": `${a.expandedHeight}px`,
    "--roundness": `${a.roundness / 100}`,
    "--hull-alpha": `${alpha}`,
    "--pill-alpha": `${Math.max(alpha, PILL_MIN_ALPHA)}`,
    "--hover-alpha": `${Math.max(alpha, px("--hover-min-alpha"))}`,
    "--panel-alpha": `${Math.max(alpha, px("--panel-min-alpha"))}`,
    "--plate-alpha": `${plate}`,
    "--blur": `${a.blur}px`,
    "--speed": `${a.animationSpeed / 100}`,
    // The window keeps up to --size-top-room above the island; the rest of the gap is above the window.
    "--stage-top": `${Math.min(px("--size-top-room"), a.positionY / (a.scale / 100))}px`,
  };
  for (const key of ["accent", "busy", "done", "error"] as ColorKey[]) {
    vars[`--c-${key}`] = colorValue(key, a[key], theme);
  }
  if (a.accent.customOn && customColorStatus(a.accent, theme).ok) {
    vars["--c-accent-ink"] = luminance(a.accent.custom) > 0.3 ? "var(--ink-on-light)" : "var(--ink-on-dark)";
    vars["--c-accent-text"] = "var(--c-accent)";
  }
  return vars;
}

/**
 * Apply the theme to the document root and the appearance variables to `el`
 * (the root for the island, the preview frame in Settings).
 */
export function applyAppearance(a: Appearance, el: HTMLElement = document.documentElement): Theme {
  const theme = resolveTheme(a);
  document.documentElement.dataset.theme = theme;
  document.documentElement.dataset.reduceMotion = reducedMotion(a) ? "true" : "false";
  // Remove old overrides first so a turned-off custom colour falls back to its preset.
  for (const k of ["--c-accent-ink", "--c-accent-text"]) el.style.removeProperty(k);
  for (const [k, v] of Object.entries(appearanceVars(a, theme))) el.style.setProperty(k, v);
  el.dataset.lowTide = a.opacity / 100 < LOW_TIDE ? "true" : "false";
  return theme;
}
