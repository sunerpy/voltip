import { sampleDevices, sampleHistory } from "@voltip/shared/mock";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type * as TauriCore from "@tauri-apps/api/core";
import { EXIT_WINDOW_MS } from "../App";
import { renderApp } from "../test/render";
import { BACK_PLUGIN, type SystemBack, tauriBack } from "./back";

const { onBackButtonPress } = vi.hoisted(() => ({ onBackButtonPress: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ onBackButtonPress }));
const core = vi.hoisted(() => ({ addPluginListener: vi.fn(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof TauriCore>()),
  ...core,
}));

/** Android's back as a test drives it: `press` is one swipe from the edge. */
function fakeBack() {
  let handler: (() => void) | undefined;
  const released: number[] = [];
  const back: SystemBack = {
    listen(next) {
      handler = next;
      return () => {
        handler = undefined;
      };
    },
    release(ms) {
      released.push(ms);
    },
  };
  return {
    back,
    released,
    press() {
      act(() => {
        handler?.();
      });
    },
    listening: () => handler !== undefined,
  };
}

const NOW = Math.floor(Date.now() / 1000);

describe("Android's back on the phone", () => {
  it("regression: it goes up a level, then to 说话, and only a second back at 说话 leaves", async () => {
    // User report 2026-10-02: the edge swipe left the app from every screen, even 设置 › 外观.
    const user = userEvent.setup();
    const system = fakeBack();
    const { backend } = renderApp({
      mock: { devices: sampleDevices(NOW) },
      systemBack: system.back,
    });
    await screen.findByTestId("tab-settings");
    await user.click(screen.getByTestId("tab-settings"));
    await user.click(await screen.findByTestId("settings-appearance"));
    expect(screen.getByRole("heading", { name: "外观与语言", level: 1 })).toBeInTheDocument();
    system.press();
    expect(screen.getByRole("heading", { name: "设置", level: 1 })).toBeInTheDocument();
    system.press();
    // 说话: the device list, since this phone is paired.
    expect(screen.getByTestId("tab-talk")).toHaveAttribute("aria-current", "page");
    expect(system.released).toEqual([]);
    system.press();
    expect(await screen.findByText("再返回一次即可退出")).toBeInTheDocument();
    expect(system.released).toEqual([EXIT_WINDOW_MS]);
    expect(system.listening()).toBe(true);
    backend.destroy();
  });

  it("closes a dialog before it changes the screen", async () => {
    const user = userEvent.setup();
    const system = fakeBack();
    const { backend } = renderApp({
      mock: { devices: sampleDevices(NOW) },
      systemBack: system.back,
    });
    const card = (await screen.findAllByTestId("device-card"))[0] as HTMLElement;
    await user.click(within(card).getByRole("button", { name: /^忘记 / }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    system.press();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getAllByTestId("device-card").length).toBeGreaterThan(0);
    expect(system.released).toEqual([]);
    backend.destroy();
  });

  it("from a page opened in 记录 it goes back to 记录, and from 记录 to 说话", async () => {
    const user = userEvent.setup();
    const system = fakeBack();
    const { backend } = renderApp({
      mock: { history: sampleHistory(Date.now()) },
      initialScreen: "history",
      systemBack: system.back,
    });
    await user.click((await screen.findAllByTestId("phone-history-row"))[0] as HTMLElement);
    await screen.findByTestId("phone-entry");
    system.press();
    expect(await screen.findByTestId("phone-history")).toBeInTheDocument();
    system.press();
    // An unpaired phone's 说话 is the welcome screen.
    expect(screen.getByTestId("tab-talk")).toHaveAttribute("aria-current", "page");
    expect(system.released).toEqual([]);
    backend.destroy();
  });

  it("back from 核对安全码 cancels the pairing, as the header's 返回 does", async () => {
    const system = fakeBack();
    const { backend } = renderApp({ initialScreen: "pair", systemBack: system.back });
    await screen.findByRole("heading", { name: "配对电脑" });
    await act(async () => {
      await backend.invoke("pairing_join_code", { code: "483921" });
    });
    expect(
      await screen.findByRole("heading", { name: "核对安全码" }, { timeout: 5000 }),
    ).toBeInTheDocument();
    system.press();
    expect(screen.getByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "cancelled" },
    });
    backend.destroy();
  });
});

describe("tauriBack", () => {
  beforeEach(() => {
    onBackButtonPress.mockReset();
    core.addPluginListener.mockReset();
    core.invoke.mockReset();
  });

  it("regression: it listens through the app's own back plugin, and the second back goes to the system", async () => {
    // The android-device CI job, 2026-10-03: Tauri's back event (`onBackButtonPress`) is added to
    // the first activity alone; Android recreated the activity after start, and the first back on
    // 外观与语言 left the app. BackPlugin.kt takes the backs of every activity.
    const unregister = vi.fn(() => Promise.resolve());
    const handlers: (() => void)[] = [];
    core.addPluginListener.mockImplementation((_plugin: string, _event: string, h: () => void) => {
      handlers.push(h);
      return Promise.resolve({ unregister });
    });
    core.invoke.mockResolvedValue(undefined);
    const back = tauriBack();
    const pressed = vi.fn();
    const stop = back.listen(pressed);
    expect(core.addPluginListener).toHaveBeenCalledTimes(1);
    expect(core.addPluginListener).toHaveBeenCalledWith(BACK_PLUGIN, "back", expect.any(Function));
    expect(onBackButtonPress).not.toHaveBeenCalled();
    handlers[0]?.();
    expect(pressed).toHaveBeenCalledTimes(1);
    // At 说话: the plugin lets the backs of the window through, and the page keeps listening.
    back.release(EXIT_WINDOW_MS);
    expect(core.invoke).toHaveBeenCalledWith(`plugin:${BACK_PLUGIN}|release`, {
      ms: EXIT_WINDOW_MS,
    });
    expect(unregister).not.toHaveBeenCalled();
    stop();
    await vi.waitFor(() => {
      expect(unregister).toHaveBeenCalledTimes(1);
    });
    handlers[0]?.();
    expect(pressed).toHaveBeenCalledTimes(1);
    // Listening again registers again.
    back.listen(pressed)();
    expect(core.addPluginListener).toHaveBeenCalledTimes(2);
    await vi.waitFor(() => {
      expect(unregister).toHaveBeenCalledTimes(2);
    });
  });
});
