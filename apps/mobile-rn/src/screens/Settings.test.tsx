// 设置 on the phone: each row's line says what the page behind it is set to.
import { act, screen, within } from "@testing-library/react-native";
import { defaultProxySettings, defaultSettings } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

const row = (id: string) => within(screen.getByTestId(`settings-${id}`));

describe("设置's rows", () => {
  it("say a theme that follows the system, history off, a manual proxy and a scene used for every take", async () => {
    const settings = defaultSettings();
    const backend = new MockBackend({
      role: "phone",
      settings: {
        follow_system_theme: true,
        history: { ...settings.history, enabled: false },
        proxy: { ...defaultProxySettings(), mode: "manual", server: "proxy.example.test:8080" },
      },
    });
    await renderApp({ backend });
    await openTab("settings");
    await screen.findByTestId("phone-settings");
    expect(row("appearance").getByText(/ · 跟随系统$/)).toBeOnTheScreen();
    expect(row("historySettings").getByText("不保存新的记录")).toBeOnTheScreen();
    expect(row("network").getByText("代理 proxy.example.test:8080")).toBeOnTheScreen();
    expect(row("scenes").getByText(/^\d+ 个场景$/)).toBeOnTheScreen();
    const chat = backend.peek().scenes.find((s) => s.builtin === "chat");
    await act(async () => {
      await backend.invoke("settings_set_pinned_scene", { id: chat?.id ?? null });
    });
    expect(await row("scenes").findByText("说话时使用「即时聊天」")).toBeOnTheScreen();
  });

  it("say no proxy and the AI polish switched off", async () => {
    const settings = defaultSettings();
    await renderApp({
      mock: {
        settings: {
          proxy: { ...defaultProxySettings(), mode: "off" },
          engines: { ...settings.engines, refine_enabled: false },
        },
      },
    });
    await openTab("settings");
    await screen.findByTestId("phone-settings");
    expect(row("network").getByText("不使用代理")).toBeOnTheScreen();
    expect(row("ai").getByText("未开启 AI 润色")).toBeOnTheScreen();
  });
});
