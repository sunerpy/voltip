import { MockBackend, type MockBackendOptions, sampleDevices } from "@voltip/shared/mock";
import { render } from "@testing-library/react";
import { App } from "../App";

export interface RenderAppOptions {
  path?: string;
  backend?: MockBackend;
  mock?: MockBackendOptions;
  /** What `settings.locale = "system"` resolves against; a Chinese OS unless a test says otherwise
   *  (jsdom's own `navigator.language` is `en-US`). */
  systemLanguage?: string;
}

/** Mounts the whole desktop app on a memory route with an in-memory core. */
export function renderApp({
  path = "/",
  backend,
  mock,
  systemLanguage = "zh-CN",
}: RenderAppOptions = {}) {
  const core =
    backend ??
    new MockBackend({
      devices: sampleDevices(1_758_700_000),
      now: () => 1_758_700_000_000,
      ...mock,
    });
  const view = render(<App backend={core} initialPath={path} systemLanguage={systemLanguage} />);
  return { ...view, backend: core };
}

export async function flushPromises(): Promise<void> {
  await new Promise((r) => {
    setTimeout(r, 0);
  });
}
