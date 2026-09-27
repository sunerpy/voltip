import { MockBackend, type MockBackendOptions } from "@voltip/shared/mock";
import { render } from "@testing-library/react";
import { App, type AppProps } from "../App";
import type { Scanner } from "../app/scanner";

export interface RenderOptions {
  backend?: MockBackend;
  mock?: MockBackendOptions;
  scanner?: Scanner | undefined;
  initialScreen?: AppProps["initialScreen"];
  /** OS language `settings.locale = "system"` follows; a Chinese phone unless a test says otherwise. */
  systemLanguage?: string;
}

export function renderApp({
  backend,
  mock,
  scanner,
  initialScreen,
  systemLanguage = "zh-CN",
}: RenderOptions = {}) {
  const core = backend ?? new MockBackend({ role: "phone", ...mock });
  const view = render(
    <App
      backend={core}
      loadScanner={() => Promise.resolve(scanner)}
      initialScreen={initialScreen}
      systemLanguage={systemLanguage}
    />,
  );
  return { ...view, backend: core };
}
