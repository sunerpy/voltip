// Vite config for the Voltip desktop webview. Tailwind 4 is wired through @tailwindcss/vite (the
// supported v4 arrangement). Port and outDir are pinned to what src-tauri/tauri.conf.json expects.
import { randomBytes } from "node:crypto";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Every build of the interface names itself (src/ui-build.ts), and the program checks that its
// windows run the build it carries (src-tauri/src/ui_guard.rs, docs/dictation.md §33.4).
const uiBuild = `voltip-ui-build-${randomBytes(16).toString("hex")}`;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: { __VOLTIP_UI_BUILD__: JSON.stringify(uiBuild) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
