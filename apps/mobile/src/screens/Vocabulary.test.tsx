import { MockBackend } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// The phone's dictionary, rules and scenes (user decision 2026-10-01: the phone works on its own
// with every feature but the local models; a scene is picked by hand on the talk card, since the
// phone cannot tell which app the text goes to).
describe("the phone's dictionary", () => {
  it("adds, switches off, edits, reorders and deletes entries, with the desktop's checks", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "dictionary" });
    const page = await screen.findByTestId("phone-dictionary");
    expect(screen.getByRole("heading", { name: "个人词典", level: 1 })).toBeInTheDocument();
    expect(within(page).getByText("暂无词条。")).toBeInTheDocument();

    await user.click(within(page).getByRole("button", { name: "新建词条" }));
    let dialog = screen.getByRole("dialog", { name: "新建词条" });
    await user.type(within(dialog).getByLabelText("正确写法"), "Voltip");
    await user.type(within(dialog).getByLabelText("曾听成"), "伏特, 沃尔提");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => [e.term, e.heard_as, e.enabled])).toEqual([
        ["Voltip", ["伏特", "沃尔提"], true],
      ]);
    });
    expect(screen.queryByRole("dialog", { name: "新建词条" })).toBeNull();
    expect(within(page).getByText("伏特 · 沃尔提")).toBeInTheDocument();

    // The same spelling again: the desktop's instant check, and no save.
    await user.click(within(page).getByRole("button", { name: "新建词条" }));
    dialog = screen.getByRole("dialog", { name: "新建词条" });
    await user.type(within(dialog).getByLabelText("正确写法"), "voltip");
    expect(within(dialog).getByText("词典里已有这个写法")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeDisabled();
    await user.type(within(dialog).getByLabelText("正确写法"), "s");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip", "voltips"]);
    });

    await user.click(within(page).getByRole("switch", { name: "启用 Voltip" }));
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.enabled).toBe(false);
    });
    expect(screen.getByRole("switch", { name: "启用 Voltip" })).toHaveAttribute(
      "aria-checked",
      "false",
    );

    // Editing keeps the switch as it is; the second entry moves up.
    await user.click(within(page).getByRole("button", { name: "编辑 voltips" }));
    dialog = screen.getByRole("dialog", { name: "编辑词条" });
    expect(within(dialog).getByText("匹配顺序第 2 条，共 2 条")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "上移 voltips" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["voltips", "Voltip"]);
    });
    const term = within(dialog).getByLabelText("正确写法");
    await user.clear(term);
    await user.type(term, "Voltips");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => [e.term, e.enabled])).toEqual([
        ["Voltips", true],
        ["Voltip", false],
      ]);
    });

    // Search, then delete after the confirmation.
    await user.type(within(page).getByRole("textbox", { name: "搜索词条" }), "沃尔");
    expect(within(page).queryByRole("button", { name: "编辑 Voltips" })).toBeNull();
    await user.click(within(page).getByRole("button", { name: "编辑 Voltip" }));
    dialog = screen.getByRole("dialog", { name: "编辑词条" });
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    const confirm = screen.getByRole("dialog", { name: "删除“Voltip”？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltips"]);
    });
    expect(within(page).getByText("没有匹配的词条")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-dictionary")).toHaveTextContent("1 条启用");
    backend.destroy();
  });

  it("moves an entry down, and a refused switch or move is a message", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    for (const term of ["甲方", "乙方"])
      await act(() =>
        backend.invoke("dictionary_add", { entry: { term, heard_as: [], enabled: true } }),
      );
    renderApp({ backend, initialScreen: "dictionary" });
    const page = await screen.findByTestId("phone-dictionary");
    await user.click(within(page).getByRole("button", { name: "编辑 甲方" }));
    const dialog = screen.getByRole("dialog", { name: "编辑词条" });
    expect(within(dialog).getByRole("button", { name: "上移 甲方" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "下移 甲方" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["乙方", "甲方"]);
    });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("dictionary: 顺序不对"));
    await user.click(within(dialog).getByRole("button", { name: "上移 甲方" }));
    expect(await screen.findByText("出错了 · 顺序不对")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("dictionary: 改不了"));
    await user.click(within(page).getByRole("switch", { name: "启用 乙方" }));
    expect(await screen.findByText("出错了 · 改不了")).toBeInTheDocument();
    expect(backend.peek().dictionary.every((e) => e.enabled)).toBe(true);
    backend.destroy();
  });

  it("a refusal of the core stays in the editor", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "dictionary" });
    const page = await screen.findByTestId("phone-dictionary");
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("dictionary: 词条太多"));
    await user.click(within(page).getByRole("button", { name: "新建词条" }));
    const dialog = screen.getByRole("dialog", { name: "新建词条" });
    await user.type(within(dialog).getByLabelText("正确写法"), "Voltip");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("词条太多");
    backend.destroy();
  });
});

describe("the phone's rules", () => {
  it("adds a rule the core checked, tries it, switches it off and deletes it", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "rules" });
    const page = await screen.findByTestId("phone-rules");
    expect(screen.getByRole("heading", { name: "替换规则", level: 1 })).toBeInTheDocument();
    expect(within(page).getByText("暂无规则")).toBeInTheDocument();

    await user.click(within(page).getByRole("button", { name: "新建规则" }));
    const dialog = screen.getByRole("dialog", { name: "新建规则" });
    await user.type(within(dialog).getByLabelText("名称"), "去掉嗯");
    await user.type(within(dialog).getByLabelText("匹配"), "嗯");
    expect(await within(dialog).findByText("✓ 规则有效")).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("试一试"), "嗯你好");
    // The core is asked again once the typing pauses: the draft rule takes the 嗯 out.
    await waitFor(() => {
      expect(within(dialog).getByTestId("rule-test-result")).toHaveTextContent("你好");
    });
    expect(within(dialog).getByTestId("rule-test-result")).not.toHaveTextContent("嗯");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => [r.name, r.pattern, r.replacement])).toEqual([
        ["去掉嗯", "嗯", ""],
      ]);
    });
    expect(within(page).getByText("嗯 → （删除）")).toBeInTheDocument();

    await user.click(within(page).getByRole("switch", { name: "启用 去掉嗯" }));
    await waitFor(() => {
      expect(backend.peek().rules[0]?.enabled).toBe(false);
    });

    await user.click(within(page).getByRole("button", { name: "编辑 去掉嗯" }));
    const edit = screen.getByRole("dialog", { name: "编辑规则" });
    await user.click(within(edit).getByRole("button", { name: "删除" }));
    const confirm = screen.getByRole("dialog", { name: "删除规则 去掉嗯？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().rules).toEqual([]);
    });
    backend.destroy();
  });

  it("a regex the pipeline cannot compile cannot be saved", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "rules" });
    const page = await screen.findByTestId("phone-rules");
    await user.click(within(page).getByRole("button", { name: "新建规则" }));
    const dialog = screen.getByRole("dialog", { name: "新建规则" });
    await user.type(within(dialog).getByLabelText("名称"), "括号");
    await user.click(within(dialog).getByRole("radio", { name: "正则" }));
    await user.type(within(dialog).getByLabelText("匹配"), "(");
    await waitFor(() => {
      expect(within(dialog).getByTestId("rule-status")).toHaveClass("text-danger");
    });
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeDisabled();
    backend.destroy();
  });

  it("imports a pasted TOML and exports the rules to the clipboard or the share sheet", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "rules" });
    const page = await screen.findByTestId("phone-rules");
    await user.click(within(page).getByRole("button", { name: "导入 TOML" }));
    const dialog = screen.getByRole("dialog", { name: "导入 TOML" });
    const toml =
      'version = 1\n\n[[rule]]\nname = "句号"\nkind = "literal"\npattern = "。。"\nreplacement = "。"\n';
    await user.click(within(dialog).getByLabelText("TOML 文本"));
    await user.paste(toml);
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["句号"]);
    });
    expect(await screen.findByText("已导入 · 合并")).toBeInTheDocument();

    await user.click(within(page).getByRole("button", { name: "导出 TOML" }));
    const exported = await screen.findByRole("dialog", { name: "导出 TOML" });
    const text = within(exported).getByTestId("rules-export-text");
    expect(text).toHaveValue(await backend.rulesExport());
    await user.click(within(exported).getByRole("button", { name: "复制" }));
    await waitFor(() => {
      expect(backend.phoneClipboard).toBe((text as HTMLTextAreaElement).value);
    });
    expect(await screen.findByText("已复制 TOML")).toBeInTheDocument();
    await user.click(within(exported).getByRole("button", { name: "分享" }));
    await waitFor(() => {
      expect(backend.shared).toEqual([(text as HTMLTextAreaElement).value]);
    });
    await user.click(within(exported).getByRole("button", { name: "关闭" }));
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-rules")).toHaveTextContent("1 条启用");
    backend.destroy();
  });

  it("moves a rule, and a refused export, copy, share, switch or move is a message", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    for (const name of ["一", "二"])
      await act(() =>
        backend.invoke("rules_add", {
          rule: {
            name,
            kind: "literal",
            pattern: name,
            replacement: "",
            case_sensitive: true,
            enabled: true,
          },
        }),
      );
    renderApp({ backend, initialScreen: "rules" });
    const page = await screen.findByTestId("phone-rules");
    await user.click(within(page).getByRole("button", { name: "编辑 二" }));
    const dialog = screen.getByRole("dialog", { name: "编辑规则" });
    expect(within(dialog).getByRole("button", { name: "下移 二" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "上移 二" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["二", "一"]);
    });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("rules: 顺序不对"));
    await user.click(within(dialog).getByRole("button", { name: "下移 二" }));
    expect(await screen.findByText("出错了 · 顺序不对")).toBeInTheDocument();
    // A refused save stays in the editor.
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("rules: 存不了"));
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    expect(await within(dialog).findByText("存不了")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "取消" }));

    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("rules: 改不了"));
    await user.click(within(page).getByRole("switch", { name: "启用 一" }));
    expect(await screen.findByText("出错了 · 改不了")).toBeInTheDocument();

    vi.spyOn(backend, "rulesExport").mockRejectedValueOnce(new Error("rules: 导不出"));
    await user.click(within(page).getByRole("button", { name: "导出 TOML" }));
    expect(await screen.findByText("导出失败：导不出")).toBeInTheDocument();
    await user.click(within(page).getByRole("button", { name: "导出 TOML" }));
    const exported = await screen.findByRole("dialog", { name: "导出 TOML" });
    vi.spyOn(backend, "pasteText").mockResolvedValueOnce({ kind: "failed", reason: "inject" });
    await user.click(within(exported).getByRole("button", { name: "复制" }));
    expect(await screen.findByText("复制失败")).toBeInTheDocument();
    vi.spyOn(backend, "pasteText").mockRejectedValueOnce(new Error("no clipboard"));
    await user.click(within(exported).getByRole("button", { name: "复制" }));
    await waitFor(() => {
      expect(screen.getAllByText("复制失败").length).toBe(2);
    });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("share: 没有可以分享的应用"));
    await user.click(within(exported).getByRole("button", { name: "分享" }));
    expect(await screen.findByText("出错了 · 没有可以分享的应用")).toBeInTheDocument();
    backend.destroy();
  });
});

describe("the phone's scenes", () => {
  it("lists the built-in scenes without the desktop's matching and creates one with no app", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ initialScreen: "scenes" });
    const page = await screen.findByTestId("phone-scenes");
    expect(screen.getByRole("heading", { name: "场景", level: 1 })).toBeInTheDocument();
    const cards = within(page).getAllByTestId("scene-card");
    expect(cards.length).toBe(backend.peek().scenes.length);
    expect(cards.length).toBeGreaterThan(0);
    // No matching on the phone: no order, switches, moves or applications.
    expect(within(page).queryByRole("switch")).toBeNull();
    expect(within(page).queryByRole("button", { name: /^上移/ })).toBeNull();
    expect(within(page).queryByTestId("scene-match")).toBeNull();
    expect(within(page).getAllByText("内置").length).toBe(cards.length);

    await user.click(within(page).getByRole("button", { name: "新建场景" }));
    const editor = await screen.findByTestId("scene-editor");
    expect(within(editor).queryByTestId("scene-editor-apps")).toBeNull();
    expect(within(editor).queryByTestId("scene-editor-keywords")).toBeNull();
    expect(within(editor).queryByRole("combobox", { name: "输出方式" })).toBeNull();
    await user.type(within(editor).getByLabelText("名称"), "会议纪要");
    await user.selectOptions(within(editor).getByRole("combobox", { name: "AI 润色" }), "off");
    await user.click(screen.getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      const mine = backend.peek().scenes.find((s) => s.name === "会议纪要");
      expect(mine?.match).toEqual({ apps: [], title_contains: [] });
      expect(mine?.overrides.refine_enabled).toBe(false);
    });
    expect(within(page).getByRole("article", { name: "会议纪要" })).toHaveTextContent("AI 润色 关");
    // An existing scene opens in the same editor.
    await user.click(within(page).getByRole("button", { name: "编辑 会议纪要" }));
    expect(within(await screen.findByTestId("scene-editor")).getByLabelText("名称")).toHaveValue(
      "会议纪要",
    );
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByTestId("scene-editor")).toBeNull();
    backend.destroy();
  });

  it("a scene picked on the talk card runs the phone's takes, and a deleted one reads as none", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    await act(() =>
      backend.invoke("scenes_add", {
        scene: {
          name: "会议纪要",
          enabled: true,
          match: { apps: [], title_contains: [] },
          overrides: { refine_enabled: false },
        },
      }),
    );
    const mine = backend.peek().scenes.find((s) => s.name === "会议纪要");
    renderApp({ backend });
    const picker = await screen.findByRole("combobox", { name: "场景" });
    expect(picker).toHaveValue("");
    await user.selectOptions(picker, mine?.id ?? "");
    await waitFor(() => {
      expect(backend.peek().settings.pinned_scene).toBe(mine?.id);
    });

    // The take runs with it: no polish, and the history names the scene.
    await act(() => backend.invoke("dictation_start"));
    await act(() => backend.invoke("dictation_stop"));
    await waitFor(
      () => {
        expect(backend.peek().history_recent[0]?.scene?.name).toBe("会议纪要");
      },
      { timeout: 3000 },
    );
    expect(backend.peek().history_recent[0]?.refined).toBe(false);

    await user.click(screen.getByTestId("tab-settings"));
    expect(screen.getByTestId("settings-scenes")).toHaveTextContent("说话时使用「会议纪要」");
    await user.click(screen.getByTestId("settings-scenes"));
    const page = screen.getByTestId("phone-scenes");
    await user.click(within(page).getByRole("button", { name: "删除 会议纪要" }));
    const confirm = screen.getByRole("dialog", { name: "删除场景「会议纪要」？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().scenes.some((s) => s.name === "会议纪要")).toBe(false);
    });
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-scenes")).not.toHaveTextContent("说话时使用");
    await user.click(screen.getByTestId("tab-talk"));
    expect(await screen.findByRole("combobox", { name: "场景" })).toHaveValue("");
    await user.selectOptions(screen.getByRole("combobox", { name: "场景" }), "");
    await waitFor(() => {
      expect(backend.peek().settings.pinned_scene).toBeUndefined();
    });
    backend.destroy();
  });
});
