import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    restoreMocks: true,
    // CI runners are slower than a laptop: user-event heavy tests exceeded the 5 s default.
    testTimeout: 20_000,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/**/*.d.ts", "src/test/**", "src/index.ts"],
      thresholds: { lines: 90, branches: 90, functions: 90, statements: 90 },
      reporter: ["text", "text-summary", "lcovonly"],
    },
  },
});
