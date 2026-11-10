// Right-click menu shared by the island tabs. A view calls `openMenu(e, items)`
// from its `oncontextmenu`; App.svelte renders the one <ContextMenu />.
import type { IconName } from "./Icon.svelte";

export interface MenuItem {
  label: string;
  icon?: IconName;
  /** Small hint on the right, e.g. "Shift+click". */
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  run: () => unknown;
}
/** `null` draws a divider. */
export type MenuEntry = MenuItem | null | false | undefined;

export const menu = $state<{ open: boolean; x: number; y: number; items: MenuEntry[] }>({ open: false, x: 0, y: 0, items: [] });

export function openMenu(e: MouseEvent, items: MenuEntry[]) {
  e.preventDefault();
  e.stopPropagation();
  menu.items = items;
  menu.x = e.clientX;
  menu.y = e.clientY;
  menu.open = true;
}

export function closeMenu() {
  menu.open = false;
}
