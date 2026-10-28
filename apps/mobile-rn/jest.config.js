// jest-expo on Android (docs/mobile-rn.md §7). `@voltip/shared` is TypeScript source in the
// workspace, so it is transformed like the app's own code; `@tauri-apps/api` maps to the stub the
// app bundles (metro.config.js).
module.exports = {
  preset: "jest-expo/android",
  // Above the 10 s the async queries may wait (src/test/setup.ts).
  testTimeout: 60_000,
  setupFilesAfterEnv: ["<rootDir>/src/test/setup.ts"],
  moduleNameMapper: {
    "^@tauri-apps/api/(core|event)$": "<rootDir>/src/backend/tauri-stub.ts",
  },
  transformIgnorePatterns: [
    "node_modules/(?!(?:.pnpm/[^/]+/node_modules/)?(?:(?:jest-)?react-native|@react-native(?:-community)?|expo(?:nent)?|@expo(?:nent)?/.*|@expo-google-fonts/.*|react-navigation|@react-navigation/.*|@sentry/react-native|native-base|react-native-svg|react-native-paper|@callstack/.*|@voltip/.*|@material/material-color-utilities))",
  ],
  testPathIgnorePatterns: ["/node_modules/", "/android/"],
  // The same floor as the web packages (`make web-coverage-gate`). `Root.tsx` is the native glue:
  // it starts the Rust shell through the native module, which no test environment has.
  collectCoverageFrom: [
    "src/**/*.{ts,tsx}",
    "!src/**/*.test.{ts,tsx}",
    "!src/test/**",
    "!src/Root.tsx",
  ],
  coverageReporters: ["text", "text-summary", "lcovonly"],
  coverageThreshold: { global: { lines: 90, branches: 90, functions: 90, statements: 90 } },
};
