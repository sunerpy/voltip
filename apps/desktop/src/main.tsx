import { isTauri } from "@tauri-apps/api/core";
import { type Backend, TauriBackend } from "@voltip/shared";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { installContextMenuPolicy } from "./app/context-menu";
import "./index.css";

async function createBackend(): Promise<Backend> {
  // `pnpm dev` in a browser: an in-memory core that auto-drives the phone side. `import.meta.env.DEV`
  // is a build-time constant, so a release build carries neither this branch nor the mock module.
  if (import.meta.env.DEV && !isTauri()) {
    const { MOCK_ENGINE_BUILTIN, MockBackend, sampleDevices } = await import("@voltip/shared/mock");
    return new MockBackend({
      autoPeer: { joinAfterMs: 20_000, confirmAfterMs: 6000 },
      devices: sampleDevices(),
      // As in a release build, the built-in service previews while recording (docs/dictation.md §11.8).
      builtIn: {
        asr: { model: MOCK_ENGINE_BUILTIN.asr_model, key: true, preview: true },
        llm: { model: MOCK_ENGINE_BUILTIN.refine_model, key: true },
      },
    });
  }
  return new TauriBackend();
}

// Windows test 2026-09-25: no browser context menu inside the client (text fields keep theirs);
// the `pnpm dev` browser preview is left alone.
installContextMenuPolicy(document, { enabled: isTauri() });

// Every launch opens where the window points (the home page): the defaults work out of the box, so
// the setup guide waits in 设置 › 通用 and a missing permission shows on the home page
// (user feedback 2026-09-28).
const container = document.getElementById("root");
if (!container) throw new Error("#root missing");
const backend = await createBackend();
/** The mounted app: a test unmounts it before its environment goes away. */
export const root = createRoot(container);
root.render(
  <StrictMode>
    <App backend={backend} />
  </StrictMode>,
);
