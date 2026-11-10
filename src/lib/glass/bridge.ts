// Bridge for glisse (Apple Liquid Glass components, React only) inside Svelte.
// `use:react={{ component, props }}` renders a React component into an element
// and re-renders it when the props change. Each mount is its own small React root.
import { createElement, type ComponentType, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ICONS, type IconName } from "../ui/Icon.svelte";

export interface ReactParams<P> {
  component: ComponentType<P>;
  props: P;
}

export function react<P extends object>(node: HTMLElement, params: ReactParams<P>) {
  const root: Root = createRoot(node);
  // Normal (batched) rendering: synchronous renders of many controls blocked the frame.
  root.render(createElement(params.component as ComponentType<object>, params.props));
  return {
    update(next: ReactParams<P>) {
      root.render(createElement(next.component as ComponentType<object>, next.props));
    },
    destroy() {
      // Unmount after Svelte finishes removing the node.
      queueMicrotask(() => root.unmount());
    },
  };
}

/** Atoll's icon (design set) as a React element, for glisse's `icon` props. */
export function icon(name: IconName, size = 20): ReactNode {
  const filled = ["prev", "next", "play", "pause", "cursor", "dot"].includes(name);
  return createElement(
    "svg",
    {
      width: size,
      height: size,
      viewBox: "0 0 24 24",
      fill: filled ? "currentColor" : "none",
      stroke: filled ? "none" : "currentColor",
      strokeWidth: 1.75,
      strokeLinecap: "round",
      strokeLinejoin: "round",
      "aria-hidden": true,
    },
    createElement("path", { d: ICONS[name] }),
  );
}

/** Icon + text as one React label (tabs, segments). */
export function iconLabel(name: IconName, text: string, size = 16): ReactNode {
  return createElement(
    "span",
    { className: "atoll-lg-label" },
    icon(name, size),
    createElement("span", { className: "atoll-lg-text" }, text),
  );
}

/** Icon-only label that keeps its name for screen readers and as a tooltip. */
export function iconOnlyLabel(name: IconName, text: string, size = 18): ReactNode {
  return createElement(
    "span",
    { className: "atoll-lg-label atoll-lg-icon-only", title: text },
    icon(name, size),
    createElement("span", { className: "atoll-sr-only" }, text),
  );
}
