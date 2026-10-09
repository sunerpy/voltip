// Vite config for the Voltip mobile webview. Tailwind 4 is wired through @tailwindcss/vite (the
// supported v4 arrangement). Port and outDir are pinned to what src-tauri/tauri.conf.json expects.
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
