import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), {
    name: "reject-vendored-dompurify",
    generateBundle() {
      for (const id of this.getModuleIds()) {
        if (/[/\\]monaco-editor[/\\].*[/\\]dompurify[/\\]dompurify\.js$/.test(id)) {
          this.error("Monaco's vulnerable vendored DOMPurify must not enter the bundle");
        }
      }
    },
  }],
  // Monaco ships an embedded copy; dependency overrides alone do not replace it.
  resolve: { alias: [{ find: /^\.\/dompurify\/dompurify\.js$/, replacement: "dompurify" }] },
  optimizeDeps: { include: ["dompurify"] },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
