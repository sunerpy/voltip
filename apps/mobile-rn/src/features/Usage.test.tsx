// The built-in service's usage on the phone (docs/dictation.md §27; Penpot 「用量与额度」 boards
// 15–16) and the floating bubble's settings (§20.12, boards 18a/18b): the talk card's line and its
// used-up state, the notices, 设置 › 用量, and 设置 › 悬浮球 on a fake native side.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { defaultEngineSettings } from "@voltip/shared";
import { MOCK_ENGINE_BUILTIN, MockBackend, sampleUsage } from "@voltip/shared/mock";

import type { BubbleConfig, BubbleControl, BubbleStatus } from "../backend/bubble";
import { openTab, renderApp } from "../test/render";

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

describe("the talk card while the phone recognises with the built-in service", () => {
  it("ends with today's usage and a bar", async () => {
    await renderApp({ mock: { usage: sampleUsage("ok") } });
    expect(await screen.findByTestId("talk-usage-text")).toHaveTextContent(
      "内置服务 · 今日 23 / 60 分钟",
    );
    expect(screen.getByTestId("talk-usage-bar")).toHaveProp(
      "accessibilityValue",
      expect.objectContaining({ now: 38 }),
    );
    expect(screen.queryByTestId("usage-notice-asr_near")).toBeNull();
  });

  it("turns amber from 80 % with a notice per service, each closed for the day", async () => {
    const { backend } = await renderApp({ mock: { usage: sampleUsage("near") } });
    expect(await screen.findByTestId("talk-usage-text")).toHaveTextContent(
      "内置服务 · 今日剩余 8 分钟",
    );
    const asr = screen.getByTestId("usage-notice-asr_near");
    expect(asr).toHaveTextContent(/内置识别今日剩余 8 分钟/);
    // No computer to send to: the other way on is another provider.
    expect(screen.getByTestId("usage-notice-asr_near-body")).toHaveTextContent(
      /重置；也可以在「语音模型」中选择其他服务商。$/,
    );
    expect(screen.getByTestId("usage-notice-refine_near")).toHaveTextContent(
      /内置 AI 润色今日剩余 14 次/,
    );
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(screen.getByTestId("usage-notice-asr_near-close"));
    expect(invoke).toHaveBeenCalledWith("usage_notice_dismiss", { notice: "asr_near" });
    await waitFor(() => {
      expect(screen.queryByTestId("usage-notice-asr_near")).toBeNull();
    });
    expect(screen.getByTestId("usage-notice-refine_near")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("usage-notice-refine_near-open"));
    expect(await screen.findByTestId("phone-ai")).toBeOnTheScreen();
  });

  it("starts no take once recognition is used up, and offers another model", async () => {
    const { backend } = await renderApp({ mock: { usage: sampleUsage("out") } });
    const invoke = jest.spyOn(backend, "invoke");
    const out = await screen.findByTestId("talk-usage-out");
    expect(out).toHaveTextContent(/^今日内置识别额度已用完/);
    expect(out).toHaveTextContent(/重置 · 可以在「语音模型」中换用其他服务商/);
    expect(screen.getByTestId("phone-mic-hold")).toBeDisabled();
    expect(screen.queryByTestId("phone-mic-label")).toBeNull();
    expect(screen.queryByTestId("talk-usage-to-desktop")).toBeNull();
    expect(screen.getByTestId("usage-notice-asr_out-body")).toHaveTextContent(
      /^(还有 .*|不到 1 分钟)重置，可以在「语音模型」中选择其他服务商。$/,
    );
    expect(invoke).not.toHaveBeenCalledWith("dictation_start");
    await fireEvent.press(screen.getByTestId("talk-usage-switch"));
    expect(await screen.findByTestId("phone-speech")).toBeOnTheScreen();
  });

  it("offers the computer online instead, which then takes the recording", async () => {
    await renderApp({ mock: { usage: sampleUsage("out"), devices: [computer(true)] } });
    // A computer online is the default route: no built-in usage on that card.
    expect(await screen.findByTestId("phone-route-computer")).toBeChecked();
    expect(screen.queryByTestId("talk-usage")).toBeNull();
    expect(screen.queryByTestId("usage-notice-asr_out")).toBeNull();
    await fireEvent.press(screen.getByTestId("phone-route-phone"));
    expect(screen.getByTestId("talk-usage-out")).toHaveTextContent(
      /重置 · 可以改为发送到电脑，由电脑识别/,
    );
    expect(screen.getByTestId("usage-notice-asr_out-body")).toHaveTextContent(
      /重置，可改为发送到电脑识别。$/,
    );
    await fireEvent.press(screen.getByTestId("talk-usage-to-desktop"));
    expect(screen.getByTestId("phone-route-computer")).toBeChecked();
    expect(screen.getByTestId("phone-mic-hold")).toBeEnabled();
  });

  it("stays ready while a fallback model stands in, and the notice names it", async () => {
    const backend = new MockBackend({
      role: "phone",
      now: () => 1_758_700_000_000,
      builtIn: {
        asr: {
          model: MOCK_ENGINE_BUILTIN.asr_model,
          models: [MOCK_ENGINE_BUILTIN.asr_model, "qwen3-asr-flash"],
          key: true,
        },
        llm: { model: MOCK_ENGINE_BUILTIN.refine_model, key: true },
      },
      usage: sampleUsage("out"),
      settings: {
        engines: {
          ...defaultEngineSettings(),
          asr_fallback: {
            enabled: true,
            models: [{ provider: "builtin", model: "qwen3-asr-flash" }],
          },
        },
      },
    });
    await renderApp({ backend });
    expect(await screen.findByTestId("usage-notice-asr_out-body")).toHaveTextContent(
      /在此之前由候补模型 qwen3-asr-flash 识别。$/,
    );
    expect(screen.queryByTestId("talk-usage-out")).toBeNull();
    expect(screen.getByTestId("phone-mic-hold")).toBeEnabled();
  });

  it("shows nothing of the built-in service while recognition uses another provider", async () => {
    await renderApp({
      mock: {
        usage: sampleUsage("out"),
        settings: { engines: { ...defaultEngineSettings(), asr_provider: "groq" } },
      },
    });
    expect(await screen.findByTestId("phone-mic-hold")).toBeEnabled();
    expect(screen.queryByTestId("talk-usage")).toBeNull();
    expect(screen.queryByTestId("talk-usage-out")).toBeNull();
    expect(screen.queryByTestId("usage-notice-asr_out")).toBeNull();
    // The clean-up still uses the built-in service: its notice stays.
    expect(screen.getByTestId("usage-notice-refine_out")).toBeOnTheScreen();
  });
});

describe("设置 › 用量 (board 16)", () => {
  it("sums both services on its row and shows each day against its limit", async () => {
    const { backend } = await renderApp({
      mock: { usage: sampleUsage("ok"), devices: [computer(false)] },
    });
    await openTab("settings");
    const row = await screen.findByTestId("settings-usage");
    expect(row).toHaveTextContent(/识别 23 \/ 60 分钟 · 润色 41 \/ 100 次/);
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(row);
    expect(await screen.findByTestId("phone-usage")).toBeOnTheScreen();
    // The page asks the service again as it opens.
    expect(invoke).toHaveBeenCalledWith("usage_refresh");
    expect(screen.getByText(/^这部手机 · 内置服务每天/)).toBeOnTheScreen();
    const asr = screen.getByTestId("usage-card-asr");
    expect(within(asr).getByTestId("usage-status-asr")).toHaveTextContent("剩余 37 分钟");
    expect(within(asr).getByTestId("usage-numbers-asr")).toHaveTextContent("23 / 60 分钟");
    expect(within(asr).getByTestId("usage-reset-asr")).toHaveTextContent(/重置 · (还有|不到)/);
    const bars = within(asr).getAllByTestId("usage-week-bar");
    expect(bars).toHaveLength(7);
    expect(
      within(screen.getByTestId("usage-card-llm")).getByTestId("usage-status-llm"),
    ).toHaveTextContent("剩余 59 次");
    // Other providers, without the computer's on-device model.
    const others = screen.getAllByTestId("usage-other");
    expect(others).toHaveLength(2);
    expect(others[0]).toHaveTextContent(/^阿里云百炼 · 识别 38 分钟剩余额度请在百炼控制台查看$/);
    expect(others[1]).toHaveTextContent(/^Groq · 润色 312 次剩余额度请在服务商处查看$/);
    expect(screen.getByTestId("usage-computer-note")).toHaveTextContent(
      "发送到电脑的录音计入电脑的用量。",
    );
  });

  it("offers 重试 while the numbers cannot be fetched, and says which service uses another provider", async () => {
    const { backend } = await renderApp({
      mock: {
        usage: sampleUsage("unknown"),
        settings: { engines: { ...defaultEngineSettings(), llm_provider: "groq" } },
      },
    });
    await openTab("settings");
    expect(await screen.findByTestId("settings-usage")).toHaveTextContent(/用量暂时无法获取/);
    await fireEvent.press(screen.getByTestId("settings-usage"));
    const asr = await screen.findByTestId("usage-card-asr");
    expect(asr).toHaveTextContent(/无法获取用量暂时无法获取/);
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(within(asr).getByTestId("usage-retry-asr"));
    expect(invoke).toHaveBeenCalledWith("usage_refresh");
    expect(screen.getByTestId("usage-not-in-use-llm")).toHaveTextContent(
      "润色现在使用的是Groq，不计入内置服务的额度",
    );
    expect(screen.queryByTestId("usage-not-in-use-asr")).toBeNull();
    expect(screen.queryByTestId("usage-computer-note")).toBeNull();
  });

  it("says 暂不限量 in the counting period", async () => {
    await renderApp({ mock: { usage: sampleUsage("count") } });
    await openTab("settings");
    expect(await screen.findByTestId("settings-usage")).toHaveTextContent(
      /识别 23 分钟 · 润色 41 次 · 暂不限量/,
    );
    await fireEvent.press(screen.getByTestId("settings-usage"));
    const asr = await screen.findByTestId("usage-card-asr");
    expect(within(asr).getByTestId("usage-status-asr")).toHaveTextContent("暂不限量");
    expect(within(asr).getByTestId("usage-numbers-asr")).toHaveTextContent(
      "23 分钟 · 统计期间不设上限",
    );
  });

  it("is not there in a build without the built-in service", async () => {
    await renderApp({ mock: { builtIn: {} } });
    await openTab("settings");
    await screen.findByTestId("settings-speech");
    expect(screen.queryByTestId("settings-usage")).toBeNull();
  });
});

/** A bubble's native side: granted what `permissions` says, running once enabled with all three. */
function fakeBubble(initial: Partial<BubbleStatus> = {}) {
  let status: BubbleStatus = {
    enabled: false,
    running: false,
    permissions: { microphone: true, overlay: false, notifications: false },
    mode: "hold",
    showResult: true,
    showDiscard: false,
    pasteEnabled: false,
    accessibilityEnabled: false,
    autoStowSeconds: 0,
    ...initial,
  };
  const ready = () =>
    status.permissions.microphone && status.permissions.overlay && status.permissions.notifications;
  const grant = (permission: keyof BubbleStatus["permissions"]) => {
    status = { ...status, permissions: { ...status.permissions, [permission]: true } };
  };
  const control = {
    status: jest.fn(() => Promise.resolve(status)),
    configure: jest.fn((config: BubbleConfig) => {
      status = {
        ...status,
        enabled: config.enabled,
        mode: config.mode,
        showResult: config.showResult,
        showDiscard: config.showDiscard,
        pasteEnabled: config.pasteEnabled,
        autoStowSeconds: config.autoStowSeconds,
        running: config.enabled && ready(),
      };
      return Promise.resolve(status);
    }),
    requestMicrophone: jest.fn(() => {
      grant("microphone");
      return Promise.resolve(true);
    }),
    requestNotifications: jest.fn(() => {
      grant("notifications");
      return Promise.resolve(true);
    }),
    openOverlaySettings: jest.fn(() => {
      grant("overlay");
      return Promise.resolve();
    }),
    openAccessibilitySettings: jest.fn(() => Promise.resolve()),
  } satisfies BubbleControl;
  return control;
}

async function openBubble() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-bubble"));
  return screen.findByTestId("phone-bubble");
}

describe("设置 › 悬浮球 (boards 18a/18b)", () => {
  it("lists the permissions still missing, each with its way, before it can be switched on", async () => {
    const bubble = fakeBubble();
    await renderApp({ bubble });
    await openTab("settings");
    expect(await screen.findByTestId("settings-bubble")).toHaveTextContent(
      /悬浮球在其他应用上方显示说话按钮/,
    );
    await fireEvent.press(screen.getByTestId("settings-bubble"));
    expect(await screen.findByTestId("bubble-progress")).toHaveTextContent("已完成 1 / 3 项");
    expect(screen.getByTestId("bubble-toggle")).toBeDisabled();
    expect(screen.getByTestId("bubble-permission-microphone-granted")).toHaveTextContent("已允许");
    await fireEvent.press(screen.getByTestId("bubble-permission-overlay-grant"));
    expect(bubble.openOverlaySettings).toHaveBeenCalledTimes(1);
    expect(await screen.findByTestId("bubble-permission-overlay-granted")).toBeOnTheScreen();
    expect(screen.getByTestId("bubble-progress")).toHaveTextContent("已完成 2 / 3 项");
    await fireEvent.press(screen.getByTestId("bubble-permission-notifications-grant"));
    expect(bubble.requestNotifications).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(screen.getByTestId("bubble-toggle")).toBeEnabled();
    });
    expect(screen.queryByTestId("bubble-progress")).toBeNull();
    expect(screen.queryByTestId("bubble-permission-microphone")).toBeNull();
  });

  it("switches on with the dictionary's texts, and keeps its options", async () => {
    const bubble = fakeBubble({
      permissions: { microphone: true, overlay: true, notifications: true },
    });
    const { backend } = await renderApp({ bubble, mock: { usage: sampleUsage("ok") } });
    await openBubble();
    await fireEvent.press(await screen.findByTestId("bubble-toggle"));
    await waitFor(() => {
      expect(screen.getByTestId("bubble-toggle")).toBeChecked();
    });
    const config = bubble.configure.mock.calls[0]?.[0];
    expect(config).toMatchObject({
      enabled: true,
      mode: "hold",
      showResult: true,
      showDiscard: false,
    });
    expect(config?.strings.copied).toBe("已复制 · 长按输入框粘贴");
    expect(config?.strings.expand).toBe("展开悬浮球");
    expect(config?.strings.failed).toBe("未完成：{message}");
    expect(config?.strings.bodyHold).toBe("按住悬浮球说话，识别结果复制到剪贴板");
    expect(screen.getByTestId("bubble-toggle")).toHaveTextContent(/已开启 · 通知栏中可随时关闭/);
    // How to tuck it into the edge, under how to talk.
    expect(screen.getByTestId("bubble-stow-hint")).toHaveTextContent(
      "将悬浮球拖出屏幕边缘后松开，可收起为一个小把手；轻点把手或将它向内拉即可恢复。",
    );
    await fireEvent.press(screen.getByTestId("bubble-mode-tap"));
    await waitFor(() => {
      expect(bubble.configure).toHaveBeenLastCalledWith(
        expect.objectContaining({ enabled: true, mode: "tap", showResult: true }),
      );
    });
    await fireEvent.press(screen.getByTestId("bubble-show-result"));
    await waitFor(() => {
      expect(bubble.configure).toHaveBeenLastCalledWith(
        expect.objectContaining({ mode: "tap", showResult: false }),
      );
    });
    // AI 润色 is the same switch as 说话's and AI 模型's.
    await fireEvent.press(screen.getByTestId("bubble-refine"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(screen.getByTestId("bubble-usage")).toHaveTextContent(
      "使用内置服务时计入这部手机的用量 · 今日 23 / 60 分钟",
    );
    const guide = jest.spyOn(backend, "guideOpen");
    await fireEvent.press(screen.getByTestId("bubble-vendor"));
    expect(guide).toHaveBeenCalledWith("bubble", "zh-CN");
  });

  it("shows 不复制 beside the result only when asked, and only while the result shows", async () => {
    const bubble = fakeBubble({
      enabled: true,
      running: true,
      permissions: { microphone: true, overlay: true, notifications: true },
    });
    await renderApp({ bubble });
    await openBubble();
    const discard = await screen.findByTestId("bubble-show-discard");
    expect(discard).not.toBeChecked();
    expect(discard).toHaveTextContent(/显示「不复制」按钮在识别结果旁显示，点一下清空剪贴板/);
    await fireEvent.press(discard);
    await waitFor(() => {
      expect(bubble.configure).toHaveBeenLastCalledWith(
        expect.objectContaining({ enabled: true, showResult: true, showDiscard: true }),
      );
    });
    await waitFor(() => {
      expect(screen.getByTestId("bubble-show-discard")).toBeChecked();
    });
    // Without the result there is no card for it: the switch waits for 显示识别结果.
    await fireEvent.press(screen.getByTestId("bubble-show-result"));
    await waitFor(() => {
      expect(screen.getByTestId("bubble-show-discard")).toBeDisabled();
    });
    expect(bubble.configure).toHaveBeenLastCalledWith(
      expect.objectContaining({ showResult: false, showDiscard: true }),
    );
    expect(screen.getByTestId("bubble-show-discard")).toHaveTextContent(
      /打开「显示识别结果」后可用/,
    );
  });

  it("says so when the system did not let it start", async () => {
    const bubble = fakeBubble({
      permissions: { microphone: true, overlay: true, notifications: true },
    });
    bubble.configure.mockImplementationOnce((config: BubbleConfig) =>
      Promise.resolve({
        enabled: config.enabled,
        running: false,
        permissions: { microphone: true, overlay: true, notifications: true },
        mode: config.mode,
        showResult: config.showResult,
        showDiscard: config.showDiscard,
        pasteEnabled: config.pasteEnabled,
        accessibilityEnabled: false,
        autoStowSeconds: config.autoStowSeconds,
      }),
    );
    await renderApp({ bubble });
    await openBubble();
    await fireEvent.press(await screen.findByTestId("bubble-toggle"));
    expect(await screen.findByText("悬浮球未能开启，请稍后重试")).toBeOnTheScreen();
  });

  it("comes back with the app when it was on, in the app's language", async () => {
    const bubble = fakeBubble({
      enabled: true,
      mode: "tap",
      showDiscard: true,
      permissions: { microphone: true, overlay: true, notifications: true },
    });
    await renderApp({ bubble, language: "en", mock: { settings: { locale: "system" } } });
    await waitFor(() => {
      expect(bubble.configure).toHaveBeenCalledTimes(1);
    });
    const config = bubble.configure.mock.calls[0]?.[0];
    expect(config).toMatchObject({ enabled: true, mode: "tap", showDiscard: true });
    expect(config?.strings.stow).toBe("Tuck in at the side of the screen");
    expect(config?.strings.bodyTap).toBe(
      "Tap the bubble to talk; the result is copied to the clipboard",
    );
    await openTab("settings");
    expect(await screen.findByTestId("settings-bubble")).toHaveTextContent(/Floating bubbleOn/);
  });

  it("leaves a bubble that is off alone, and is not in 设置 without a native side", async () => {
    const bubble = fakeBubble();
    const { unmount } = await renderApp({ bubble });
    await openTab("settings");
    await screen.findByTestId("settings-bubble");
    await act(async () => {
      await Promise.resolve();
    });
    expect(bubble.configure).not.toHaveBeenCalled();
    await unmount();
    await renderApp();
    await openTab("settings");
    await screen.findByTestId("settings-speech");
    expect(screen.queryByTestId("settings-bubble")).toBeNull();
  });
});
