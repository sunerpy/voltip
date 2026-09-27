import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MOCK_ASR_MS, MOCK_REFINE_MS, MockBackend } from "@voltip/shared/mock";
import { TITLE_BAR_SEARCH_LABEL } from "@voltip/ui";
import { renderApp } from "../test/render";

function paletteInput() {
  return within(screen.getByRole("dialog", { name: "命令菜单" })).getByRole("combobox");
}

describe("Shell", () => {
  it("regression: a phone that unpaired this computer is announced; it has left the device list", async () => {
    const { backend } = renderApp({ path: "/devices" });
    await screen.findByRole("heading", { level: 1 });
    const phone = backend.peek().devices[0]?.device;
    if (phone === undefined) throw new Error("fixture has no paired device");
    act(() => {
      backend.publish({ type: "unpaired", ...phone });
    });
    expect(await screen.findByText(`「${phone.name}」解除了与这台设备的配对`)).toBeInTheDocument();
  });

  it("renders sidebar, title-bar readouts and footer shortcuts; navigates via the sidebar", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    });
    // The compact readout sits on the title bar and names the core's engine, never a fixture.
    const header = await screen.findByTestId("title-bar-readout");
    expect(within(header).getByText("Qwen3-ASR-1.7B")).toBeInTheDocument();
    expect(within(header).getByTitle("内置服务 · Qwen/Qwen3-ASR-1.7B · 就绪")).toBeInTheDocument();
    // The 麦克风 readout is the device the native meter enumerated (async), not a fixture.
    expect(await within(header).findByText("Fifine K669")).toBeInTheDocument();
    expect(screen.queryByText(/精确 · SenseVoice/)).toBeNull();
    // regression (2026-09-25): Bridge & MCP was removed — no nav item, no readout, no shortcut.
    expect(screen.queryByRole("button", { name: /Bridge/ })).toBeNull();
    expect(screen.queryByText(/客户端|Bridge|MCP/)).toBeNull();
    // regression (2026-09-25): 引擎 is a shortcut into the settings dialog's engines group; the
    // title bar keeps naming the page beneath.
    await user.click(screen.getByRole("button", { name: /^引擎$/ }));
    let dialog = screen.getByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("tab", { name: /引擎/, selected: true })).toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "引擎", level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "引擎", level: 1 })).toBeNull();
    await user.keyboard("{Escape}");
    // Settings open as a modal dialog over the current page; the title bar keeps naming the page.
    await user.click(screen.getByRole("button", { name: /^设置$/ }));
    dialog = screen.getByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("heading", { name: "外观", level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "关于" }));
    dialog = screen.getByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("heading", { name: "关于", level: 2 })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "本机身份" }));
    expect(screen.getByRole("heading", { name: "手机麦克风", level: 1 })).toBeInTheDocument();
    // 反馈 opens the repository's new-issue page through the shell, not a toast that names it.
    await user.click(screen.getByRole("button", { name: "反馈" }));
    expect(backend.linksOpened).toEqual(["feedback"]);
    expect(screen.queryByText(/反馈渠道/)).toBeNull();
  });

  it("opens the command palette with Ctrl K, previews and applies a theme, and navigates", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    await user.keyboard("{Control>}k{/Control}");
    await user.type(paletteInput(), "暗黑");
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("dark");
    });
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(backend.peek().settings.theme).toBe("dark");
    });
    expect(await screen.findByText("主题已切换：暗黑")).toBeInTheDocument();

    await user.keyboard("{Control>}k{/Control}");
    await user.type(paletteInput(), "跟随系统");
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(backend.peek().settings.follow_system_theme).toBe(true);
    });

    await user.click(screen.getByRole("button", { name: /搜索或输入命令/ }));
    await user.type(paletteInput(), "打开历史");
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();

    await user.keyboard("{Control>}k{/Control}");
    await user.type(paletteInput(), "明亮");
    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("light");
    });
  });

  it("regression: the palette's 新建规则 opens the rule editor, from another page and from the rules page", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    await user.keyboard("{Control>}k{/Control}");
    await user.type(paletteInput(), "新建规则");
    await user.keyboard("{Enter}");
    expect(await screen.findByTestId("rule-editor")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "规则", level: 1 })).toBeInTheDocument();
    // The one-shot flag is gone, so asking again from the rules page opens a fresh editor.
    await user.click(
      within(screen.getByTestId("rule-editor")).getByRole("button", { name: /取消/ }),
    );
    expect(screen.queryByTestId("rule-editor")).toBeNull();
    await user.keyboard("{Control>}k{/Control}");
    await user.type(paletteInput(), "新建规则");
    await user.keyboard("{Enter}");
    expect(await screen.findByTestId("rule-editor")).toBeInTheDocument();
  });

  it("Ctrl , and Ctrl H shortcuts navigate", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    await user.keyboard("{Control>},{/Control}");
    const dialog = screen.getByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("heading", { name: "外观", level: 2 })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>}h{/Control}");
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
  });

  it("regression: the palette's dictation, copy-last and clear-history entries are real core actions", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    try {
      const { backend } = renderApp({ mock: { now: () => Date.now() } });
      await screen.findByRole("heading", { name: "首页", level: 1 });
      const count = backend.peek().history.length;
      // 开始听写 starts a session; the entry turns into 停止听写 while listening.
      await user.keyboard("{Control>}k{/Control}");
      await user.type(paletteInput(), "开始听写");
      let palette = screen.getByRole("dialog", { name: "命令菜单" });
      const dictate = within(palette).getByRole("option", { name: /开始听写/ });
      expect(dictate).not.toHaveAttribute("aria-disabled", "true");
      expect(within(dictate).getByLabelText("Ctrl Alt Space")).toBeInTheDocument();
      await user.keyboard("{Enter}");
      expect(backend.peek().dictation.phase.phase).toBe("listening");
      await user.keyboard("{Control>}k{/Control}");
      await user.type(paletteInput(), "停止听写");
      palette = screen.getByRole("dialog", { name: "命令菜单" });
      expect(within(palette).getByRole("option", { name: /停止听写/ })).toBeInTheDocument();
      await user.keyboard("{Enter}");
      expect(backend.peek().dictation.phase.phase).toBe("processing");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS);
      });
      expect(backend.peek().history).toHaveLength(count + 1);
      // 复制上一条结果 copies the newest history text.
      await user.keyboard("{Control>}k{/Control}");
      await user.type(paletteInput(), "复制上一条");
      await user.keyboard("{Enter}");
      expect(writeText).toHaveBeenCalledWith(backend.peek().history[0]?.text);
      expect(await screen.findByText(/已复制上一条结果 · \d+ 字/)).toBeInTheDocument();
      // 删除全部历史 asks first, then clears the core's list.
      await user.keyboard("{Control>}k{/Control}");
      await user.type(paletteInput(), "删除全部");
      palette = screen.getByRole("dialog", { name: "命令菜单" });
      expect(within(palette).getByRole("option", { name: /删除全部历史/ })).toHaveTextContent(
        `${count + 1} 条`,
      );
      expect(within(palette).queryByText(/第二阶段|尚未接入/)).toBeNull();
      await user.keyboard("{Enter}");
      const confirm = screen.getByRole("dialog", { name: /删除全部 \d+ 条历史记录/ });
      expect(confirm).toHaveTextContent("history.json 会被清空");
      await user.click(within(confirm).getByRole("button", { name: "删除全部" }));
      await waitFor(() => {
        expect(backend.peek().history).toEqual([]);
      });
      // With nothing left both history entries are disabled with a plain reason.
      await user.keyboard("{Control>}k{/Control}");
      await user.type(paletteInput(), "复制上一条");
      palette = screen.getByRole("dialog", { name: "命令菜单" });
      expect(within(palette).getByRole("option", { name: /复制上一条/ })).toHaveAttribute(
        "aria-disabled",
        "true",
      );
      expect(within(palette).getByText("历史记录为空")).toBeInTheDocument();
      await user.keyboard("{Escape}");
      expect(screen.queryByRole("dialog", { name: "命令菜单" })).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('regression: an {type:"error"} event shows a danger toast; trusted, message and identity events toast too', async () => {
    const { backend } = renderApp({ path: "/devices" });
    await screen.findByRole("heading", { name: "手机麦克风", level: 1 });
    act(() => {
      backend.simulateError("relay refused the ticket");
    });
    const toast = await screen.findByRole("alert");
    expect(toast).toHaveTextContent("出错了 · relay refused the ticket");
    act(() => {
      backend.simulateMessage(
        "9f0c2b1e6a4d3c5f7e8a9b0c1d2e3f405162738495a6b7c8d9e0f1a2b3c4d5e6",
        "hello",
      );
      backend.simulateMessage(
        "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809",
        "hi from pixel",
      );
    });
    expect(await screen.findByText("未知设备：hello")).toBeInTheDocument();
    expect(await screen.findByText("Pixel 8：hi from pixel")).toBeInTheDocument();
  });

  it("regression: the title bar shows the compact engine and microphone readout inline and no second header row", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    // Windows test 2026-09-24: the app drew its own toolbar under the native title bar. The strip
    // now *is* the title bar: one `deep` drag region continued by the sidebar brand row. User
    // feedback 2026-09-25 (round two): the facts come back, but simplified and on the bar itself —
    // a compact engine · microphone readout right after the title — never as a second header row.
    const bar = screen.getByTestId("title-bar");
    expect(bar).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(bar).toHaveClass("h-10");
    expect(within(bar).getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    expect(
      within(bar)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual([TITLE_BAR_SEARCH_LABEL, "润色 · 开/关"]);
    expect(within(bar).queryByText("Ctrl K")).toBeNull();
    expect(within(bar).queryByText(/SenseVoice|示例/)).toBeNull();
    expect(screen.queryByTestId("sample-data-notice")).toBeNull();
    const readout = within(bar).getByTestId("title-bar-readout");
    expect(readout).toHaveClass("mono", "text-[11px]", "text-fg-subtle", "hidden", "md:flex");
    expect(within(readout).getByText("Qwen3-ASR-1.7B")).toBeInTheDocument();
    expect(await within(readout).findByText("Fifine K669")).toBeInTheDocument();
    expect(readout).toHaveTextContent("Qwen3-ASR-1.7B·Fifine K669");
    expect(within(readout).queryByText("引擎")).toBeNull();
    expect(within(readout).queryByText("麦克风")).toBeNull();
    expect(bar.querySelectorAll("[data-tone]")).toHaveLength(2); // engine lamp + polish lamp
    // No second header row: the content follows the bar directly.
    expect(screen.queryByTestId("page-header")).toBeNull();
    expect(bar.nextElementSibling?.tagName).toBe("MAIN");
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(brand).toHaveClass("h-10");
    // jsdom is a plain browser (no Tauri window handle) whose UA says linux: no window controls,
    // no macOS traffic-light inset, and the identity hint ("windows") does not override the UA.
    expect(screen.queryByTestId("window-controls")).toBeNull();
    expect(bar).toHaveAttribute("data-platform", "linux");
    expect(brand).not.toHaveClass("pl-15");
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);

    // The 润色 toggle is icon + short label + lamp and really toggles the LLM pass through the core.
    const toggle = within(bar).getByTestId("polish-toggle");
    expect(toggle.querySelector("svg[data-icon='wand']")).toBeInTheDocument();
    expect(toggle.textContent).toBe("AI润色");
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(toggle).toHaveAttribute("title", "润色 · 开 · 点击关闭 LLM 润色");
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(toggle).toHaveAttribute("title", "润色 · 关 · 点击开启 LLM 润色");
    expect(toggle.querySelector("[data-tone='idle']")).toBeInTheDocument();
    expect(backend.peek().engines.refine_enabled).toBe(false);
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().engines.refine_enabled).toBe(true);
    });
    expect(toggle.querySelector("[data-tone='ok']")).toBeInTheDocument();
  });

  it("regression: no page shows the phase-2 chip or the sample-data notice for dictation, history, engines or hotkey", async () => {
    for (const path of ["/", "/history", "/settings/hotkey", "/onboarding?step=4"]) {
      const { unmount } = renderApp({ path });
      await screen.findByTestId("title-bar");
      await screen.findByTestId("title-bar-readout");
      expect(screen.queryByTestId("sample-data-notice")).toBeNull();
      expect(screen.queryByTestId("deferred-badge")).toBeNull();
      expect(screen.queryByTestId("sample-footnote")).toBeNull();
      expect(document.body.textContent).not.toMatch(/第二阶段|示例数据|示例内容/);
      unmount();
    }
    // The engines group of the settings dialog is real end to end: no footnote, no "planned".
    {
      const { unmount } = renderApp({ path: "/settings/engine" });
      await screen.findByTestId("title-bar-readout");
      await screen.findByTestId("providers-asr");
      expect(screen.queryByTestId("sample-data-notice")).toBeNull();
      expect(screen.queryByTestId("deferred-badge")).toBeNull();
      expect(screen.queryByTestId("sample-footnote")).toBeNull();
      expect(document.body.textContent).not.toMatch(/第二阶段|示例数据|示例内容|计划中/);
      unmount();
    }
    // The dictionary and rules pages are the core's lists (docs/dictation.md §16): no footnote,
    // no badge, nothing "not wired yet".
    for (const path of ["/dictionary", "/rules"]) {
      const { unmount } = renderApp({ path });
      await screen.findByTestId("title-bar");
      expect(screen.queryByTestId("sample-data-notice")).toBeNull();
      expect(screen.queryByTestId("deferred-badge")).toBeNull();
      expect(screen.queryByTestId("sample-footnote")).toBeNull();
      expect(document.body.textContent).not.toMatch(/第二阶段|示例数据|示例内容|尚未接入/);
      unmount();
    }
  });

  it("regression: the polish toggle carries the AI润色 text label", async () => {
    const { unmount } = renderApp();
    await screen.findByRole("heading", { name: "首页", level: 1 });
    const toggle = screen.getByTestId("polish-toggle");
    const label = within(toggle).getByTestId("polish-toggle-label");
    expect(label).toHaveTextContent("AI润色");
    expect(label).toHaveClass("text-[12px]");
    // Icon, then label, then lamp: the words sit between the wand and the status dot.
    const children = [...toggle.children];
    expect(children[0]?.tagName.toLowerCase()).toBe("svg");
    expect(children[1]).toBe(label);
    expect(children[2]).toHaveAttribute("data-tone", "ok");
    expect(toggle).toHaveAttribute("aria-label", "润色 · 开/关");
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    unmount();
    // English: `AI Polish`.
    renderApp({ mock: { settings: { locale: "en" } } });
    await screen.findByRole("heading", { name: "Home", level: 1 });
    expect(screen.getByTestId("polish-toggle-label")).toHaveTextContent("AI Polish");
    expect(screen.getByTestId("polish-toggle")).toHaveAttribute("aria-label", "Polish · on/off");
  });

  it("regression: the title-bar readout stays on every page and the footer hotkey follows settings", async () => {
    const { backend } = renderApp({ path: "/nowhere" });
    await screen.findByText("没有这个页面");
    expect(screen.getByTestId("title-bar-readout")).toBeInTheDocument();
    expect(screen.queryByTestId("page-header")).toBeNull();
    renderApp({ backend: new MockBackend({ settings: { hotkey: "Ctrl+Shift+D" } }) });
    // The footer re-keys its items when the core's settings arrive, so wait for the loaded chord.
    await waitFor(() => {
      const footer = [...document.querySelectorAll("footer")].at(-1);
      if (!footer) throw new Error("no footer");
      expect(within(footer).getByLabelText("Ctrl Shift D")).toBeInTheDocument();
      expect(within(footer).getByText("按住听写")).toBeInTheDocument();
      expect(within(footer).queryByLabelText("Ctrl Alt Space")).toBeNull();
    });
    expect(backend.peek().settings.hotkey).toBe("Ctrl+Alt+Space");
  });

  it("renders the not-found page and gets back home", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/nowhere" });
    expect(await screen.findByText("没有这个页面")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "回到首页" }));
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
  });

  it("regression: the sidebar counts come from the core dictionary and rules and a zero count is hidden", async () => {
    const entry = {
      id: "00000000-0000-4000-8000-000000000001",
      term: "Voltip",
      heard_as: ["沃提普"],
      enabled: true,
      source: { kind: "manual" as const },
      created_at_ms: 1,
      updated_at_ms: 1,
    };
    const backend = new MockBackend({ history: [], dictionary: [entry] });
    renderApp({ backend });
    await screen.findByRole("heading", { name: "首页", level: 1 });
    const item = (name: string) => screen.getByRole("button", { name: new RegExp(`^${name}`) });
    expect(item("词典")).toHaveTextContent(/^词典1$/);
    expect(item("规则")).toHaveTextContent(/^规则$/);
    expect(item("历史记录")).toHaveTextContent(/^历史记录$/);
    act(() => {
      backend.publish({
        type: "rules",
        rules: [
          {
            id: "00000000-0000-4000-9000-000000000001",
            name: "r",
            kind: "literal",
            pattern: "a",
            replacement: "b",
            case_sensitive: true,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
          },
        ],
      });
      backend.publish({ type: "dictionary", entries: [] });
    });
    expect(item("规则")).toHaveTextContent(/^规则1$/);
    expect(item("词典")).toHaveTextContent(/^词典$/);
  });
});
