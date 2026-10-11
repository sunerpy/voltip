import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { DeviceEventEmitter } from "react-native";
import { historyEntrySchema } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";

import state from "../../../packages/shared/src/fixtures/ipc/state.json";

import { appTheme } from "./theme/themes";
import { statTileColors } from "./ui/statTile";
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
    // The card opens the built-in service's sheet, with the model picker in it.
    await fireEvent.press(
      within(await screen.findByTestId("provider-llm-builtin")).getByRole("button", {
        name: "内置服务",
      }),
    );
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

  it("语音模型 lists the providers as cards; one opens its settings in a sheet, and 设为当前 saves its key and switches", async () => {
    // User decision 2026-10-10 (Penpot 「Voltip 服务商设置」): cards two to a row, a provider's
    // settings in a sheet from the bottom.
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-speech"));
    const builtin = await screen.findByTestId("provider-asr-builtin");
    expect(within(builtin).getByText("使用中")).toBeOnTheScreen();
    expect(within(builtin).getByText("Qwen/Qwen3-ASR-1.7B")).toBeOnTheScreen();
    const groq = screen.getByTestId("provider-asr-groq");
    expect(within(groq).getByText("缺少密钥")).toBeOnTheScreen();
    expect(within(groq).getByText("whisper-large-v3-turbo")).toBeOnTheScreen();
    // A problem colours the card's dot, before 使用中 would.
    expect(screen.getByTestId("provider-asr-groq-lamp")).toHaveStyle({
      backgroundColor: appTheme("glass").voltip.warning,
    });
    expect(screen.queryByTestId("provider-sheet-sheet")).toBeNull();
    await fireEvent.press(within(groq).getByRole("button", { name: "Groq" }));
    const sheet = await screen.findByTestId("provider-sheet-sheet");
    expect(within(sheet).getByText("Groq")).toBeOnTheScreen();
    expect(within(sheet).getByText("缺少密钥")).toBeOnTheScreen();
    // Groq needs its key: 设为当前 refuses first and switches nothing.
    await fireEvent.press(within(sheet).getByTestId("provider-use"));
    expect(within(sheet).getByTestId("provider-problem")).toHaveTextContent("请填写 API 密钥");
    expect(invoke).not.toHaveBeenCalledWith("settings_set_engines", expect.anything());
    // Regression (Device Farm 2026-10-11): the fields filled themselves with the page's grey.
    const outlines = within(sheet).getAllByTestId("text-input-outline");
    expect(outlines.length).toBeGreaterThanOrEqual(2);
    for (const outline of outlines)
      expect(outline).toHaveStyle({
        backgroundColor: appTheme("glass").colors.elevation.level1,
      });
    await fireEvent.changeText(within(sheet).getByTestId("provider-key"), "gsk_test");
    await fireEvent.press(within(sheet).getByTestId("provider-use"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
        engines: expect.objectContaining({ asr_provider: "groq" }),
      });
    });
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "groq",
      kind: "asr",
      value: "gsk_test",
    });
    await waitFor(() => {
      expect(screen.queryByTestId("provider-sheet-sheet")).toBeNull();
    });
    await waitFor(() => {
      expect(within(screen.getByTestId("provider-asr-groq")).getByText("使用中")).toBeOnTheScreen();
    });
    expect(within(screen.getByTestId("provider-asr-builtin")).queryByText("使用中")).toBeNull();
  });

  it("a provider's sheet saves its model and stays, Android's back closes it, and it opens afresh", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-speech"));
    const custom = await screen.findByTestId("provider-asr-custom");
    expect(within(custom).getByText("缺少接口地址")).toBeOnTheScreen();
    await fireEvent.press(within(custom).getByRole("button", { name: "自定义接口" }));
    const sheet = await screen.findByTestId("provider-sheet-sheet");
    // No model list: the model is a free field.
    await fireEvent.changeText(within(sheet).getByTestId("provider-model-id"), "whisper-large-v3");
    await fireEvent.changeText(
      within(sheet).getByTestId("provider-url"),
      "http://192.168.1.20:8000/v1",
    );
    await fireEvent.press(within(sheet).getByTestId("provider-save"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
        engines: expect.objectContaining({
          providers: {
            custom: { asr_model: "whisper-large-v3", asr_url: "http://192.168.1.20:8000/v1" },
          },
        }),
      });
    });
    expect(invoke).not.toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({ asr_provider: "custom" }),
    });
    expect(screen.getByTestId("provider-sheet-sheet")).toBeOnTheScreen();
    await waitFor(() => {
      expect(within(screen.getByTestId("provider-asr-custom")).getByText("就绪")).toBeOnTheScreen();
    });
    await fireEvent.changeText(within(sheet).getByTestId("provider-url"), "draft");
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.queryByTestId("provider-sheet-sheet")).toBeNull();
    });
    // The page stays where it was, and the next opening forgets the unsaved draft.
    expect(screen.getByTestId("phone-speech")).toBeOnTheScreen();
    await fireEvent.press(
      within(screen.getByTestId("provider-asr-custom")).getByRole("button", { name: "自定义接口" }),
    );
    expect(await screen.findByTestId("provider-sheet-sheet")).toBeOnTheScreen();
    expect(screen.getByTestId("provider-url")).toHaveDisplayValue("http://192.168.1.20:8000/v1");
  });

  it("关于 opens the privacy policy in the phone's language", async () => {
    // Google Play asks an app that records audio to link its privacy policy inside the app too
    // (the Play listing, 2026-10-08).
    const { backend } = await renderApp();
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-about"));
    await fireEvent.press(await screen.findByText("隐私政策"));
    expect(backend.guidesOpened).toEqual([{ page: "privacy", locale: "zh-CN" }]);
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

  it("录音 picks the longest take from a bottom sheet, not a dialog of radio buttons", async () => {
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

  it("a picker opens a sheet from the bottom: its title, the current choice checked, the scrim closes it", async () => {
    // User request 2026-10-09: 下拉框都应优化为底部弹出的抽屉.
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-recording"));
    await fireEvent.press(await screen.findByTestId("recording-max-minutes"));
    const sheet = await screen.findByTestId("recording-max-minutes-sheet");
    expect(within(sheet).getByText("最长录音时长")).toBeOnTheScreen();
    expect(within(sheet).getByTestId("recording-max-minutes-option-10")).toBeChecked();
    expect(within(sheet).getByTestId("recording-max-minutes-option-30")).not.toBeChecked();
    // The sheet is modal for screen readers, so the scrim behind it is hidden from them (they close
    // it with back); a finger can still tap it.
    await fireEvent.press(
      screen.getByTestId("recording-max-minutes-scrim", { includeHiddenElements: true }),
    );
    await waitFor(() => {
      expect(screen.queryByTestId("recording-max-minutes-sheet")).toBeNull();
    });
    expect(invoke).not.toHaveBeenCalledWith("settings_set_recording", expect.anything());
    expect(screen.getByTestId("phone-recording")).toBeOnTheScreen();
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

  it("regression: each stat tile takes its span's hue and the total is the lit ink card", async () => {
    // User decision 2026-10-10 (Penpot 「Voltip 统计卡」, option C with total ①).
    const history = historyEntrySchema.array().parse(state.history_recent);
    await renderApp({ backend: new MockBackend({ role: "phone", history }) });
    await openTab("history");
    const theme = appTheme("glass");
    for (const id of ["today", "week", "month"] as const) {
      const tile = await screen.findByTestId(`phone-history-stat-${id}`);
      expect(tile).toHaveStyle({ backgroundColor: statTileColors(theme, id).background });
      expect(within(tile).queryByTestId("phone-history-stat-glow")).toBeNull();
    }
    const total = screen.getByTestId("phone-history-stat-total");
    expect(within(total).getByTestId("phone-history-stat-glow")).toBeOnTheScreen();
    expect(within(total).getByTestId("phone-history-stat-chip")).toHaveStyle({
      backgroundColor: statTileColors(theme, "total").chip,
    });
    const today = screen.getByTestId("phone-history-stat-today");
    expect(within(today).getByTestId("phone-history-stat-saved")).toHaveTextContent(/^\d/);
    expect(within(today).getByTestId("phone-history-stat-caption")).toHaveTextContent(
      /^节省时间 · 听写 \d+ 次$/,
    );
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
