// Island lab (dev only): mounts the real Shell and Pill on a plain page so
// width, spring and frame times can be checked without touching the live
// island. Tauri is stubbed: every command resolves to nothing.
import { mount } from "svelte";
import "../../../app.css";

// Count animation-frame requests, so the lab can prove nothing loops at rest.
const realRaf = window.requestAnimationFrame.bind(window);
const counter = { requests: 0 };
window.requestAnimationFrame = (cb) => {
  counter.requests++;
  return realRaf(cb);
};
(window as unknown as { __rafCounter: typeof counter }).__rafCounter = counter;

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async () => null,
  transformCallback: () => 0,
  metadata: { currentWindow: { label: "island" }, currentWebview: { label: "island" } },
};

const { default: Lab } = await import("./Lab.svelte");
mount(Lab, { target: document.getElementById("app")! });
