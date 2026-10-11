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

/** A paired computer, online or not. */
function computer(online: boolean) {
  return {
    device: {
      device_id: "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b",
      public_key: "1".repeat(64),
      name: "Studio",
      platform: "windows",
      fingerprint: "AB:CD",
      trusted_at: 1_758_700_000,
      last_seen: 1_758_700_600,
    },
    connection: { state: online ? "online" : "offline" },
  } as never;
}

/** One finished take in the phone's own history. */
function result(id: number, text: string, sentTo?: string) {
  return historyEntrySchema.parse({
    id: `00000000-0000-4000-8000-${String(id).padStart(12, "0")}`,
    at_ms: Date.now() - id * 60_000,
    raw_text: text,
    text,
    refined: false,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 1200,
    asr_ms: 300,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...(sentTo === undefined ? {} : { origin: { device: sentTo, kind: "sent" } }),
  });
}

const PRESS = { nativeEvent: { locationX: 60, locationY: 60 } };

describe("说话 (Penpot 「澄空 Glass · Voltip」 boards 03/04)", () => {
  it("is titled 说话 over the pairing state, with the way to this phone's page and the dock of three tabs", async () => {
    await renderApp();
    expect(await screen.findByRole("header", { name: "说话" })).toBeOnTheScreen();
    expect(screen.getByTestId("talk-status")).toHaveTextContent("尚未配对电脑 · 在手机上识别");
    const dock = screen.getByTestId("tab-dock");
    expect(
      within(dock)
        .getAllByRole("tab")
        .map((tab) => tab.props.accessibilityLabel as string),
    ).toEqual(["说话", "记录", "设置"]);
    expect(within(dock).getByRole("tab", { name: "说话" })).toBeSelected();
    // Before a computer is paired there is nowhere else to send a take: no 识别方式 segment.
    expect(screen.queryByTestId("phone-route")).toBeNull();
    await fireEvent.press(screen.getByTestId("open-this-device"));
    expect(await screen.findByTestId("phone-device")).toBeOnTheScreen();
  });

  it("with a computer online a take goes to it unless 在手机上识别 is picked, and the scene chip is the phone's own", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer(true)] } });
    const invoke = jest.spyOn(backend, "invoke");
    expect(await screen.findByTestId("talk-status")).toHaveTextContent(
      /^1 台已配对 · 1 台在线 · 中继/,
    );
    expect(screen.getByTestId("phone-route-computer")).toBeChecked();
    expect(within(screen.getByTestId("phone-route")).getByText("发送到 Studio")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-mic-route")).toHaveTextContent(
      "松开后，文字出现在电脑的光标处",
    );
    // The computer picks the scene for its front app: no chip for a take sent to it.
    expect(screen.queryByTestId("phone-scene")).toBeNull();
    await holdAndRelease(screen.getByTestId("phone-mic-hold"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("phone_take_start", { publicKey: "1".repeat(64) });
    });
    // While the take runs its route stays.
    expect(screen.getByTestId("phone-route-phone")).toBeDisabled();
    expect(await screen.findByText(/^已插入电脑：/)).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("phone-route-phone"));
    expect(screen.getByTestId("phone-route-phone")).toBeChecked();
    expect(screen.getByTestId("phone-mic-route")).toHaveTextContent(
      "松开后在手机上识别并润色，结果自动复制",
    );
    expect(screen.getByTestId("phone-scene")).toBeOnTheScreen();
    invoke.mockClear();
    await holdAndRelease(screen.getByTestId("phone-mic-hold"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_start");
    });
    expect(invoke).not.toHaveBeenCalledWith("phone_take_start", expect.anything());
  });

  it("with every paired computer offline the phone transcribes the take and says why", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer(false)] } });
    const invoke = jest.spyOn(backend, "invoke");
    const toComputer = await screen.findByTestId("phone-route-computer");
    expect(toComputer).toBeDisabled();
    expect(within(screen.getByTestId("phone-route")).getByText("发送到电脑")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-route-phone")).toBeChecked();
    expect(screen.getByTestId("phone-mic-offline")).toHaveTextContent(
      "已配对的电脑都不在线，现在在手机上识别。",
    );
    await holdAndRelease(screen.getByTestId("phone-mic-hold"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_start");
    });
  });

  it("while held the card says it records and for how long, shows the wave and what letting go does (board 04)", async () => {
    await renderApp({ mock: { devices: [computer(true)] } });
    const button = await screen.findByTestId("phone-mic-hold");
    expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent("用手机说话");
    expect(screen.getAllByTestId("phone-mic-halo")).toHaveLength(2);
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("按住说话");
    await fireEvent(button, "responderGrant", PRESS);
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent(/^正在录音 · 00:0\d$/);
    });
    expect(screen.getByTestId("phone-mic-wave")).toBeOnTheScreen();
    expect(screen.getAllByTestId("phone-mic-halo")).toHaveLength(3);
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("松开发送到 Studio");
    // The first line of the hint said what releasing does; the label says it now.
    expect(screen.queryByTestId("phone-mic-route")).toBeNull();
    expect(screen.getByText("手指滑出按钮再松开则取消")).toBeOnTheScreen();
    await fireEvent(button, "responderRelease", PRESS);
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent("用手机说话");
    });
    expect(screen.queryByTestId("phone-mic-wave")).toBeNull();
    expect(screen.getAllByTestId("phone-mic-halo")).toHaveLength(2);
  });

  it("lists the three newest results on cards with where each went; copying one says so on its card", async () => {
    const { backend } = await renderApp({
      mock: {
        history: [
          result(1, "把这段逻辑抽成一个 helper。", "Studio"),
          result(2, "会议改到周四下午三点，地点不变。"),
          result(3, "第三条。"),
          result(4, "第四条。"),
        ],
      },
    });
    const rows = await screen.findAllByTestId("phone-recent-row");
    expect(rows).toHaveLength(3);
    const meta = screen.getAllByTestId("phone-recent-meta");
    expect(meta[0]).toHaveTextContent(/ · 已插入 Studio$/);
    expect(meta[1]).toHaveTextContent(/ · 在手机上识别$/);
    const first = rows[0];
    if (first === undefined) throw new Error("no row");
    await fireEvent.press(within(first).getByTestId("phone-recent-copy"));
    expect(await within(first).findByTestId("phone-recent-copied")).toHaveTextContent(/已复制$/);
    expect(within(first).queryByTestId("phone-recent-copy")).toBeNull();
    expect((backend as unknown as { phoneClipboard: string }).phoneClipboard).toBe(
      "把这段逻辑抽成一个 helper。",
    );
    // 全部记录 opens the 记录 tab.
    await fireEvent.press(screen.getByTestId("phone-recent-all"));
    expect(await screen.findByTestId("phone-history")).toBeOnTheScreen();
  });

  it("draws the aurora behind the tab pages on 澄空 and 夜空 only", async () => {
    const { backend } = await renderApp();
    expect(await screen.findByTestId("aurora")).toBeOnTheScreen();
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "glass_dark", followSystem: false });
    });
    expect(screen.getByTestId("aurora")).toBeOnTheScreen();
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "light", followSystem: false });
    });
    await waitFor(() => {
      expect(screen.queryByTestId("aurora")).toBeNull();
    });
  });
});
