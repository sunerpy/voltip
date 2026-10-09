import { defineConfig } from "vitest/config";

// Cross-language IPC end-to-end run (`pnpm run test:e2e` / `make e2e-ipc`): builds and spawns the
// Rust relay and bridge harness, so it is kept out of the unit-test and coverage configuration.
export default defineConfig({
  test: {
    environment: "node",
    globals: true,
    include: ["src/ipc-e2e.test.ts"],
    fileParallelism: false,
    testTimeout: 180_000,
    hookTimeout: 600_000,
    reporters: ["verbose"],
  },
});
