// Motion: GSAP set up from the design's motion tokens.
// Durations come from tokens.css, are divided by the user's animation speed,
// and collapse to a short fade when Reduce motion is on (setting or Windows).
import gsap from "gsap";
import { CustomEase } from "gsap/CustomEase";
import { Flip } from "gsap/Flip";
import { token } from "./theme";

gsap.registerPlugin(CustomEase, Flip);

export { gsap, Flip };

export type Duration = "expand" | "peek" | "collapse" | "fast" | "slow" | "lyric";

let settle: string | null = null;

/** The design's settling curve (no overshoot), registered once from the token. */
export function ease(): string {
  if (!settle) {
    const nums = token("--ease-settle").match(/[\d.]+/g)?.join(",") ?? "0.2,0.8,0.2,1";
    CustomEase.create("settle", nums);
    settle = "settle";
  }
  return settle;
}

export function reduced(): boolean {
  return document.documentElement.dataset.reduceMotion === "true";
}

/**
 * A CSS time ("1200ms", "1.2s", ".2s") in seconds. The production CSS minifier
 * rewrites "1200ms" as "1.2s", so the unit must be read, never assumed.
 */
export function toSeconds(value: string): number {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return 0;
  return /ms\s*$/i.test(value) ? n / 1000 : /s\s*$/i.test(value) ? n : n / 1000;
}

/** A duration token in seconds, not scaled (loops, holds). */
export function seconds(name: string, el?: Element): number {
  return toSeconds(token(name, el));
}

/** Seconds for a motion token, scaled by the user's animation speed. */
export function dur(name: Duration): number {
  if (reduced()) return seconds("--dur-reduced");
  const speed = parseFloat(token("--speed")) || 1;
  return seconds(`--dur-${name}`) / speed;
}

/** Press feedback (pointer-down, instant): scale to the token and back. */
export function press(el: Element) {
  if (reduced()) return;
  const to = parseFloat(token("--press-scale")) || 0.97;
  gsap.to(el, { scale: to, duration: dur("fast") / 2, ease: ease(), overwrite: "auto" });
}

export function release(el: Element) {
  gsap.to(el, { scale: 1, duration: dur("fast"), ease: ease(), overwrite: "auto" });
}

/** Svelte action: press feedback on any button. `use:pressable` */
export function pressable(el: HTMLElement) {
  const down = () => press(el);
  const up = () => release(el);
  el.addEventListener("pointerdown", down);
  el.addEventListener("pointerup", up);
  el.addEventListener("pointerleave", up);
  el.addEventListener("pointercancel", up);
  return {
    destroy() {
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointerleave", up);
      el.removeEventListener("pointercancel", up);
      gsap.killTweensOf(el);
    },
  };
}

/** Fade + small rise for content arriving (reduced: fade only). */
export function enter(targets: gsap.TweenTarget, delay = 0) {
  return gsap.fromTo(
    targets,
    { autoAlpha: 0, y: reduced() ? 0 : -4 },
    { autoAlpha: 1, y: 0, duration: dur("slow"), delay, ease: ease(), clearProps: "transform" },
  );
}
