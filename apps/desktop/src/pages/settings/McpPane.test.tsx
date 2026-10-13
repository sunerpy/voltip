import { type McpSettings, defaultMcpSettings } from "@voltip/shared";
import { MOCK_TAKEN_PORTS, MockBackend } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { MCP_ACTIVITY_SHOWN } from "./McpPane";

/** A core whose MCP service has `mcp` (on by default) and a clock that runs, so 「刚刚」 reads. */
function core(mcp: Partial<McpSettings> = {}) {
  return new MockBackend({ settings: { mcp: { ...defaultMcpSettings(), enabled: true, ...mcp } } });
}

async function pane() {
  return screen.findByTestId("mcp-pane");
}

describe("Settings · AI 工具接入", () => {
  it("sits right after 本机服务 and opens its guide in the interface's language", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp({ path: "/settings/mcp", backend });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    const tabs = within(dialog)
      .getAllByRole("tab")
      .map((tab) => tab.textContent ?? "");
    expect(tabs.indexOf("AI 工具接入")).toBe(tabs.indexOf("本机服务") + 1);
    expect(within(dialog).getByRole("tab", { name: "AI 工具接入", selected: true })).toBeVisible();
    await user.click(within(await pane()).getByRole("button", { name: "使用指南" }));
    expect(backend.guidesOpened).toEqual([{ page: "mcp", locale: "zh-CN" }]);
  });

  it("switching the service on shows it running at its address and who connected; off stops it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    expect(within(view).getByTestId("mcp-status")).toHaveTextContent("未启用");
    expect(within(view).queryByTestId("mcp-address")).toBeNull();
    await user.click(within(view).getByRole("switch", { name: "启用 MCP 服务" }));
    await waitFor(() => {
      expect(within(view).getByTestId("mcp-status")).toHaveTextContent("运行中");
    });
    expect(within(view).getByTestId("mcp-address")).toHaveTextContent("http://127.0.0.1:47841/mcp");
    expect(within(view).getByTestId("mcp-clients")).toHaveTextContent("尚无 AI 工具连接");
    expect(backend.peek().settings.mcp).toEqual({ ...defaultMcpSettings(), enabled: true });
    act(() => {
      backend.simulateMcpRead("Claude Code", "list_rules");
    });
    expect(within(view).getByTestId("mcp-clients")).toHaveTextContent(
      "已连接：Claude Code（刚刚）",
    );
    await user.click(within(view).getByTestId("mcp-copy-address"));
    expect(await screen.findByText("地址已复制")).toBeInTheDocument();
    expect(await navigator.clipboard.readText()).toBe("http://127.0.0.1:47841/mcp");
    await user.click(within(view).getByRole("switch", { name: "启用 MCP 服务" }));
    await waitFor(() => {
      expect(within(view).getByTestId("mcp-status")).toHaveTextContent("未启用");
    });
    expect(within(view).queryByTestId("mcp-clients")).toBeNull();
  });

  it("a port is applied when valid, refused when not or when the local service has it, and a taken one is reported", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    const port = within(view).getByTestId("mcp-port");
    const apply = within(view).getByTestId("mcp-port-apply");
    expect(apply).toBeDisabled();
    await user.clear(port);
    await user.type(port, "80");
    await user.click(apply);
    expect(await within(view).findByText("端口须在 1024–65535 之间")).toBeInTheDocument();
    await user.clear(port);
    await user.type(port, `${backend.peek().settings.serve.port}{Enter}`);
    expect(await within(view).findByText("该端口已用于本机服务，请换一个端口")).toBeInTheDocument();
    expect(backend.peek().settings.mcp.port).toBe(47841);
    await user.clear(port);
    await user.type(port, "48100{Enter}");
    await waitFor(() => {
      expect(within(view).getByTestId("mcp-address")).toHaveTextContent(
        "http://127.0.0.1:48100/mcp",
      );
    });
    // The commands follow the address.
    expect(within(view).getByTestId("mcp-setup-command-claude_code")).toHaveTextContent(
      "http://127.0.0.1:48100/mcp",
    );
    const taken = MOCK_TAKEN_PORTS[0] ?? 47999;
    await user.clear(port);
    await user.type(port, `${taken}{Enter}`);
    await waitFor(() => {
      expect(within(view).getByTestId("mcp-status")).toHaveTextContent(
        `无法启动：无法监听 127.0.0.1:${taken}`,
      );
    });
    expect(within(view).queryByTestId("mcp-address")).toBeNull();
  });

  it("the correction summary is read by default; narrowing applies at once, the full history only after a confirmation with its period", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    const read = within(view).getByRole("radiogroup", { name: "读取范围" });
    expect(within(read).getByRole("radio", { name: "替换汇总" })).toBeChecked();
    expect(within(read).getAllByText("推荐")).toHaveLength(1);
    await user.click(within(read).getByRole("radio", { name: "仅词典和规则" }));
    await waitFor(() => {
      expect(backend.peek().settings.mcp.read).toBe("rules");
    });
    expect(screen.queryByRole("dialog", { name: /完整的历史记录/ })).toBeNull();

    // 完整历史: the choice waits for the confirmation; Esc closes only the confirmation.
    await user.click(within(read).getByRole("radio", { name: "完整历史" }));
    let confirm = screen.getByRole("dialog", { name: "允许 AI 工具读取完整的历史记录？" });
    expect(confirm).toHaveTextContent("可能保存在它的对话记录里");
    expect(confirm).toHaveTextContent("可能包含个人信息、账号或其他敏感内容");
    expect(within(read).getByRole("radio", { name: "完整历史" })).not.toBeChecked();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "允许 AI 工具读取完整的历史记录？" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    expect(backend.peek().settings.mcp.read).toBe("rules");

    await user.click(within(read).getByRole("radio", { name: "完整历史" }));
    confirm = screen.getByRole("dialog", { name: "允许 AI 工具读取完整的历史记录？" });
    const period = within(confirm).getByRole("combobox", { name: "时间范围" });
    expect(period).toHaveValue("last_30_days");
    await user.selectOptions(period, "last_7_days");
    await user.click(within(confirm).getByRole("button", { name: "允许读取" }));
    await waitFor(() => {
      expect(backend.peek().settings.mcp).toMatchObject({
        read: "full",
        history_range: "last_7_days",
      });
    });
    expect(within(read).getByRole("radio", { name: "完整历史" })).toBeChecked();
    expect(within(view).getByTestId("mcp-full-warning")).toHaveTextContent(
      "已允许读取完整历史记录（最近 7 天）。",
    );
    await user.selectOptions(within(view).getByTestId("mcp-range"), "all");
    await waitFor(() => {
      expect(within(view).getByTestId("mcp-full-warning")).toHaveTextContent(
        "已允许读取完整历史记录（全部）。",
      );
    });
    expect(backend.peek().settings.mcp.history_range).toBe("all");

    // 收回 goes back to the summary at once.
    await user.click(within(view).getByRole("button", { name: "收回" }));
    await waitFor(() => {
      expect(backend.peek().settings.mcp.read).toBe("summary");
    });
    expect(await screen.findByText("已收回完整历史的读取授权")).toBeInTheDocument();
    expect(within(view).queryByTestId("mcp-full-warning")).toBeNull();
  });

  it("turning rules on directly needs a confirmation; the other levels apply at once", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    const write = within(view).getByRole("radiogroup", { name: "修改权限" });
    expect(within(write).getByRole("radio", { name: "修改词典，新建规则需确认" })).toBeChecked();
    await user.click(within(write).getByRole("radio", { name: "只读" }));
    await waitFor(() => {
      expect(backend.peek().settings.mcp.write).toBe("read_only");
    });
    await user.click(within(write).getByRole("radio", { name: "也可以直接启用规则" }));
    let confirm = screen.getByRole("dialog", { name: "允许外部工具直接启用规则？" });
    expect(confirm).toHaveTextContent("可能改动插入到其他应用里的文字");
    await user.click(within(confirm).getByRole("button", { name: "取消" }));
    expect(backend.peek().settings.mcp.write).toBe("read_only");
    expect(within(write).getByRole("radio", { name: "只读" })).toBeChecked();
    await user.click(within(write).getByRole("radio", { name: "也可以直接启用规则" }));
    confirm = screen.getByRole("dialog", { name: "允许外部工具直接启用规则？" });
    await user.click(within(confirm).getByRole("button", { name: "允许" }));
    await waitFor(() => {
      expect(backend.peek().settings.mcp.write).toBe("enable_rules");
    });
    // The rest of the setting is left as it was.
    expect(backend.peek().settings.mcp).toEqual({
      ...defaultMcpSettings(),
      enabled: true,
      write: "enable_rules",
    });
  });

  it("the commands show a placeholder; 复制命令 and 复制令牌 have the core copy the real token, which never reaches the page", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    expect(within(view).getByTestId("mcp-setup-command-claude_code")).toHaveTextContent(
      'claude mcp add --transport http voltip http://127.0.0.1:47841/mcp --header "Authorization: Bearer <令牌>"',
    );
    const codex = within(view).getByTestId("mcp-setup-command-codex").textContent ?? "";
    expect(codex.split("\n")).toEqual([
      "export VOLTIP_MCP_TOKEN=<令牌>",
      "codex mcp add voltip --url http://127.0.0.1:47841/mcp --bearer-token-env-var VOLTIP_MCP_TOKEN",
    ]);
    await user.click(within(view).getByRole("button", { name: "复制命令 · Codex" }));
    await user.click(within(view).getByRole("button", { name: "复制命令 · Claude Code" }));
    await waitFor(() => {
      expect(backend.mcpSetupCopies).toEqual(["codex", "claude_code"]);
    });
    expect((await screen.findAllByText("命令已复制，其中已填入令牌")).length).toBeGreaterThan(0);

    await user.click(within(view).getByTestId("mcp-copy-token"));
    await waitFor(() => {
      expect(backend.mcpTokenCopies).toBe(1);
    });
    expect(await screen.findByText("令牌已复制到剪贴板")).toBeInTheDocument();
    await user.click(within(view).getByTestId("mcp-rotate-token"));
    const confirm = screen.getByRole("dialog", { name: "重新生成令牌？" });
    expect(confirm).toHaveTextContent("旧令牌立即失效");
    expect(backend.mcpTokenRotations).toBe(0);
    await user.click(within(confirm).getByRole("button", { name: "重新生成" }));
    await waitFor(() => {
      expect(backend.mcpTokenRotations).toBe(1);
    });
    expect(document.body.textContent).not.toMatch(/[0-9a-f]{64}|[A-Za-z0-9_-]{43}/);
  });

  it("on Windows the Codex command sets the variable the PowerShell way, as the core copies it", async () => {
    const agent = vi
      .spyOn(navigator, "userAgent", "get")
      .mockReturnValue("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
    try {
      renderApp({ path: "/settings/mcp", backend: core() });
      const view = await pane();
      expect(within(view).getByTestId("mcp-setup-command-codex").textContent).toMatch(
        /^\$env:VOLTIP_MCP_TOKEN = "<令牌>"\ncodex mcp add voltip /,
      );
    } finally {
      agent.mockRestore();
    }
  });

  it("the recent activity lists reads and writes; a write can be undone, and 查看全部 shows the rest", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    expect(within(view).getByTestId("mcp-activity-empty")).toHaveTextContent("尚无活动");
    act(() => {
      backend.simulateMcpRead("Codex", "list_dictionary");
      backend.simulateMcpRead("Claude Code", "read_summary", 42);
      backend.simulateMcpEntry("Claude Code", {
        term: "Voltip",
        heard_as: ["沃提普"],
        enabled: true,
      });
      backend.simulateMcpRule("Claude Code", {
        name: "非书 → 飞书",
        kind: "literal",
        pattern: "非书",
        replacement: "飞书",
        case_sensitive: true,
        enabled: true,
      });
    });
    const list = within(view).getByRole("list", { name: "最近活动" });
    expect(
      within(list)
        .getAllByTestId("mcp-activity-row")
        .map((row) => row.textContent),
    ).toEqual([
      "Claude Code 新建了规则「非书 → 飞书」，待确认刚刚撤销",
      "Claude Code 添加了词条「Voltip」刚刚撤销",
      "Claude Code 读取了替换汇总（42 对）刚刚",
      "Codex 读取了词典刚刚",
    ]);
    expect(within(list).getAllByRole("img", { name: "修改" })).toHaveLength(2);
    expect(within(list).getAllByRole("img", { name: "读取" })).toHaveLength(2);
    expect(within(view).queryByTestId("mcp-activity-toggle")).toBeNull();

    await user.click(
      within(list).getByRole("button", { name: "撤销：Claude Code 添加了词条「Voltip」" }),
    );
    await waitFor(() => {
      expect(backend.peek().dictionary).toEqual([]);
    });
    expect(
      await screen.findByText("已撤销 · Claude Code 添加了词条「Voltip」"),
    ).toBeInTheDocument();
    const undone = within(list).getAllByTestId("mcp-activity-row")[1];
    expect(undone).toHaveTextContent("已撤销");
    expect(undone && within(undone).queryByRole("button")).toBeNull();
    // The rule waits for approval and can still be undone.
    expect(backend.peek().rules.map((r) => [r.name, r.pending])).toEqual([["非书 → 飞书", true]]);

    act(() => {
      for (let i = 0; i < 3; i += 1) backend.simulateMcpRead("Codex", "list_rules");
    });
    expect(within(list).getAllByTestId("mcp-activity-row")).toHaveLength(MCP_ACTIVITY_SHOWN);
    await user.click(within(view).getByRole("button", { name: "查看全部" }));
    expect(within(list).getAllByTestId("mcp-activity-row")).toHaveLength(7);
    await user.click(within(view).getByRole("button", { name: "收起" }));
    expect(within(list).getAllByTestId("mcp-activity-row")).toHaveLength(MCP_ACTIVITY_SHOWN);
  });

  it("an undo the core refuses says why and leaves the line as it was", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/settings/mcp", backend });
    const view = await pane();
    let id = "";
    act(() => {
      id = backend.simulateMcpEntry("Claude Code", {
        term: "Voltip",
        heard_as: ["沃提普"],
        enabled: true,
      });
    });
    // The user changes the entry after the tool wrote it.
    await act(async () => {
      await backend.invoke("dictionary_update", {
        id,
        entry: { term: "Voltip", heard_as: ["沃提普", "我替谱"], enabled: true },
      });
    });
    await user.click(
      within(view).getByRole("button", { name: "撤销：Claude Code 添加了词条「Voltip」" }),
    );
    expect(
      await screen.findByText("出错了 · 无法撤销：这个词条之后又被修改过"),
    ).toBeInTheDocument();
    expect(within(view).getByRole("button", { name: /^撤销：/ })).toBeInTheDocument();
    expect(screen.queryByText(/^已撤销 · /)).toBeNull();
    expect(backend.peek().dictionary.map((e) => e.heard_as)).toEqual([["沃提普", "我替谱"]]);
  });

  it("a listener being started says so and shows no address yet", async () => {
    renderApp({
      path: "/settings/mcp",
      backend: new MockBackend({
        settings: { mcp: { ...defaultMcpSettings(), enabled: true } },
        mcp: { available: true, phase: "starting", clients: [], activity: [] },
      }),
    });
    const view = await pane();
    expect(within(view).getByTestId("mcp-status")).toHaveTextContent("正在启动");
    expect(within(view).queryByTestId("mcp-address")).toBeNull();
    // The commands already name the address the saved port gives.
    expect(within(view).getByTestId("mcp-setup-command-claude_code")).toHaveTextContent(
      "http://127.0.0.1:47841/mcp",
    );
  });

  it("the English pane reads the same", async () => {
    renderApp({ path: "/settings/mcp", systemLanguage: "en-US", backend: core() });
    const view = await pane();
    expect(view).toHaveTextContent("AI tools");
    expect(within(view).getByRole("switch", { name: "Turn on the MCP server" })).toBeChecked();
    expect(within(view).getByTestId("mcp-status")).toHaveTextContent("Running");
    expect(within(view).getByRole("radio", { name: "Correction summary" })).toBeChecked();
    expect(within(view).getByTestId("mcp-setup-command-claude_code")).toHaveTextContent(
      "Authorization: Bearer <token>",
    );
  });
});
