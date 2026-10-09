// Metro for the RN app inside the pnpm workspace (docs/mobile-rn.md §6). Expo's default config
// already watches the workspace root, so `@voltip/shared` resolves from packages/shared.
const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// `@voltip/shared`'s TauriBackend imports `@tauri-apps/api`, which only a Tauri webview can run.
// The app never calls it (its transport is the native module, src/backend/transport.ts), so those
// imports resolve to a stub that throws if anything does.
const TAURI_STUB = path.resolve(__dirname, "src/backend/tauri-stub.ts");
const TAURI_MODULES = new Set(["@tauri-apps/api/core", "@tauri-apps/api/event"]);
config.resolver.resolveRequest = (context, moduleName, platform) =>
  TAURI_MODULES.has(moduleName)
    ? { type: "sourceFile", filePath: TAURI_STUB }
    : context.resolveRequest(context, moduleName, platform);

module.exports = config;
