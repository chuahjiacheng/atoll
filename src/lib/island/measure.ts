// Text measurement for the width contract (docs/contracts/width-contract.md §6).
//
// Widths are cached by type style + string. Nothing is trusted before the
// fonts have loaded, and the cache is dropped whenever a result could have
// changed: a font (or one more of Inter's script subsets) finishes loading,
// the display scale changes, the island moves to another monitor, or the
// type settings change.
//
// Two ways of measuring sit behind one interface:
// - canvas: no layout at all. Used for plain text.
// - a hidden element: lays the string out exactly as the pill will, so it is
//   right by construction. Used for tabular numbers (canvas has no
//   tabular-nums) and for scripts where shaping and font fallback matter
//   (Chinese, Japanese, Korean, Arabic, Hebrew and other non-Latin text).
//   The host is `contain: strict`, so that layout never touches the page.
import type { Measurer, TextStyle } from "./width";

interface Face {
  size: string;
  weight: string;
  tabular: boolean;
}

const STYLES: Record<TextStyle, Face> = {
  label: { size: "--fs-label", weight: "--fw-regular", tabular: false },
  labelStrong: { size: "--fs-label", weight: "--fw-strong", tabular: false },
  small: { size: "--fs-small", weight: "--fw-regular", tabular: false },
  smallStrong: { size: "--fs-small", weight: "--fw-strong", tabular: false },
  body: { size: "--fs-body", weight: "--fw-regular", tabular: false },
  bodyStrong: { size: "--fs-body", weight: "--fw-strong", tabular: false },
  tabular: { size: "--fs-label", weight: "--fw-regular", tabular: true },
  tabularStrong: { size: "--fs-body", weight: "--fw-strong", tabular: true },
};

/** Basic Latin, Latin-1, Latin Extended, general punctuation: what Inter itself draws (Malay included). */
const PLAIN = /^[ -ɏ‐-‧‰-⁞₠-₿]*$/;
const CACHE_LIMIT = 600;

export type Backend = "canvas" | "element";

class TextMeasurer implements Measurer {
  ready = false;
  /** Cache statistics, for the dev overlay and tests. */
  hits = 0;
  misses = 0;
  private cache = new Map<string, number>();
  private fonts = new Map<TextStyle, string>();
  private tokens = new Map<string, number>();
  private ctx: CanvasRenderingContext2D | null = null;
  private host: HTMLDivElement | null = null;
  private probe: HTMLSpanElement | null = null;
  private listeners = new Set<() => void>();
  private stopScale: (() => void) | null = null;

  /** Load the faces the pill uses, then start answering. Safe to call twice. */
  async start(): Promise<void> {
    if (this.ready || typeof document === "undefined") return;
    const family = this.family();
    const sizes = new Set(Object.values(STYLES).map((f) => `${this.raw(f.weight)} ${this.raw(f.size)} ${family}`));
    await Promise.all([...sizes].map((f) => document.fonts.load(f).catch(() => [])));
    await document.fonts.ready;
    this.ready = true;
    // A script subset arriving later (first Vietnamese or Cyrillic string, say) changes widths.
    document.fonts.addEventListener("loadingdone", this.clear);
    this.watchScale();
    this.clear();
  }

  /** Called when widths may have changed (cache dropped): resolve every state again. */
  onChange(cb: () => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  /** Drop everything measured so far (fonts, scale, monitor or type settings changed). */
  clear = () => {
    this.cache.clear();
    this.fonts.clear();
    this.tokens.clear();
    this.listeners.forEach((cb) => cb());
  };

  text(value: string, style: TextStyle): number {
    if (!value) return 0;
    const key = this.key(value, style);
    const cached = this.cache.get(key);
    if (cached !== undefined) {
      this.hits++;
      return cached;
    }
    this.misses++;
    const width = this.measure(value, style, this.backendFor(value, style));
    if (this.cache.size >= CACHE_LIMIT) this.cache.clear();
    this.cache.set(key, width);
    return width;
  }

  token(name: string): number {
    let v = this.tokens.get(name);
    if (v === undefined) {
      v = parseFloat(this.raw(name)) || 0;
      this.tokens.set(name, v);
    }
    return v;
  }

  backendFor(value: string, style: TextStyle): Backend {
    return STYLES[style].tabular || !PLAIN.test(value) ? "element" : "canvas";
  }

  /** Measure without the cache, by a chosen method (tests compare the two). */
  measure(value: string, style: TextStyle, backend: Backend): number {
    return backend === "canvas" ? this.byCanvas(value, style) : this.byElement(value, style);
  }

  get cacheSize() {
    return this.cache.size;
  }

  // Tabular digits are all one width, so "1:05" and "2:47" share a cache entry
  // and a ticking clock never measures again.
  private key(value: string, style: TextStyle) {
    return `${style}\n${STYLES[style].tabular ? value.replace(/\d/g, "0") : value}`;
  }

  private raw(name: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  private family() {
    return this.raw("--font") || "sans-serif";
  }

  private font(style: TextStyle) {
    let f = this.fonts.get(style);
    if (!f) {
      const face = STYLES[style];
      f = `${this.raw(face.weight) || "400"} ${this.raw(face.size) || "13px"} ${this.family()}`;
      this.fonts.set(style, f);
    }
    return f;
  }

  private byCanvas(value: string, style: TextStyle) {
    this.ctx ??= document.createElement("canvas").getContext("2d");
    if (!this.ctx) return this.byElement(value, style);
    this.ctx.font = this.font(style);
    return this.ctx.measureText(value).width;
  }

  private byElement(value: string, style: TextStyle) {
    if (!this.probe) {
      this.host = document.createElement("div");
      this.host.setAttribute("aria-hidden", "true");
      // Zero-sized and strictly contained: laying the probe out cannot disturb the page.
      this.host.style.cssText = "position:fixed;left:0;top:0;width:0;height:0;overflow:hidden;contain:strict;visibility:hidden;pointer-events:none";
      this.probe = document.createElement("span");
      this.probe.style.cssText = "position:absolute;white-space:pre;unicode-bidi:plaintext";
      this.host.append(this.probe);
      document.body.append(this.host);
    }
    const p = this.probe;
    p.style.font = this.font(style);
    p.style.fontVariantNumeric = STYLES[style].tabular ? "tabular-nums" : "normal";
    p.textContent = value;
    return p.getBoundingClientRect().width;
  }

  private watchScale() {
    this.stopScale?.();
    const query = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
    const changed = () => {
      this.clear();
      this.watchScale(); // the query is tied to the old scale: watch the new one
    };
    query.addEventListener("change", changed, { once: true });
    this.stopScale = () => query.removeEventListener("change", changed);
  }
}

/** The one measurer the island uses. */
export const measurer = new TextMeasurer();
