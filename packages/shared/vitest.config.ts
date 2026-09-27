import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    globals: true,
    include: ["src/**/*.test.ts"],
    // The cross-language e2e run spawns Rust binaries; it has its own config (vitest.e2e.config.ts).
    exclude: ["src/ipc-e2e.test.ts", "**/node_modules/**"],
    restoreMocks: true,
    // CI runners are slower than a laptop: user-event heavy tests exceeded the 5 s default.
    testTimeout: 20_000,
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      exclude: ["src/**/*.test.ts", "src/**/*.d.ts", "src/fixtures/**", "src/index.ts"],
      // fixtures/ipc/*.json is data shared with the Rust side, not code; the glob above skips it.
      thresholds: { lines: 90, branches: 90, functions: 90, statements: 90 },
      reporter: ["text", "text-summary", "lcovonly"],
    },
  },
});
