import { MockBackend, sampleDevices } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// The phone's own settings (user decision 2026-10-01: the phone works on its own with every setting
// but the local models; a take sent to a computer still follows the computer's settings).
describe("the phone's settings", () => {
  async function openSettings(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByTestId("tab-settings"));
    return screen.findByTestId("phone-settings");
  }

  it("the tab bar switches between talking and the settings, and each row opens its page", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    expect(await screen.findByTestId("tab-talk")).toHaveAttribute("aria-current", "page");
    const settings = await openSettings(user);
    expect(screen.getByRole("heading", { name: "设置", level: 1 })).toBeInTheDocument();
    expect(screen.getByTestId("tab-settings")).toHaveAttribute("aria-current", "page");
    expect(within(settings).getByTestId("settings-speech")).toHaveTextContent("语音模型");
    expect(within(settings).getByTestId("settings-ai")).toHaveTextContent("预设「校对」");
    expect(within(settings).getByTestId("settings-appearance")).toHaveTextContent("跟随系统");
    expect(within(settings).getByTestId("settings-recording")).toHaveTextContent("单次最长");
    expect(within(settings).getByTestId("settings-about")).toHaveTextContent("版本");
    expect(within(settings).getByTestId("settings-dictionary")).toHaveTextContent("0 条启用");
    expect(within(settings).getByTestId("settings-rules")).toHaveTextContent("0 条启用");
    expect(within(settings).getByTestId("settings-scenes")).toHaveTextContent(
      `${backend.peek().scenes.length} 个场景`,
    );
    // No paired computer: no device list row.
    expect(within(settings).queryByTestId("settings-devices")).toBeNull();
    for (const [row, heading] of [
      ["settings-speech", "语音模型"],
      ["settings-ai", "AI 模型与预设"],
      ["settings-appearance", "外观与语言"],
      ["settings-recording", "录音"],
      ["settings-about", "关于 Voltip"],
      ["settings-device", "本机"],
      ["settings-dictionary", "个人词典"],
      ["settings-rules", "替换规则"],
      ["settings-scenes", "场景"],
    ] as const) {
      await user.click(screen.getByTestId(row));
      expect(screen.getByRole("heading", { name: heading, level: 1 })).toBeInTheDocument();
      // A page has no tab bar; 返回 leads back to the settings.
      expect(screen.queryByTestId("tab-settings")).toBeNull();
      await user.click(screen.getByRole("button", { name: "返回" }));
      expect(screen.getByTestId("phone-settings")).toBeInTheDocument();
    }
    await user.click(screen.getByTestId("tab-talk"));
    expect(await screen.findByTestId("phone-mic")).toBeInTheDocument();
    backend.destroy();
  });

  it("with a paired computer the talk tab is the device list and the settings link to it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", devices: sampleDevices(1) });
    renderApp({ backend });
    expect(
      await screen.findByRole("heading", { name: "已配对设备", level: 1 }),
    ).toBeInTheDocument();
    const settings = await openSettings(user);
    await user.click(within(settings).getByTestId("settings-devices"));
    expect(screen.getByRole("heading", { name: "已配对设备", level: 1 })).toBeInTheDocument();
    expect(screen.getByTestId("tab-talk")).toHaveAttribute("aria-current", "page");
    backend.destroy();
  });

  it("the speech page lists the cloud providers, no on-device one, and saves the language and script", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await user.click(within(await openSettings(user)).getByTestId("settings-speech"));
    const page = screen.getByTestId("phone-speech");
    const cards = within(page).getByRole("list", { name: "语音识别服务商" });
    expect(within(cards).queryByTestId("provider-asr-local")).toBeNull();
    expect(within(cards).getByTestId("provider-asr-builtin")).toBeInTheDocument();
    expect(within(cards).getByTestId("provider-asr-groq")).toBeInTheDocument();
    await user.selectOptions(within(page).getByRole("combobox", { name: "识别语言" }), "en");
    await waitFor(() => {
      expect(backend.peek().settings.engines.language).toBe("en");
    });
    await user.selectOptions(within(page).getByRole("combobox", { name: "识别语言" }), "");
    await waitFor(() => {
      expect(backend.peek().settings.engines.language).toBeUndefined();
    });
    await user.click(within(page).getByRole("radio", { name: "繁体" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.chinese_script).toBe("traditional");
    });
    backend.destroy();
  });

  it("the AI page turns the polish off and on and holds the presets and the LLM providers", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await user.click(within(await openSettings(user)).getByTestId("settings-ai"));
    const page = screen.getByTestId("phone-ai");
    expect(within(page).getByTestId("presets-section")).toBeInTheDocument();
    expect(within(page).getByTestId("provider-llm-groq")).toBeInTheDocument();
    await user.click(within(page).getByRole("switch"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-ai")).toHaveTextContent("未开启 AI 润色");
    backend.destroy();
  });

  it("an AI polish that cannot run says why", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ mock: { role: "phone", builtIn: {} } });
    await user.click(within(await openSettings(user)).getByTestId("settings-ai"));
    expect(await screen.findByTestId("refine-not-ready")).toBeInTheDocument();
    backend.destroy();
  });

  it("the appearance page sets the language and the theme", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await user.click(within(await openSettings(user)).getByTestId("settings-appearance"));
    const page = screen.getByTestId("phone-appearance");
    await user.click(within(page).getByRole("radio", { name: "English" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("en");
    });
    expect(
      await screen.findByRole("heading", { name: "Appearance and language", level: 1 }),
    ).toBeInTheDocument();
    await user.click(within(page).getByRole("radio", { name: "简体中文" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("zh-cn");
    });
    // Following the system greys the tiles out; off again, a tile chooses the theme.
    expect(backend.peek().settings.follow_system_theme).toBe(false);
    await user.click(within(page).getByRole("switch"));
    await waitFor(() => {
      expect(backend.peek().settings.follow_system_theme).toBe(true);
    });
    await user.click(within(page).getByRole("switch"));
    await waitFor(() => {
      expect(backend.peek().settings.follow_system_theme).toBe(false);
    });
    const group = within(page).getByRole("radiogroup", { name: "主题" });
    await user.click(within(group).getAllByRole("radio")[1] as HTMLElement);
    await waitFor(() => {
      expect(backend.peek().settings.theme).toBe("dark");
    });
    backend.destroy();
  });

  it("the recording page sets the longest take", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await user.click(within(await openSettings(user)).getByTestId("settings-recording"));
    await user.selectOptions(screen.getByTestId("recording-max-minutes"), "60");
    await waitFor(() => {
      expect(backend.peek().settings.recording.max_minutes).toBe(60);
    });
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-recording")).toHaveTextContent("单次最长 1 小时");
    backend.destroy();
  });

  it("about names the version and the AGPL licence and opens the project's pages", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await user.click(within(await openSettings(user)).getByTestId("settings-about"));
    const page = screen.getByTestId("phone-about");
    expect(page).toHaveTextContent("AGPL-3.0-or-later");
    await user.click(within(page).getByRole("button", { name: "源代码" }));
    await user.click(within(page).getByRole("button", { name: "发布版本" }));
    expect(backend.linksOpened).toEqual(["source", "releases"]);
    vi.spyOn(backend, "projectLinkOpen").mockRejectedValueOnce(new Error("opener: 没有浏览器"));
    await user.click(within(page).getByRole("button", { name: "源代码" }));
    expect(await screen.findByText("出错了 · 没有浏览器")).toBeInTheDocument();
    backend.destroy();
  });
});
