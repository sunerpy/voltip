import {
  DEFAULT_HOTKEY,
  MAX_ACTIVATION_MS,
  createTranslator,
  defaultEngineSettings,
  zhT,
} from "@voltip/shared";
import {
  MOCK_HOTKEY_BACKEND,
  MockBackend,
  MOCK_CURRENT_VERSION,
  desktopIdentity,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import {
  EXTRA_RECORDING_RANGE,
  HOLD_THRESHOLD_RANGE,
  clampActivationMs,
  soloKeyLabel,
  soloKeyNotes,
} from "./Hotkey";

describe("Settings · 外观", () => {
  it("regression: picking a tile calls settings_set_theme and repaints <html>; follow-system disables tiles", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/appearance" });
    const group = await screen.findByRole("radiogroup", { name: "主题" });
    expect(within(group).getByRole("radio", { name: "明亮" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(group).getByRole("radio", { name: "暖纸" }));
    await waitFor(() => {
      expect(backend.peek().settings.theme).toBe("warm");
    });
    expect(document.documentElement.dataset.theme).toBe("warm");
    expect(await screen.findByText("主题已切换：暖纸")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^撤销$/ }));
    await waitFor(() => {
      expect(backend.peek().settings.theme).toBe("light");
    });
    await user.click(screen.getAllByRole("switch")[0] as HTMLElement);
    await waitFor(() => {
      expect(backend.peek().settings.follow_system_theme).toBe(true);
    });
    expect(within(group).getByRole("radio", { name: "石墨" })).toBeDisabled();
    expect(screen.getByText(/跟随系统 · 明亮/)).toBeInTheDocument();
    expect(screen.getByText("prefers-color-scheme: light")).toBeInTheDocument();
  });

  it("density, font size, overlay position and reduce motion write local appearance", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/settings/appearance" });
    await screen.findByRole("radiogroup", { name: "主题" });
    await user.click(screen.getByRole("radio", { name: "紧凑" }));
    expect(document.documentElement.dataset.density).toBe("compact");
    // The dialog header carries the appearance readouts; the toolbar keeps the page beneath.
    expect(screen.getByTestId("settings-readouts")).toHaveTextContent("密度 紧凑 · 14 px");
    await user.click(screen.getByRole("button", { name: "增大字号" }));
    expect(screen.getByTestId("font-size")).toHaveTextContent("15 px");
    for (let i = 0; i < 5; i += 1)
      await user.click(screen.getByRole("button", { name: "减小字号" }));
    expect(screen.getByTestId("font-size")).toHaveTextContent("12 px");
    expect(screen.getByRole("button", { name: "减小字号" })).toBeDisabled();
    for (let i = 0; i < 7; i += 1)
      await user.click(screen.getByRole("button", { name: "增大字号" }));
    expect(screen.getByRole("button", { name: "增大字号" })).toBeDisabled();
    await user.click(screen.getByRole("radio", { name: "顶部" }));
    expect(screen.getByRole("radio", { name: "顶部" })).toHaveAttribute("aria-checked", "true");
    const switches = screen.getAllByRole("switch");
    await user.click(switches[1] as HTMLElement);
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
    await user.click(screen.getByRole("button", { name: "恢复默认" }));
    expect(screen.getByTestId("font-size")).toHaveTextContent("14 px");
    expect(screen.getByTestId("preview-strip")).toBeInTheDocument();
  });

  it("regression: the preview strip shows the saved hotkey, the engine's readiness and the last take's latency, not fixed samples", async () => {
    const backend = new MockBackend({ history: [] });
    renderApp({ path: "/settings/appearance", backend });
    const strip = await screen.findByTestId("preview-strip");
    expect(strip).not.toHaveTextContent("412");
    expect(within(strip).getByTestId("preview-latency")).toHaveTextContent("暂无听写记录");
    const hotkey = backend.peek().settings.hotkey;
    expect(within(strip).getByLabelText(hotkey.split("+").join(" "))).toBeInTheDocument();
    expect(strip).toHaveTextContent(backend.peek().engines.asr_ready ? "就绪" : "未就绪");
    await act(async () => {
      await backend.invoke("dictation_start");
      await backend.invoke("dictation_stop");
    });
    await waitFor(() => {
      expect(backend.peek().history).toHaveLength(1);
    });
    const entry = backend.peek().history[0];
    if (!entry) throw new Error("take");
    expect(within(strip).getByTestId("preview-latency")).toHaveTextContent(
      `上次延迟 ${entry.asr_ms + (entry.refine_ms ?? 0)} ms`,
    );
  });

  it("regression: the pill's placement is a core setting the shell reads, the same on every platform", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ identity: { ...desktopIdentity(), platform: "linux" } });
    renderApp({ path: "/settings/appearance", backend });
    const group = await screen.findByRole("radiogroup", { name: "悬浮窗位置" });
    expect(within(group).getByRole("radio", { name: "底部" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // The Linux note claimed an off default no code implemented.
    expect(screen.queryByText(/默认关闭/)).toBeNull();
    await user.click(within(group).getByRole("radio", { name: "关闭" }));
    expect(backend.peek().settings.overlay).toBe("off");
    expect(within(group).getByRole("radio", { name: "关闭" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(window.localStorage.getItem("voltip.appearance") ?? "").not.toContain("overlay");
    await user.click(screen.getByRole("button", { name: "恢复默认" }));
    await waitFor(() => {
      expect(backend.peek().settings.overlay).toBe("bottom");
    });
  });

  it("shows real 语音模型 / AI 模型 / 通用 / 隐私与历史 / 关于 panes without sample footnotes", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/appearance",
      backend: new MockBackend({ identity: { ...desktopIdentity(), platform: "linux" } }),
    });
    await screen.findByRole("radiogroup", { name: "主题" });
    expect(screen.queryByTestId("sample-data-notice")).toBeNull();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    await user.click(screen.getByRole("tab", { name: /隐私与历史/ }));
    expect(screen.getByRole("heading", { name: "隐私与历史", level: 2 })).toBeInTheDocument();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(screen.getByTestId("privacy-pane")).toBeInTheDocument();
    // 通用 and 关于 are real now: language + auto-update, and the version facts + update status.
    await user.click(screen.getByRole("tab", { name: /通用/ }));
    expect(screen.getByRole("heading", { name: "通用", level: 2 })).toBeInTheDocument();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(screen.getByRole("radiogroup", { name: "语言" })).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /关于/ }));
    expect(screen.getByRole("heading", { name: "关于", level: 2 })).toBeInTheDocument();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(screen.getByText("Apache-2.0")).toBeInTheDocument();
    expect(screen.getByText(MOCK_CURRENT_VERSION)).toBeInTheDocument();
    expect(screen.getByTestId("model-sources")).toHaveTextContent(
      "handy-computer/Qwen3-ASR-0.6B-gguf",
    );
    expect(screen.queryByText(/私有|All rights reserved|AGPL/)).toBeNull();
    expect(screen.getByTestId("update-status")).toHaveTextContent("尚未检查更新");
    // The repository opens through the shell (the webview names the page); 反馈 is the same
    // dialog the sidebar opens, in this dialog's place, never the issue tracker (user feedback and
    // decision 2026-09-28).
    await user.click(screen.getByRole("button", { name: "源代码" }));
    expect(backend.linksOpened).toEqual(["source"]);
    await user.click(screen.getByTestId("about-feedback"));
    expect(await screen.findByRole("dialog", { name: "反馈" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    expect(backend.linksOpened).toEqual(["source"]);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "反馈" })).toBeNull();
    await user.click(screen.getByTestId("sidebar-settings"));
    // 语音模型 and AI 模型 are pages since 2026-09-28: no group of the dialog names them.
    expect(screen.queryByRole("tab", { name: /语音模型|AI 模型|润色/ })).toBeNull();
    await user.keyboard("{Escape}");
    // The 语音模型 page is the recognition half of the former engines pane: the provider cards
    // read the core's engines, the local model manager sits underneath; the LLM half is AI 模型.
    await user.click(screen.getByRole("button", { name: /^语音模型$/ }));
    const pane = await screen.findByTestId("page-speech");
    expect(within(pane).getByRole("heading", { name: "语音模型", level: 2 })).toBeInTheDocument();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(within(pane).getByRole("article", { name: "内置服务" })).toBeInTheDocument();
    expect(within(pane).getByRole("article", { name: "本机" })).toBeInTheDocument();
    expect(within(pane).getAllByText("Qwen/Qwen3-ASR-1.7B").length).toBeGreaterThan(0);
    expect(within(pane).getByTestId("providers-asr")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^AI 模型$/ }));
    const ai = await screen.findByTestId("page-ai");
    expect(within(ai).getByRole("heading", { name: "AI 模型", level: 2 })).toBeInTheDocument();
    expect(within(ai).getByTestId("providers-llm")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/第二阶段|示例数据|计划中/);
  });
});

describe("Settings · 对话框", () => {
  it("regression: settings open as a modal dialog over the previous page and close back to it", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    expect(screen.queryByTestId("page-background")).toBeNull();

    await user.keyboard("{Control>},{/Control}");
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    // The home page stays mounted beneath, inert and hidden from assistive tech.
    const beneath = screen.getByTestId("page-background");
    expect(beneath).not.toBeEmptyDOMElement();
    expect(beneath).toHaveAttribute("aria-hidden", "true");
    expect(beneath).toHaveAttribute("inert");
    expect(within(dialog).getByRole("tab", { name: /外观/, selected: true })).toHaveFocus();
    expect(within(dialog).getByRole("heading", { name: "外观", level: 2 })).toBeInTheDocument();
    expect(within(dialog).getByTestId("settings-readouts")).toHaveTextContent(
      "主题 明亮 · 不跟随系统 · 密度 默认 · 14 px",
    );

    // ↑/↓ move between groups through the route; the content follows.
    await user.keyboard("{ArrowDown}");
    expect(within(dialog).getByRole("tab", { name: /关于/, selected: true })).toHaveFocus();
    expect(within(dialog).getByRole("heading", { name: "关于", level: 2 })).toBeInTheDocument();
    await user.keyboard("{ArrowDown}");
    expect(within(dialog).getByRole("tab", { name: /通用/, selected: true })).toBeInTheDocument();
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(within(dialog).getByRole("tab", { name: /外观/, selected: true })).toBeInTheDocument();

    // Clicking inside keeps it open; Esc closes back to the page beneath.
    await user.click(within(dialog).getByRole("heading", { name: "外观", level: 2 }));
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    expect(screen.queryByTestId("page-background")).toBeNull();
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();

    // From another page the dialog remembers that page: the sidebar opens it, 关闭 returns.
    await user.click(screen.getByRole("button", { name: /^手机$/ }));
    await user.click(screen.getByRole("button", { name: /^设置$/ }));
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    expect(screen.getByRole("heading", { name: "手机", level: 1 })).toBeInTheDocument();

    // The scrim closes too.
    await user.keyboard("{Control>},{/Control}");
    await user.click(screen.getByTestId("settings-scrim"));
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    expect(screen.getByRole("heading", { name: "手机", level: 1 })).toBeInTheDocument();
  });

  it("regression: a deep link to /settings/hotkey renders home beneath the dialog", async () => {
    renderApp({ path: "/settings/hotkey" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("tab", { name: /快捷键/, selected: true })).toBeInTheDocument();
    expect(within(dialog).getByTestId("hotkey-recorder")).toBeInTheDocument();
    // The method readout is the shell's own report (the mock's here), in plain words.
    expect(within(dialog).getByTestId("settings-readouts")).toHaveTextContent(
      `快捷键 Ctrl Alt Space · 快捷键方式 ${MOCK_HOTKEY_BACKEND}`,
    );
    expect(screen.getByTestId("page-background")).not.toBeEmptyDOMElement();
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
  });

  it("regression: the settings dialog is fluid (no fixed 640 px column)", async () => {
    renderApp({ path: "/settings/appearance" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(dialog.className).toMatch(/w-\[min\(960px,calc\(100vw-48px\)\)\]/);
    expect(dialog.className).toMatch(/h-\[min\(660px,calc\(100vh-48px\)\)\]/);
    const nav = within(dialog).getByRole("tablist", { name: "设置分组" }).closest("nav");
    expect(nav?.className).toMatch(/w-\[200px\]/);
    const content = within(dialog).getByTestId("settings-content");
    expect(content.className).toMatch(/min-h-0 overflow-auto p-6|min-h-0 flex-1 overflow-auto p-6/);
    const fixed = [content, ...content.querySelectorAll("*")].filter((el) =>
      /w-\[640px\]|w-\[200px\]/.test(el.className),
    );
    expect(fixed).toHaveLength(0);
    expect(content.firstElementChild?.className).toMatch(/max-w-\[720px\]/);
  });

  it("regression: Esc while recording cancels the recording without closing the dialog", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    const recorder = await screen.findByTestId("hotkey-recorder");
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    expect(recorder).toHaveAttribute("data-recording", "true");
    await user.keyboard("{Escape}");
    expect(recorder).toHaveAttribute("data-recording", "false");
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    // A chord recorded while listening never reaches the shell's Ctrl H shortcut.
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    await user.keyboard("{Control>}h{/Control}");
    await waitFor(() => {
      expect(backend.peek().settings.hotkey).toBe("Ctrl+H");
    });
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "历史记录", level: 1 })).toBeNull();
    // Esc after recording closes the dialog as usual.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
  });
});

describe("Settings · 听写", () => {
  it("the insert setting writes settings_set_engines { inject } with the rest of the block unchanged", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/dictation" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("tab", { name: /听写/, selected: true })).toBeInTheDocument();
    const pane = within(dialog).getByTestId("dictation-pane");
    expect(within(pane).getByRole("radio", { name: "粘贴到光标处" })).toBeChecked();
    await user.click(within(pane).getByRole("radio", { name: "仅复制到剪贴板" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual({
        ...defaultEngineSettings(),
        inject: "clipboard_only",
      });
    });
    expect(within(pane).getByRole("radio", { name: "仅复制到剪贴板" })).toBeChecked();
    // It sits right after 快捷键 in the group list.
    const tabs = within(dialog)
      .getAllByRole("tab")
      .map((tab) => tab.textContent ?? "");
    expect(tabs.findIndex((name) => name.includes("听写"))).toBe(
      tabs.findIndex((name) => name.includes("快捷键")) + 1,
    );
  });
});

describe("Settings · 热键", () => {
  it("regression: recording a chord saves it through settings_set_hotkey and shows the shell's registration; single keys are refused; Esc cancels; defaults restore", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    const recorder = await screen.findByTestId("hotkey-recorder");
    expect(recorder).toHaveAttribute("data-recording", "false");
    expect(screen.queryByTestId("sample-data-notice")).toBeNull();
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
    // The activation cards are real (docs/dictation.md §13): hold is the default, all three enabled.
    const modes = screen.getByRole("listbox", { name: "录音方式" });
    expect(within(modes).getByRole("option", { name: "按住说话" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(modes).getAllByRole("option")).toHaveLength(3);
    expect(within(modes).queryByText(/尚未接入/)).toBeNull();
    expect(screen.getByTestId("hotkey-status")).toHaveTextContent("已保存 · 快捷键已生效");
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    expect(recorder).toHaveAttribute("data-recording", "true");
    // While the recorder is open the shell has suspended the OS registration.
    await waitFor(() => {
      expect(backend.peek().hotkey.capturing).toBe(true);
    });
    // Modifier only → refused on release; the recorder closes and must be reopened.
    await user.keyboard("{Control>}{/Control}");
    expect(await screen.findByText(/只按了修饰键/)).toBeInTheDocument();
    expect(recorder).toHaveAttribute("data-recording", "false");
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    await user.keyboard("x");
    expect(await screen.findByText(/不允许单个按键作为快捷键/)).toBeInTheDocument();
    expect(backend.peek().settings.hotkey).toBe(DEFAULT_HOTKEY);
    // Physical keys and the peak set: Shift released first still records Ctrl+Shift+D.
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    await user.keyboard("{Control>}{Shift>}{d>}");
    expect(within(recorder).getByText("Shift")).toBeInTheDocument();
    await user.keyboard("{/Shift}{/d}{/Control}");
    expect(recorder).toHaveAttribute("data-recording", "false");
    await waitFor(() => {
      expect(backend.peek().hotkey.capturing).toBe(false);
    });
    // The chord went to the core (`settings.hotkey`) and the shell re-registered it.
    await waitFor(() => {
      expect(backend.peek().settings.hotkey).toBe("Ctrl+Shift+D");
    });
    expect(backend.peek().hotkey.registered).toBe("Ctrl+Shift+D");
    expect(screen.getByTestId("hotkey-status")).toHaveTextContent("已保存 · 快捷键已生效");
    expect(within(recorder).getByText("Ctrl")).toBeInTheDocument();
    expect(within(recorder).getByText("D")).toBeInTheDocument();
    expect(screen.getByText("当前快捷键：Ctrl+Shift+D")).toBeInTheDocument();
    expect(screen.queryByText(/已被另一个 X11 客户端占用/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    await user.keyboard("{Escape}");
    expect(recorder).toHaveAttribute("data-recording", "false");
    expect(backend.peek().settings.hotkey).toBe("Ctrl+Shift+D");
    await user.click(screen.getByRole("button", { name: "恢复默认" }));
    await waitFor(() => {
      expect(backend.peek().settings.hotkey).toBe(DEFAULT_HOTKEY);
    });
    expect(within(recorder).getByText("Space")).toBeInTheDocument();
    // The hint under the cards spells the saved chord.
    expect(screen.getByTestId("activation-hint")).toHaveTextContent(
      "按住 Ctrl Alt Space 说一句，松开即插入",
    );
  });

  it("regression: the activation cards write settings_set_activation with all three values; the threshold field appears only for 按住或按一下, the tail field always, both snap to the 50 ms grid and clamp to the core's cap; the hint, the home chip and the footer follow the mode", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    await screen.findByTestId("hotkey-recorder");
    const modes = screen.getByRole("listbox", { name: "录音方式" });
    expect(screen.queryByTestId("hold-threshold")).toBeNull();
    const extra = screen.getByTestId("extra-recording");
    expect(extra).toHaveValue(0);
    expect(screen.getByText("立即停止")).toBeInTheDocument();
    // hold → toggle: one command, the other two values untouched.
    await user.click(within(modes).getByRole("option", { name: "按一下开始，再按一下结束" }));
    await waitFor(() => {
      expect(backend.peek().settings.activation).toBe("toggle");
    });
    expect(backend.peek().settings.hold_threshold_ms).toBe(300);
    expect(backend.peek().settings.extra_recording_ms).toBe(0);
    expect(within(modes).getByRole("option", { name: "按一下开始，再按一下结束" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByTestId("activation-hint")).toHaveTextContent(
      "按一下 Ctrl Alt Space 开始，再按一下结束",
    );
    expect(screen.queryByTestId("hold-threshold")).toBeNull();
    // Re-selecting the current mode sends nothing.
    const writes = () => backend.log.filter((e) => e.type === "settings").length;
    const before = writes();
    await user.click(within(modes).getByRole("option", { name: "按一下开始，再按一下结束" }));
    expect(writes()).toBe(before);
    // hold_or_toggle shows the threshold; Enter commits a snapped value.
    await user.click(within(modes).getByRole("option", { name: "按住或按一下" }));
    await waitFor(() => {
      expect(backend.peek().settings.activation).toBe("hold_or_toggle");
    });
    const threshold = screen.getByTestId("hold-threshold");
    expect(threshold).toHaveValue(300);
    expect(threshold).toHaveAttribute("min", "50");
    expect(threshold).toHaveAttribute("max", String(MAX_ACTIVATION_MS));
    expect(threshold).toHaveAttribute("step", "50");
    await user.clear(threshold);
    await user.type(threshold, "470{Enter}");
    await waitFor(() => {
      expect(backend.peek().settings.hold_threshold_ms).toBe(450);
    });
    expect(threshold).toHaveValue(450);
    expect(backend.peek().settings.activation).toBe("hold_or_toggle");
    // Above the cap the UI clamps before the core ever sees it: no error event.
    await user.clear(threshold);
    await user.type(threshold, "99999");
    await user.tab();
    await waitFor(() => {
      expect(backend.peek().settings.hold_threshold_ms).toBe(MAX_ACTIVATION_MS);
    });
    expect(backend.log.filter((e) => e.type === "error")).toHaveLength(0);
    // The tail field: blur commits, 0 reads 立即停止, a value hides it.
    await user.clear(extra);
    await user.type(extra, "180");
    await user.tab();
    await waitFor(() => {
      expect(backend.peek().settings.extra_recording_ms).toBe(200);
    });
    expect(screen.queryByText("立即停止")).toBeNull();
    // Garbage keeps the value; nothing is written.
    const settingsWrites = writes();
    await user.clear(extra);
    await user.type(extra, "-");
    await user.tab();
    expect(extra).toHaveValue(200);
    expect(writes()).toBe(settingsWrites);
    expect(screen.getByTestId("activation-hint")).toHaveTextContent(
      "按住 Ctrl Alt Space 说话，短按锁定",
    );
    // The chip on the home page and the footer follow the mode (the dialog floats over home).
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: "按住说话 · 短按锁定" })).toBeInTheDocument();
    expect(screen.getByText("按住或按一下听写")).toBeInTheDocument();
    expect(screen.queryByText("按住听写")).toBeNull();
    // Pure helper: snap, clamp, fall back.
    expect(clampActivationMs("470", HOLD_THRESHOLD_RANGE, 300)).toBe(450);
    expect(clampActivationMs("10", HOLD_THRESHOLD_RANGE, 300)).toBe(50);
    expect(clampActivationMs("0", EXTRA_RECORDING_RANGE, 100)).toBe(0);
    expect(clampActivationMs("99999", EXTRA_RECORDING_RANGE, 100)).toBe(MAX_ACTIVATION_MS);
    expect(clampActivationMs("abc", EXTRA_RECORDING_RANGE, 100)).toBe(100);
    expect(clampActivationMs("", EXTRA_RECORDING_RANGE, 100)).toBe(100);
    expect(clampActivationMs("125", EXTRA_RECORDING_RANGE, 0)).toBe(150);
  });

  it("regression: a Windows identity never shows the linux_x11 fixture as its backend; the readout is the shell's own report", async () => {
    renderApp({ path: "/settings/hotkey" });
    await screen.findByTestId("hotkey-recorder");
    const readout = screen.getByTestId("hotkey-backend");
    expect(readout).toHaveTextContent("Windows");
    expect(readout).toHaveTextContent(MOCK_HOTKEY_BACKEND);
    expect(readout).not.toHaveTextContent("linux_x11");
    expect(readout).not.toHaveTextContent("X11");
    expect(screen.queryByText("X11 · XGrabKey")).toBeNull();
    // Regression (public release, 2026-09-27): no fixture capability matrix — only what the shell
    // reports about the backend in use.
    expect(screen.queryByRole("table")).toBeNull();
    expect(document.body.textContent).not.toMatch(/windows_ll_hook|macos_event_tap|单独按住修饰键/);
  });

  it("regression: the hotkey pane says what the hotkey can do in this session, and on pure Wayland gives the shortcut command instead", async () => {
    const user = userEvent.setup();
    const copied: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: (text: string) => {
          copied.push(text);
          return Promise.resolve();
        },
      },
      configurable: true,
    });
    const { backend } = renderApp({ path: "/settings/hotkey" });
    await screen.findByTestId("hotkey-recorder");
    const caps = screen.getByTestId("hotkey-capabilities");
    expect(within(caps).getByTestId("capability-global")).toHaveTextContent("可用");
    expect(within(caps).getByTestId("capability-everywhere")).toHaveTextContent(
      "任何窗口有焦点时都生效",
    );
    expect(within(caps).getByTestId("capability-hold")).toHaveTextContent("支持");
    act(() => {
      backend.publish({
        type: "hotkey",
        error: "Ctrl+Alt+Space 未能生效：纯 Wayland 会话不允许应用设置全局快捷键",
        pressed: false,
        capturing: false,
        backend: "global-shortcut · Linux · Wayland",
        capabilities: {
          global: false,
          everywhere: false,
          hold: false,
          toggle_command: "/usr/bin/voltip-desktop --toggle",
          edit_toggle_command: "/usr/bin/voltip-desktop --edit-toggle",
          solo_keys: [],
        },
      });
    });
    await waitFor(() => {
      expect(within(caps).getByTestId("capability-global")).toHaveTextContent(
        "当前会话不允许应用设置全局快捷键",
      );
    });
    // No input hook on pure Wayland: the single-key row says so and offers nothing.
    const solo = screen.getByTestId("solo-key");
    expect(solo).toHaveTextContent("当前会话不支持单键触发");
    expect(within(solo).getByRole("combobox", { name: "单键触发的按键" })).toBeDisabled();
    expect(within(caps).getByTestId("capability-hold")).toHaveTextContent("不支持");
    expect(within(caps).getByTestId("capability-command")).toHaveTextContent(
      "/usr/bin/voltip-desktop --toggle",
    );
    await user.click(within(caps).getByRole("button", { name: "复制听写命令" }));
    await user.click(within(caps).getByRole("button", { name: "复制语音编辑命令" }));
    expect(copied).toEqual([
      "/usr/bin/voltip-desktop --toggle",
      "/usr/bin/voltip-desktop --edit-toggle",
    ]);
    // Still no fixture matrix: rows about this session only.
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("names each lone key for its platform and says what to know before using it", () => {
    const { t } = zhT;
    expect(soloKeyLabel("right_meta", "macos", t)).toBe("右 Command");
    expect(soloKeyLabel("right_meta", "windows", t)).toBe("右 Win");
    expect(soloKeyLabel("right_meta", "linux", t)).toBe("右 Super");
    expect(soloKeyLabel("right_alt", "macos", t)).toBe("右 Option");
    expect(soloKeyLabel("fn", "macos", t)).toBe("Fn（🌐）");
    expect(soloKeyLabel("right_ctrl", "windows", createTranslator("en").t)).toBe("Right Ctrl");
    expect(soloKeyNotes(null, "windows", true, t)).toEqual([]);
    expect(soloKeyNotes("right_ctrl", "windows", true, t)).toEqual([]);
    expect(soloKeyNotes("right_alt", "windows", true, t).join()).toContain("AltGr");
    expect(soloKeyNotes("right_alt", "macos", true, t).join()).not.toContain("AltGr");
    expect(soloKeyNotes("fn", "macos", true, t).join()).toContain("不执行任何操作");
    expect(soloKeyNotes("mouse_forward", "linux", false, t).join()).toContain("XWayland");
  });

  it("regression: section 13.1 the single-key trigger saves through settings_set_solo_key and shows what the shell watches, presses and refuses", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    const row = await screen.findByTestId("solo-key");
    const select = within(row).getByRole("combobox", { name: "单键触发的按键" });
    expect(select).toHaveValue("off");
    // The session's keys, named for the platform (the mock is a Windows session: no Fn).
    const labels = within(select)
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(labels).toEqual([
      "关闭",
      "右 Ctrl",
      "右 Alt",
      "右 Shift",
      "右 Win",
      "鼠标中键",
      "鼠标侧键 · 后退",
      "鼠标侧键 · 前进",
    ]);
    await user.selectOptions(select, "mouse_back");
    await waitFor(() => {
      expect(backend.peek().settings.solo_key).toBe("mouse_back");
    });
    expect(within(row).getByTestId("solo-key-status")).toHaveTextContent("已生效");
    expect(within(row).getByTestId("solo-key-notes")).toHaveTextContent("由 Voltip 独占");
    // The shell reports the key held down on its own.
    act(() => {
      backend.publish({ type: "hotkey", ...backend.peek().hotkey, solo_pressed: true });
    });
    expect(within(row).getByTestId("solo-key-status")).toHaveTextContent("按下中");
    // A key the hook could not watch: the shell's reason, as an alert.
    act(() => {
      backend.publish({
        type: "hotkey",
        ...backend.peek().hotkey,
        solo_registered: undefined,
        solo_pressed: false,
        solo_error: "鼠标后退键 无法单独触发：另一个程序已经占用了这个鼠标键",
      });
    });
    expect(within(row).getByTestId("solo-key-status")).toHaveTextContent("未生效");
    expect(within(row).getByRole("alert")).toHaveTextContent("另一个程序已经占用了这个鼠标键");
    await user.selectOptions(select, "off");
    await waitFor(() => {
      expect(backend.peek().settings.solo_key).toBeNull();
    });
    expect(within(row).queryByTestId("solo-key-status")).toBeNull();
  });

  it("regression: the conflict banner comes from the shell's registration error and its action starts recording", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    await screen.findByTestId("hotkey-recorder");
    expect(screen.queryByRole("button", { name: "更换快捷键" })).toBeNull();
    act(() => {
      backend.publish({
        type: "hotkey",
        error: "Ctrl+Alt+Space 注册失败：HotKey already registered",
        pressed: false,
        capturing: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    expect(await screen.findByText(/HotKey already registered/)).toBeInTheDocument();
    expect(screen.getByTestId("hotkey-status")).toHaveTextContent("已保存 · 未能生效");
    expect(screen.getByTestId("hotkey-backend")).toHaveTextContent("未生效");
    await user.click(screen.getByRole("button", { name: "更换快捷键" }));
    expect(screen.getByTestId("hotkey-recorder")).toHaveAttribute("data-recording", "true");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", altKey: true }));
    });
    expect(screen.getByTestId("hotkey-recorder")).toHaveAttribute("data-recording", "true");
    // A successful re-registration clears the banner.
    act(() => {
      backend.publish({
        type: "hotkey",
        registered: DEFAULT_HOTKEY,
        pressed: true,
        capturing: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "更换快捷键" })).toBeNull();
    });
    expect(screen.getByTestId("hotkey-backend")).toHaveTextContent("已生效 · 按下中");
  });

  it("regression: the voice-edit chord has its own recorder that saves through settings_set_edit_hotkey and never runs alongside the dictation recorder and turns off and on and refuses the dictation chord and shows the shell registration (section 19)", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/hotkey" });
    const edit = await screen.findByTestId("edit-hotkey-recorder");
    const dictation = screen.getByTestId("hotkey-recorder");
    expect(screen.getByText("编辑选中文本")).toBeInTheDocument();
    expect(screen.getByText(/按这个快捷键说出修改指令/)).toBeInTheDocument();
    expect(within(edit).getByText("E")).toBeInTheDocument();
    expect(screen.getByTestId("edit-hotkey-status")).toHaveTextContent("已保存 · 快捷键已生效");
    // The built-in refine key is there: no warning.
    expect(screen.queryByText(/需要 AI 润色服务/)).toBeNull();
    // Recording the edit chord closes the dictation recorder (one recorder at a time).
    await user.click(screen.getByRole("button", { name: "重新录制" }));
    expect(dictation).toHaveAttribute("data-recording", "true");
    await user.click(screen.getByRole("button", { name: "重新录制 · 编辑选中文本" }));
    expect(dictation).toHaveAttribute("data-recording", "false");
    expect(edit).toHaveAttribute("data-recording", "true");
    await waitFor(() => {
      expect(backend.peek().hotkey.capturing).toBe(true);
    });
    await user.keyboard("{Control>}{Alt>}{Shift>}{e>}");
    await user.keyboard("{/e}{/Shift}{/Alt}{/Control}");
    expect(edit).toHaveAttribute("data-recording", "false");
    await waitFor(() => {
      expect(backend.peek().settings.edit_hotkey).toBe("Ctrl+Alt+Shift+E");
    });
    expect(backend.peek().settings.hotkey).toBe(DEFAULT_HOTKEY);
    expect(backend.peek().hotkey).toMatchObject({
      capturing: false,
      edit_registered: "Ctrl+Alt+Shift+E",
    });
    expect(within(edit).getByText("Shift")).toBeInTheDocument();
    // The dictation chord is refused with the core's reason; the edit chord stays.
    await user.click(screen.getByRole("button", { name: "重新录制 · 编辑选中文本" }));
    await user.keyboard("{Control>}{Alt>}{ >}{/ }{/Alt}{/Control}");
    await waitFor(() => {
      expect(
        backend.log.some((e) => e.type === "error" && e.message.includes("已用作听写快捷键")),
      ).toBe(true);
    });
    expect(backend.peek().settings.edit_hotkey).toBe("Ctrl+Alt+Shift+E");
    // Off, and on again with the default chord.
    await user.click(screen.getByRole("button", { name: "关闭 · 编辑选中文本" }));
    await waitFor(() => {
      expect(backend.peek().settings.edit_hotkey).toBeNull();
    });
    expect(screen.getByTestId("edit-hotkey-status")).toHaveTextContent("已关闭");
    expect(backend.peek().hotkey.edit_registered).toBeUndefined();
    await user.click(screen.getByRole("button", { name: "启用 · 编辑选中文本" }));
    await waitFor(() => {
      expect(backend.peek().settings.edit_hotkey).toBe("Ctrl+Alt+E");
    });
    expect(screen.getByTestId("edit-hotkey-status")).toHaveTextContent("已保存 · 快捷键已生效");
    // The shell could not register it: the row says so and the banner offers a new chord.
    act(() => {
      backend.publish({
        type: "hotkey",
        registered: DEFAULT_HOTKEY,
        edit_error: "Ctrl+Alt+E 注册失败：HotKey already registered",
        pressed: false,
        capturing: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    expect(await screen.findByText(/Ctrl\+Alt\+E 注册失败/)).toBeInTheDocument();
    expect(screen.getByTestId("edit-hotkey-status")).toHaveTextContent("已保存 · 未能生效");
    expect(screen.getByTestId("hotkey-status")).toHaveTextContent("已保存 · 快捷键已生效");
    await user.click(screen.getByRole("button", { name: "更换快捷键" }));
    expect(edit).toHaveAttribute("data-recording", "true");
    expect(dictation).toHaveAttribute("data-recording", "false");
    await user.keyboard("{Escape}");
    expect(edit).toHaveAttribute("data-recording", "false");
  });

  it("regression: without a refine key the edit row says it needs the AI refine service (section 19)", async () => {
    const backend = new MockBackend({
      settings: { engines: { ...defaultEngineSettings(), llm_provider: "groq" } },
    });
    renderApp({ path: "/settings/hotkey", backend });
    await screen.findByTestId("edit-hotkey-recorder");
    expect(screen.getByText("需要 AI 润色服务：先在「AI 模型」页配置服务商")).toBeInTheDocument();
  });
});
