// <App> on `@voltip/shared/mock`'s in-memory backend, as a phone, in Chinese unless a test says
// otherwise; with a window of a Pixel's size (there is none under jest).
import { fireEvent, render, screen } from "@testing-library/react-native";
import { MockBackend, type MockBackendOptions } from "@voltip/shared/mock";

import { App } from "../App";
import type { BubbleControl } from "../backend/bubble";

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
  bubble,
  navigationBar,
  metrics = METRICS,
}: {
  backend?: MockBackend;
  mock?: MockBackendOptions;
  language?: string;
  /** The wallpaper's colour the system would hand over (Android 12+). */
  accent?: string | null;
  /** The floating bubble's native side; none by default, as on a build without it. */
  bubble?: BubbleControl;
  /** What the native module's navigation bar call would be handed. */
  navigationBar?: (lightButtons: boolean) => void;
  /** The window and its insets: a portrait phone unless a test turns it on its side. */
  metrics?: typeof METRICS;
} = {}) {
  const core = backend ?? new MockBackend({ role: "phone", ...mock });
  backends.add(core);
  const view = await render(
    <App
      backend={core}
      language={language}
      metrics={metrics}
      accent={accent}
      bubble={bubble}
      navigationBar={navigationBar}
    />,
  );
  return { ...view, backend: core };
}

/** Open a tab from the dock. */
export async function openTab(tab: "talk" | "history" | "settings") {
  await fireEvent.press(await screen.findByTestId(`tab-${tab}`));
}

/** The `i`th of `list`: a test fails here, naming the item, rather than on `undefined` later. */
export function nth<T>(list: readonly T[], i: number): T {
  const item = list[i];
  if (item === undefined) throw new Error(`no item ${String(i)} of ${String(list.length)}`);
  return item;
}

/** `backend`'s commands `names` answered at once and acted on by nothing, every other one as the
 *  mock does it: the test reports what the core would, so a state it asserts cannot move on by
 *  itself on a slow machine. `mockRestore` on the returned spy undoes it. */
export function hold(backend: MockBackend, ...names: readonly string[]) {
  const original = backend.invoke.bind(backend);
  return jest
    .spyOn(backend, "invoke")
    .mockImplementation((name, ...args) =>
      names.includes(name) ? Promise.resolve() : original(name, ...args),
    );
}

/** The request id of the last `name` command `spy` (from `hold`) saw; a test fails here without
 *  one. */
export function requestIdOf(spy: ReturnType<typeof hold>, name: string): number {
  const call = spy.mock.calls.reduce<(typeof spy.mock.calls)[number] | undefined>(
    (last, each) => (each[0] === name ? each : last),
    undefined,
  );
  if (call === undefined) throw new Error(`no ${name}`);
  return (call[1] as { requestId: number }).requestId;
}
