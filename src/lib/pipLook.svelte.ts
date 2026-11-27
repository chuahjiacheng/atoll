// Pip's look for this window, shared by every Pip on the page. The island and
// the Settings window each copy it from settings (App.svelte, Dashboard.svelte),
// so Pip.svelte doesn't need the props threaded through every view.
import type { Outfit } from "./pip";

export const pipLook = $state({
  outfit: "none" as Outfit,
  headphonesWithMusic: true,
  blink: true,
  celebrate: true,
  /** Music is playing right now (island only). */
  music: false,
});

export function copyPipLook(m: { outfit: Outfit; headphonesWithMusic: boolean; blink: boolean; celebrate: boolean }) {
  pipLook.outfit = m.outfit;
  pipLook.headphonesWithMusic = m.headphonesWithMusic;
  pipLook.blink = m.blink;
  pipLook.celebrate = m.celebrate;
}
