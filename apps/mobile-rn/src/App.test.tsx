import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { DeviceEventEmitter } from "react-native";
import { historyEntrySchema } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";

import state from "../../../packages/shared/src/fixtures/ipc/state.json";

import { appTheme } from "./theme/themes";
import { openTab, renderApp } from "./test/render";

/** Press and release the hold-to-talk button, the finger staying on it. */
async function holdAndRelease(button: ReturnType<typeof screen.getByTestId>) {
  await fireEvent(button, "responderGrant", { nativeEvent: { locationX: 60, locationY: 60 } });
  await fireEvent(button, "responderRelease", { nativeEvent: { locationX: 60, locationY: 60 } });
}

describe("the phone app on React Native", () => {
  it("starts at 说话 with the hold-to-talk button and the way to pair a computer", async () => {
    await renderApp();
    expect(await screen.findByTestId("phone-welcome")).toBeOnTheScreen();
    expect(screen.getByText("按住说话")).toBeOnTheScreen();
    expect(screen.getByTestId("welcome-start")).toBeOnTheScreen();
    expect(screen.getByTestId("tab-talk")).toBeOnTheScreen();
    expect(screen.getByTestId("tab-history")).toBeOnTheScreen();
    expect(screen.getByTestId("tab-settings")).toBeOnTheScreen();
  });

  it("speaks English when the phone does and the setting follows the system", async () => {
    await renderApp({ language: "en-US" });
    expect(await screen.findByText("Hold to talk")).toBeOnTheScreen();
  });

  it("holding the button starts a take on the phone and releasing stops it", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await holdAndRelease(await screen.findByTestId("phone-mic-hold"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_start");
    });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_stop");
    });
  });

  it("a release off the button cancels the take", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    const button = await screen.findByTestId("phone-mic-hold");
    await fireEvent(button, "layout", {
      nativeEvent: { layout: { x: 0, y: 0, width: 136, height: 136 } },
    });
    await fireEvent(button, "responderGrant", { nativeEvent: { locationX: 60, locationY: 60 } });
    await fireEvent(button, "responderMove", { nativeEvent: { locationX: 60, locationY: -200 } });
    expect(await screen.findByText("松开取消")).toBeOnTheScreen();
    await fireEvent(button, "responderRelease", {
      nativeEvent: { locationX: 60, locationY: -200 },
    });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_cancel");
    });
    expect(invoke).not.toHaveBeenCalledWith("dictation_stop");
  });

  it("设置 lists the phone's pages and opens them", async () => {
    await renderApp();
    await openTab("settings");
    expect(await screen.findByTestId("phone-settings")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("settings-speech"));
    expect(await screen.findByTestId("phone-speech")).toBeOnTheScreen();
    expect(screen.getByTestId("provider-asr-builtin")).toBeOnTheScreen();
    expect(screen.queryByTestId("provider-asr-local")).toBeNull();
  });

  it("AI 模型 picks one of the built-in service's models", async () => {
    // User request 2026-10-08: the built-in AI polish offers several models.
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-ai"));
    await fireEvent.press(await screen.findByTestId("builtin-llm-model"));
    await fireEvent.press(
      await screen.findByTestId("builtin-llm-model-option-openai/gpt-oss-120b"),
    );
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
        engines: expect.objectContaining({
          providers: { builtin: { llm_model: "openai/gpt-oss-120b" } },
        }),
      });
    });
  });

  it("外观 changes the theme through the core", async () => {
    const { backend } = await renderApp({
      mock: { settings: { follow_system_theme: false, theme: "light" } },
    });
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-appearance"));
    await fireEvent.press(await screen.findByTestId("theme-dark"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_theme", {
        theme: "dark",
        followSystem: false,
      });
    });
  });

  it("录音 picks the longest take from a dropdown menu, not a dialog of radio buttons", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-recording"));
    await fireEvent.press(await screen.findByTestId("recording-max-minutes"));
    await fireEvent.press(await screen.findByTestId("recording-max-minutes-option-30"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "settings_set_recording",
        expect.objectContaining({ recording: expect.objectContaining({ max_minutes: 30 }) }),
      );
    });
  });

  it("regression: Android back closes an open dropdown and leaves the page where it is", async () => {
    // Device Farm run rn-accept-1 (Pixel 8, 2026-10-07): Paper's menu closed on back but let the
    // press through, so the page under it went back too.
    await renderApp();
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-recording"));
    await fireEvent.press(await screen.findByTestId("recording-max-minutes"));
    expect(await screen.findByTestId("recording-max-minutes-option-30")).toBeOnTheScreen();
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.queryByTestId("recording-max-minutes-option-30")).toBeNull();
    });
    expect(screen.getByTestId("phone-recording")).toBeOnTheScreen();
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    expect(await screen.findByTestId("phone-settings")).toBeOnTheScreen();
  });

  it("regression: a dropdown opened while its page is still settling stays open", async () => {
    // jest under load (2026-10-07): Paper's menu runs a hide animation when it mounts closed, and
    // when that animation ended after the field had opened the menu, it unmounted the open menu's
    // options; the field stayed "expanded" with nothing under it. The fake clock puts the press
    // inside that animation every time.
    jest.useFakeTimers();
    try {
      await renderApp();
      await openTab("settings");
      await fireEvent.press(await screen.findByTestId("settings-recording"));
      await fireEvent.press(await screen.findByTestId("recording-max-minutes"));
      expect(await screen.findByTestId("recording-max-minutes-option-30")).toBeOnTheScreen();
      // Past the end of every menu animation: the open menu keeps its options.
      await act(async () => {
        jest.advanceTimersByTime(2000);
        await Promise.resolve();
      });
      expect(screen.getByTestId("recording-max-minutes-option-30")).toBeOnTheScreen();
    } finally {
      jest.useRealTimers();
    }
  });

  it("takes the wallpaper's colour while the appearance follows the system, Voltip's otherwise", async () => {
    const wallpaper = "#4c8b5f";
    const { backend } = await renderApp({ accent: wallpaper });
    const hold = await screen.findByTestId("phone-mic-hold");
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "light", followSystem: true });
    });
    await waitFor(() => {
      expect(hold).toHaveStyle({ backgroundColor: appTheme("light", wallpaper).colors.primary });
    });
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "light", followSystem: false });
    });
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-hold")).toHaveStyle({
        backgroundColor: appTheme("light").colors.primary,
      });
    });
    expect(appTheme("light").colors.primary).not.toBe(appTheme("light", wallpaper).colors.primary);
  });

  it("Android back on 设置 goes to 说话, and the first back there says a second one leaves", async () => {
    await renderApp();
    await openTab("settings");
    expect(await screen.findByTestId("phone-settings")).toBeOnTheScreen();
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    expect(await screen.findByTestId("phone-welcome")).toBeOnTheScreen();
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    expect(await screen.findByText("再返回一次即可退出")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-welcome")).toBeOnTheScreen();
  });

  it("配对 joins with a six-digit code", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(await screen.findByTestId("welcome-start"));
    expect(await screen.findByTestId("phone-device")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("配对电脑"));
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("pair-method-code"));
    await fireEvent.changeText(await screen.findByTestId("pair-code-input"), "483921");
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("pairing_join_code", { code: "483921" });
    });
  });

  it("记录 shows the phone's history and opens an entry", async () => {
    const history = historyEntrySchema.array().parse(state.history_recent);
    const backend = new MockBackend({ role: "phone", history });
    await renderApp({ backend });
    await openTab("history");
    expect(await screen.findByTestId("phone-history")).toBeOnTheScreen();
    const rows = await screen.findAllByTestId("phone-history-row");
    expect(rows.length).toBe(history.length);
    const [first] = rows;
    if (first === undefined) throw new Error("no row");
    await fireEvent.press(first);
    expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
    expect(await screen.findByTestId("phone-entry-text")).toHaveTextContent(history[0]?.text ?? "");
  });

  it("a toast says what the core reports", async () => {
    const { backend } = await renderApp();
    await screen.findByTestId("phone-welcome");
    await act(async () => {
      (backend as unknown as { emit: (e: unknown) => void }).emit({
        type: "message",
        body: "来自电脑的消息",
      });
      await Promise.resolve();
    });
    expect(await screen.findByText(/来自电脑的消息/)).toBeOnTheScreen();
  });

  it("the talk tab lists a paired computer with its state", async () => {
    await renderApp({
      mock: {
        devices: [
          {
            device: {
              device_id: "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b",
              public_key: "1".repeat(64),
              name: "Studio",
              platform: "windows",
              fingerprint: "AB:CD",
              trusted_at: 1_758_700_000,
              last_seen: 1_758_700_600,
            },
            connection: { state: "offline" },
          },
        ] as never,
      },
    });
    const card = await screen.findByTestId("device-card");
    expect(within(card).getAllByText("Studio").length).toBeGreaterThan(0);
    expect(screen.getByTestId("pair-new")).toBeOnTheScreen();
  });
});
