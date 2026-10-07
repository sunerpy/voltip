// <App> on `@voltip/shared/mock`'s in-memory backend, as a phone, in Chinese unless a test says
// otherwise; with a window of a Pixel's size (there is none under jest).
import { fireEvent, render, screen } from "@testing-library/react-native";
import { MockBackend, type MockBackendOptions } from "@voltip/shared/mock";

import { App } from "../App";

export const METRICS = {
  frame: { x: 0, y: 0, width: 412, height: 915 },
  insets: { top: 24, right: 0, bottom: 24, left: 0 },
};

// Every test stops what it started: the mock backends' timers end with the test.
const backends = new Set<MockBackend>();
afterEach(() => {
  for (const backend of backends) backend.destroy();
  backends.clear();
});

export async function renderApp({
  backend,
  mock,
  language = "zh-CN",
  accent = null,
}: {
  backend?: MockBackend;
  mock?: MockBackendOptions;
  language?: string;
  /** The wallpaper's colour the system would hand over (Android 12+). */
  accent?: string | null;
} = {}) {
  const core = backend ?? new MockBackend({ role: "phone", ...mock });
  backends.add(core);
  const view = await render(
    <App backend={core} language={language} metrics={METRICS} accent={accent} />,
  );
  return { ...view, backend: core };
}

/** Open a tab. Paper's navigation bar takes no touches until it has measured itself, which under
 *  jest nothing does: the layout event is sent first. */
export async function openTab(tab: "talk" | "history" | "settings") {
  await fireEvent(screen.getByTestId("bottom-navigation-bar"), "layout", {
    nativeEvent: { layout: { x: 0, y: 0, width: METRICS.frame.width, height: 80 } },
  });
  await fireEvent.press(await screen.findByTestId(`tab-${tab}`));
}
