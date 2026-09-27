import { isTauri } from "@tauri-apps/api/core";
import { type Backend, TauriBackend } from "@voltip/shared";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { installContextMenuPolicy } from "./app/context-menu";
import { firstRunPath } from "./app/first-run";
import { currentLocationPath } from "./app/router";
import "./index.css";

async function createBackend(): Promise<Backend> {
  // `pnpm dev` in a browser: an in-memory core that auto-drives the phone side. `import.meta.env.DEV`
  // is a build-time constant, so a release build carries neither this branch nor the mock module.
  if (import.meta.env.DEV && !isTauri()) {
    const { MockBackend, sampleDevices } = await import("@voltip/shared/mock");
    return new MockBackend({
      autoPeer: { joinAfterMs: 20_000, confirmAfterMs: 6000 },
      devices: sampleDevices(),
    });
  }
  return new TauriBackend();
}

// Windows test 2026-09-25: no browser context menu inside the client (text fields keep theirs);
// the `pnpm dev` browser preview is left alone.
installContextMenuPolicy(document, { enabled: isTauri() });

// The first launch opens the first-run guide; finishing or skipping it sets the flag.
const firstRun = firstRunPath(currentLocationPath(), window.localStorage);
if (firstRun !== undefined) window.history.replaceState(null, "", firstRun);

const container = document.getElementById("root");
if (!container) throw new Error("#root missing");
const backend = await createBackend();
createRoot(container).render(
  <StrictMode>
    <App backend={backend} />
  </StrictMode>,
);
