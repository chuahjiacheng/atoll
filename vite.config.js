import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

// Two pages: index.html = the island overlay, dashboard.html = the settings window.
// Tauri expects a fixed dev port and must not clear the terminal it shares.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // Shown on Settings → About.
  define: { __BUILD_DATE__: JSON.stringify(new Date().toISOString().slice(0, 10)) },
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**", "**/bridge/**", "**/target/**", "**/graphify-out/**"] },
  },
  build: {
    rollupOptions: {
      input: {
        island: resolve(import.meta.dirname, "index.html"),
        dashboard: resolve(import.meta.dirname, "dashboard.html"),
        palette: resolve(import.meta.dirname, "palette.html"), // search-clipboard
      },
    },
  },
});
