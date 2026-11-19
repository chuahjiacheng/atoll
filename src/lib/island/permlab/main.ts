// Permission lab (dev only, permission-lab.html): the real PermissionPeek with
// sample requests whose analysis came from the real crate (make-fixtures.py).
// Tauri is stubbed: deciding a request only logs it.
import { mount } from "svelte";
import "../../../app.css";

(window as unknown as { __decided: unknown[] }).__decided = [];
(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args: unknown) => {
    (window as unknown as { __decided: unknown[] }).__decided.push({ cmd, args });
    return null;
  },
  transformCallback: () => 0,
  metadata: { currentWindow: { label: "island" }, currentWebview: { label: "island" } },
};

const { default: PermLab } = await import("./PermLab.svelte");
const fixtures = await fetch(new URL("./fixtures.json", import.meta.url)).then((r) => r.json());
mount(PermLab, { target: document.getElementById("app")!, props: { fixtures } });
