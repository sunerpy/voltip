import { isTauri } from "@tauri-apps/api/core";
import { type Backend, TauriBackend } from "@voltip/shared";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { tauriBack } from "./app/back";
import { loadScanner } from "./app/scanner";
import "./index.css";

async function createBackend(): Promise<Backend> {
  // Browser preview: the phone side with a simulated desktop that confirms 3 s after the handshake.
  // `import.meta.env.DEV` is a build-time constant: a release build never bundles the mock.
  if (import.meta.env.DEV && !isTauri()) {
    const { MockBackend } = await import("@voltip/shared/mock");
    return new MockBackend({ role: "phone", autoPeer: { joinAfterMs: 0, confirmAfterMs: 3000 } });
  }
  return new TauriBackend();
}

const container = document.getElementById("root");
if (!container) throw new Error("#root missing");
const backend = await createBackend();
createRoot(container).render(
  <StrictMode>
    <App
      backend={backend}
      loadScanner={loadScanner}
      {...(isTauri() ? { systemBack: tauriBack() } : {})}
    />
  </StrictMode>,
);
