// 设置 › 替换规则 on the phone (docs/dictation.md §16): the core's rules in order, each switched in
// its row and edited full screen against the core's own check; TOML comes in pasted and goes out
// copied or through the share sheet.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import type { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

async function openRules() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-rules"));
  return screen.findByTestId("phone-rules");
}

async function seed(
  backend: MockBackend,
  name: string,
  pattern: string,
  replacement: string,
  kind: "literal" | "regex" = "literal",
) {
  await act(async () => {
    await backend.invoke("rules_add", {
      rule: { name, kind, pattern, replacement, case_sensitive: true, enabled: true },
    });
  });
}

describe("设置 › 替换规则", () => {
  it("starts empty; a new rule is checked by the core, tried on a text and saved", async () => {
    const { backend } = await renderApp();
    const page = await openRules();
    expect(within(page).getByText("暂无规则")).toBeOnTheScreen();
    expect(screen.getByTestId("rules-footer")).toHaveTextContent("0 / 200 条 · 按此顺序执行");
    await fireEvent.press(screen.getByTestId("rules-add"));
    const editor = await screen.findByTestId("rule-editor");
    expect(within(editor).getByText("新建规则")).toBeOnTheScreen();
    expect(within(editor).getByTestId("dialog-save")).toBeDisabled();
    await fireEvent.changeText(within(editor).getByTestId("rule-name"), "产品名");
    await fireEvent.changeText(within(editor).getByTestId("rule-pattern"), "维他命");
    await fireEvent.changeText(within(editor).getByTestId("rule-replacement"), "Voltip");
    await waitFor(() => {
      expect(within(editor).getByTestId("rule-status")).toHaveTextContent("✓ 规则有效");
    });
    await fireEvent.changeText(within(editor).getByTestId("rule-sample"), "我在用维他命输入");
    await waitFor(() => {
      expect(within(editor).getByTestId("rule-test-result")).toHaveTextContent(
        "规则替换后我在用Voltip输入",
      );
    });
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    await waitFor(() => {
      expect(screen.queryByTestId("rule-editor")).toBeNull();
    });
    expect(backend.peek().rules).toMatchObject([
      { name: "产品名", kind: "literal", pattern: "维他命", replacement: "Voltip", enabled: true },
    ]);
    expect(within(page).getByLabelText("编辑 产品名")).toHaveTextContent(/维他命 → Voltip$/);
    expect(screen.getByTestId("rules-footer")).toHaveTextContent(/^1 \/ 200 条/);
  });

  it("refuses a taken name and a regex the core cannot compile, and keeps the editor open", async () => {
    const { backend } = await renderApp();
    await seed(backend, "产品名", "维他命", "Voltip");
    await openRules();
    await fireEvent.press(screen.getByTestId("rules-add"));
    const editor = await screen.findByTestId("rule-editor");
    await fireEvent.changeText(within(editor).getByTestId("rule-name"), "产品名");
    expect(within(editor).getByText("已有同名规则")).toBeOnTheScreen();
    expect(within(editor).getByTestId("dialog-save")).toBeDisabled();
    await fireEvent.changeText(within(editor).getByTestId("rule-name"), "日期");
    await fireEvent.press(within(editor).getByTestId("rule-kind-regex"));
    await fireEvent.changeText(within(editor).getByTestId("rule-pattern"), "(\\d+");
    await waitFor(() => {
      expect(within(editor).getByTestId("rule-status")).toHaveTextContent(/正则无法编译/);
    });
    expect(within(editor).getByTestId("dialog-save")).toBeDisabled();
    await fireEvent.changeText(within(editor).getByTestId("rule-pattern"), "(\\d+)年");
    await fireEvent.changeText(within(editor).getByTestId("rule-replacement"), "$1 年");
    await fireEvent.press(within(editor).getByTestId("rule-case"));
    await waitFor(() => {
      expect(within(editor).getByTestId("rule-status")).toHaveTextContent("✓ 规则有效");
    });
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["产品名", "日期"]);
    });
    expect(backend.peek().rules[1]).toMatchObject({
      kind: "regex",
      pattern: "(\\d+)年",
      replacement: "$1 年",
      case_sensitive: false,
    });
    expect(screen.getByLabelText("编辑 日期")).toHaveTextContent(/日期\s+正则/);
  });

  it("switches a rule in its row, moves it in the editor and deletes it after asking", async () => {
    const { backend } = await renderApp();
    await seed(backend, "第一条", "甲", "");
    await seed(backend, "第二条", "乙", "丙");
    const page = await openRules();
    expect(within(page).getByLabelText("编辑 第一条")).toHaveTextContent(/甲 → （删除）$/);
    await fireEvent(within(page).getByLabelText("启用 第一条"), "valueChange", false);
    await waitFor(() => {
      expect(backend.peek().rules[0]?.enabled).toBe(false);
    });
    await fireEvent.press(within(page).getByLabelText("编辑 第二条"));
    const editor = await screen.findByTestId("rule-editor");
    expect(within(editor).getByText("匹配顺序第 2 条，共 2 条")).toBeOnTheScreen();
    expect(within(editor).getByTestId("rule-down")).toBeDisabled();
    await fireEvent.press(within(editor).getByTestId("rule-up"));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["第二条", "第一条"]);
    });
    expect(within(editor).getByText("匹配顺序第 1 条，共 2 条")).toBeOnTheScreen();
    await fireEvent.press(within(editor).getByTestId("rule-delete"));
    expect(await screen.findByText("删除规则 第二条？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm-cancel"));
    expect(backend.peek().rules).toHaveLength(2);
    await fireEvent.press(within(editor).getByTestId("rule-delete"));
    await fireEvent.press(await screen.findByTestId("shell-confirm"));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["第一条"]);
    });
    await waitFor(() => {
      expect(screen.queryByTestId("rule-editor")).toBeNull();
    });
  });

  it("edits a rule in place, and says why when the core refuses a change", async () => {
    const { backend } = await renderApp();
    await seed(backend, "产品名", "维他命", "Voltip");
    const page = await openRules();
    await fireEvent.press(within(page).getByLabelText("编辑 产品名"));
    let editor = await screen.findByTestId("rule-editor");
    expect(within(editor).getByText("编辑规则")).toBeOnTheScreen();
    expect(within(editor).queryByTestId("rule-up")).toBeNull();
    await fireEvent.press(within(editor).getByTestId("rule-kind-literal"));
    await fireEvent.changeText(within(editor).getByTestId("rule-replacement"), "VOLTIP");
    await waitFor(() => {
      expect(within(editor).getByTestId("dialog-save")).toBeEnabled();
    });
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    await waitFor(() => {
      expect(backend.peek().rules[0]?.replacement).toBe("VOLTIP");
    });
    // The core refuses the save: the reason stays in the editor, which stays open.
    await fireEvent.press(within(page).getByLabelText("编辑 产品名"));
    editor = await screen.findByTestId("rule-editor");
    await fireEvent.changeText(within(editor).getByTestId("rule-replacement"), "V");
    await waitFor(() => {
      expect(within(editor).getByTestId("dialog-save")).toBeEnabled();
    });
    const invoke = jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("磁盘已满"));
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    expect(await within(editor).findByText("磁盘已满")).toBeOnTheScreen();
    invoke.mockRestore();
    await fireEvent.press(within(editor).getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("rule-editor")).toBeNull();
    });
    expect(backend.peek().rules[0]?.replacement).toBe("VOLTIP");
  });

  it("imports pasted TOML, merged or in place of every rule, and shows what is wrong with it", async () => {
    const { backend } = await renderApp();
    await seed(backend, "旧规则", "旧", "新");
    await openRules();
    await fireEvent.press(screen.getByTestId("rules-import-open"));
    let dialog = await screen.findByTestId("rules-import");
    expect(within(dialog).getByTestId("dialog-save")).toBeDisabled();
    expect(within(dialog).getByText("同名规则原位更新，其余追加到末尾")).toBeOnTheScreen();
    await fireEvent.changeText(within(dialog).getByTestId("rules-import-text"), "version = 2");
    await fireEvent.press(within(dialog).getByTestId("dialog-save"));
    expect(await within(dialog).findByTestId("rules-import-error")).toHaveTextContent(
      /不支持的 version = 2/,
    );
    await fireEvent.changeText(
      within(dialog).getByTestId("rules-import-text"),
      'version = 1\n\n[[rule]]\nname = "导入的"\npattern = "甲"\nreplacement = "乙"\n',
    );
    expect(within(dialog).queryByTestId("rules-import-error")).toBeNull();
    await fireEvent.press(within(dialog).getByTestId("dialog-save"));
    expect(await screen.findByText("已导入 · 合并")).toBeOnTheScreen();
    expect(backend.peek().rules.map((r) => r.name)).toEqual(["旧规则", "导入的"]);
    await fireEvent.press(screen.getByTestId("rules-import-open"));
    dialog = await screen.findByTestId("rules-import");
    await fireEvent.press(within(dialog).getByTestId("rules-import-replace"));
    expect(within(dialog).getByText("文件里的规则成为全部规则")).toBeOnTheScreen();
    await fireEvent.changeText(
      within(dialog).getByTestId("rules-import-text"),
      'version = 1\n\n[[rule]]\nname = "唯一"\nkind = "regex"\npattern = "\\\\d+"\nreplacement = "#"\n',
    );
    await fireEvent.press(within(dialog).getByTestId("dialog-save"));
    expect(await screen.findByText("已导入 · 替换")).toBeOnTheScreen();
    expect(backend.peek().rules.map((r) => [r.name, r.kind])).toEqual([["唯一", "regex"]]);
  });

  it("exports the rules as TOML to copy or share, and says so when that fails", async () => {
    const { backend } = await renderApp();
    await seed(backend, "产品名", "维他命", "Voltip");
    await openRules();
    await fireEvent.press(screen.getByTestId("rules-export-open"));
    const dialog = await screen.findByTestId("rules-export");
    expect(within(dialog).getByTestId("rules-export-text")).toHaveTextContent(/name = "产品名"/);
    await fireEvent.press(within(dialog).getByTestId("rules-export-copy"));
    expect(await screen.findByText("已复制 TOML")).toBeOnTheScreen();
    expect(backend.phoneClipboard).toMatch(/pattern = "维他命"/);
    await fireEvent.press(within(dialog).getByTestId("rules-export-share"));
    await waitFor(() => {
      expect(backend.shared).toHaveLength(1);
    });
    expect(backend.shared[0]).toMatch(/replacement = "Voltip"/);
    const paste = jest
      .spyOn(backend, "pasteText")
      .mockResolvedValueOnce({ kind: "failed", reason: "inject" })
      .mockRejectedValueOnce(new Error("gone"));
    await fireEvent.press(within(dialog).getByTestId("rules-export-copy"));
    expect(await screen.findByText("复制失败")).toBeOnTheScreen();
    await fireEvent.press(within(dialog).getByTestId("rules-export-copy"));
    await waitFor(() => {
      expect(paste).toHaveBeenCalledTimes(2);
    });
    paste.mockRestore();
    const invoke = jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("没有分享应用"));
    await fireEvent.press(within(dialog).getByTestId("rules-export-share"));
    expect(await screen.findByText(/没有分享应用/)).toBeOnTheScreen();
    invoke.mockRestore();
    await fireEvent.press(within(dialog).getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("rules-export")).toBeNull();
    });
    jest.spyOn(backend, "rulesExport").mockRejectedValueOnce(new Error("读不出规则"));
    await fireEvent.press(screen.getByTestId("rules-export-open"));
    expect(await screen.findByText("导出失败：读不出规则")).toBeOnTheScreen();
    expect(screen.queryByTestId("rules-export")).toBeNull();
  });

  it("says so when switching a rule fails", async () => {
    const { backend } = await renderApp();
    await seed(backend, "产品名", "维他命", "Voltip");
    const page = await openRules();
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("核心忙"));
    await fireEvent(within(page).getByLabelText("启用 产品名"), "valueChange", false);
    expect(await screen.findByText(/核心忙/)).toBeOnTheScreen();
    expect(backend.peek().rules[0]?.enabled).toBe(true);
  });
});
