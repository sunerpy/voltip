import { type DictionaryEntry, type HistoryEntry, type ReplacementRule } from "@voltip/shared";
import { MockBackend, sampleDevices } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

/** Fixed pixel panel sizes and two-fixed-column grids broke the 1440 / 1920 px windows (Windows
 *  test 2026-09-24); `max-w-[…]` / `min-w-[…]` caps stay allowed (the root is `max-w-[1440px]`). */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

const NOW = 1_758_700_000_000;
const GIT = "00000000-0000-4000-9000-000000000001";
const PR = "00000000-0000-4000-9000-000000000002";
const OFF = "00000000-0000-4000-9000-000000000003";

function rule(
  id: string,
  name: string,
  pattern: string,
  replacement: string,
  extra: Partial<ReplacementRule> = {},
) {
  return {
    id,
    name,
    kind: "literal" as const,
    pattern,
    replacement,
    case_sensitive: true,
    enabled: true,
    created_at_ms: NOW,
    updated_at_ms: NOW,
    ...extra,
  };
}

function rules(): ReplacementRule[] {
  return [
    rule(GIT, "git push", "给他push", "git push"),
    rule(PR, "PR 编号", "\\bpr (\\d+)", "PR #$1", { kind: "regex", case_sensitive: false }),
    rule(OFF, "filler", "嗯，", "", { enabled: false }),
  ];
}

function history(): HistoryEntry[] {
  return [
    {
      id: "00000000-0000-4000-8000-0000000000a1",
      at_ms: NOW - 1000,
      raw_text: "给他push 然后看 pr 7",
      text: "git push 然后看 PR #7",
      refined: false,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 1000,
      asr_ms: 300,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      vocabulary: {
        corrections: [],
        rules: [
          { id: GIT, count: 1 },
          { id: PR, count: 1 },
        ],
      },
    },
  ];
}

function seeded(dictionary: DictionaryEntry[] = []) {
  return new MockBackend({
    devices: sampleDevices(1_758_700_000),
    now: () => NOW,
    rules: rules(),
    history: history(),
    dictionary,
  });
}

describe("Rules page", () => {
  it("regression: rules is fluid (no fixed-width panels)", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/rules", backend: seeded() });
    const page = await screen.findByTestId("page-rules");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6");
    // Allow-list: none (the search Input keeps a spacing width).
    expect(fixedSizeOffenders(page)).toEqual([]);
    const split = page.querySelector(".grid");
    expect(split?.className).toContain("grid-cols-1");
    expect(split?.className).toMatch(/lg:grid-cols-\[minmax\(0,3fr\)_minmax\(320px,2fr\)\]/);
    await user.click(screen.getByRole("button", { name: /新规则/ }));
    await user.click(screen.getByRole("button", { name: /运行/ }));
    expect(fixedSizeOffenders(page)).toEqual([]);
    expect(page.textContent).not.toMatch(/bridge|Bridge|MCP/);
  });

  it("regression: the rules page renders the core rules in execution order with hits and no sample data or deferred controls", async () => {
    renderApp({ path: "/rules", backend: seeded() });
    expect(await screen.findByRole("heading", { name: "替换规则", level: 2 })).toBeInTheDocument();
    expect(screen.getByText("3 条")).toBeInTheDocument();
    expect(screen.getByText("2 条启用")).toBeInTheDocument();
    const table = screen.getByRole("table", { name: "规则" });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(
      rows.map((r) => [...within(r).getAllByRole("cell")].slice(0, 6).map((c) => c.textContent)),
    ).toEqual([
      ["1", "git push", "字面", "给他push", "git push", "1"],
      ["2", "PR 编号", "正则", "\\bpr (\\d+)Aa", "PR #$1", "1"],
      ["3", "filler", "字面", "嗯，", "（删除）", "—"],
    ]);
    expect(screen.getByText("3 / 200 条 · 按这个顺序执行")).toBeInTheDocument();
    for (const name of ["导入 TOML", "导出 TOML"])
      expect(screen.getByRole("button", { name })).toBeEnabled();
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
    expect(document.body.textContent).not.toMatch(/尚未接入|示例|规则集|timeout_ms|作用域/);
  });

  it("regression: the editor has the core check the draft and saves through rules_add and rules_update while toggles reorder and delete follow the core", async () => {
    const user = userEvent.setup();
    const backend = seeded();
    renderApp({ path: "/rules", backend });
    await screen.findByRole("table", { name: "规则" });
    await user.click(screen.getByRole("button", { name: /新规则/ }));
    const editor = screen.getByTestId("rule-editor");
    expect(within(editor).getByTestId("draft-status")).toHaveTextContent("名称不能为空");
    await user.type(within(editor).getByLabelText("名称"), "git push");
    expect(within(editor).getByTestId("draft-status")).toHaveTextContent("已有同名规则");
    await user.clear(within(editor).getByLabelText("名称"));
    await user.type(within(editor).getByLabelText("名称"), "ticket");
    await user.click(within(editor).getByRole("radio", { name: "正则" }));
    await user.click(within(editor).getByLabelText("匹配"));
    await user.paste("(ticket");
    // The core compiles the pattern: its refusal is shown and saving is blocked.
    await waitFor(() => {
      expect(within(editor).getByTestId("draft-status")).toHaveTextContent(/正则无法编译/);
    });
    expect(within(editor).getByRole("button", { name: /保存/ })).toBeDisabled();
    await user.clear(within(editor).getByLabelText("匹配"));
    await user.paste("ticket (\\d+)");
    await user.type(within(editor).getByLabelText("替换"), "#$1");
    await waitFor(() => {
      expect(within(editor).getByTestId("draft-status")).toHaveTextContent("✓ 规则有效");
    });
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().rules.at(-1)).toMatchObject({
        name: "ticket",
        kind: "regex",
        pattern: "ticket (\\d+)",
        replacement: "#$1",
        case_sensitive: true,
        enabled: true,
      });
    });
    expect(screen.queryByTestId("rule-editor")).toBeNull();
    // Edit in place, Ctrl S saves.
    await user.click(screen.getByRole("button", { name: "编辑 git push" }));
    const to = within(screen.getByTestId("rule-editor")).getByLabelText("替换");
    await user.clear(to);
    await user.type(to, "git push --force-with-lease");
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => {
      expect(backend.peek().rules[0]?.replacement).toBe("git push --force-with-lease");
    });
    // Esc cancels an open editor.
    await user.click(screen.getByRole("button", { name: "编辑 filler" }));
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("rule-editor")).toBeNull();
    // Enable, reorder, delete (with a confirm).
    await user.click(screen.getByRole("switch", { name: "启用 filler" }));
    await waitFor(() => {
      expect(backend.peek().rules.find((r) => r.id === OFF)?.enabled).toBe(true);
    });
    await user.click(screen.getByRole("button", { name: "上移 PR 编号" }));
    await waitFor(() => {
      expect(
        backend
          .peek()
          .rules.map((r) => r.name)
          .slice(0, 2),
      ).toEqual(["PR 编号", "git push"]);
    });
    expect(screen.getByRole("button", { name: "上移 PR 编号" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "删除 filler" }));
    const confirm = screen.getByRole("dialog", { name: "删除规则 filler？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["PR 编号", "git push", "ticket"]);
    });
    // Kind filter and search.
    await user.click(screen.getByRole("radio", { name: "字面" }));
    expect(within(screen.getByRole("table", { name: "规则" })).getAllByRole("row")).toHaveLength(2);
    await user.type(screen.getByLabelText("搜索规则"), "zzz");
    expect(screen.getByText("没有匹配的规则")).toBeInTheDocument();
  });

  it("regression: the dry run is the core preview of the dictionary and every enabled rule with the last dictation and the draft being edited", async () => {
    const user = userEvent.setup();
    const dictionary: DictionaryEntry[] = [
      {
        id: "00000000-0000-4000-8000-000000000001",
        term: "给他",
        heard_as: ["给它"],
        enabled: true,
        source: { kind: "manual" },
        created_at_ms: NOW,
        updated_at_ms: NOW,
      },
    ];
    renderApp({ path: "/rules", backend: seeded(dictionary) });
    await screen.findByRole("table", { name: "规则" });
    expect(screen.getByText("运行后在这里看到前后差异")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "用最近一次听写" }));
    expect(screen.getByLabelText("识别出的原文")).toHaveValue("给他push 然后看 pr 7");
    await user.clear(screen.getByLabelText("识别出的原文"));
    await user.type(
      screen.getByLabelText("识别出的原文"),
      "嗯，给它push 看 PR 12{Control>}{Enter}{/Control}",
    );
    expect(await screen.findByTestId("dry-run-corrected")).toHaveTextContent(
      "嗯，给他push 看 PR 12",
    );
    expect(screen.getByTestId("dry-run-after")).toHaveTextContent("嗯，git push 看 PR #12");
    expect(screen.getByText("2 条规则命中 · 替换 2 处")).toBeInTheDocument();
    const hits = screen.getByRole("list", { name: "命中的规则" });
    expect(
      within(hits)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["1git push×1", "2PR 编号×1"]);
    // A draft being edited takes part while the switch is on, named as unsaved.
    await user.click(screen.getByRole("button", { name: /新规则/ }));
    const editor = screen.getByTestId("rule-editor");
    await user.type(within(editor).getByLabelText("名称"), "嗯");
    await user.type(within(editor).getByLabelText("匹配"), "嗯，");
    await waitFor(() => {
      expect(within(editor).getByTestId("draft-status")).toHaveTextContent("✓ 规则有效");
    });
    await user.click(screen.getByRole("button", { name: /^运行/ }));
    await waitFor(() => {
      expect(screen.getByTestId("dry-run-after")).toHaveTextContent(/^git push 看 PR #12$/);
    });
    expect(
      within(screen.getByRole("list", { name: "命中的规则" })).getByText("嗯（未保存）"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "包含正在编辑的规则" }));
    await user.click(screen.getByRole("button", { name: /^运行/ }));
    await waitFor(() => {
      expect(screen.getByTestId("dry-run-after")).toHaveTextContent("嗯，git push 看 PR #12");
    });
    // Nothing to change: the summary says so.
    await user.clear(screen.getByLabelText("识别出的原文"));
    await user.type(screen.getByLabelText("识别出的原文"), "无关的句子");
    await user.click(screen.getByRole("button", { name: /^运行/ }));
    expect(await screen.findByText("没有规则命中 · 文本未改变")).toBeInTheDocument();
  });

  it("regression: TOML export shows the core text and import merges or replaces refusing a bad file with its position", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const backend = seeded();
    renderApp({ path: "/rules", backend });
    await screen.findByRole("table", { name: "规则" });
    await user.click(screen.getByRole("button", { name: "导出 TOML" }));
    const exportDialog = await screen.findByRole("dialog", { name: "导出 TOML" });
    const text = within(exportDialog).getByTestId("rules-export-text");
    expect(text).toHaveValue(await backend.rulesExport());
    expect((text as HTMLTextAreaElement).value).toContain("pattern = '\\bpr (\\d+)'");
    await user.click(within(exportDialog).getByRole("button", { name: "复制" }));
    expect(writeText).toHaveBeenCalledWith(await backend.rulesExport());
    await user.click(within(exportDialog).getByRole("button", { name: "关闭" }));
    // Import: a bad file is refused whole, with the core's words; the list stays.
    await user.click(screen.getByRole("button", { name: "导入 TOML" }));
    const importDialog = screen.getByRole("dialog", { name: "导入 TOML" });
    const area = within(importDialog).getByLabelText("TOML 文本");
    await user.click(area);
    await user.paste('version = 1\n[[rule]]\nname = "x"\npatern = "y"\n');
    await user.click(within(importDialog).getByRole("button", { name: "导入" }));
    expect(await within(importDialog).findByTestId("rules-import-error")).toHaveTextContent(
      /TOML 无法解析.*line 4.*unknown field `patern`/,
    );
    expect(backend.peek().rules).toHaveLength(3);
    // Merge: same-name rules are updated in place, the rest appended.
    await user.clear(area);
    await user.paste(
      'version = 1\n[[rule]]\nname = "git push"\npattern = "推"\nreplacement = "push"\n[[rule]]\nname = "new"\npattern = "n"\n',
    );
    await user.click(within(importDialog).getByRole("button", { name: "导入" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => [r.name, r.pattern])).toEqual([
        ["git push", "推"],
        ["PR 编号", "\\bpr (\\d+)"],
        ["filler", "嗯，"],
        ["new", "n"],
      ]);
    });
    expect(screen.getByText("已导入 · 合并")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "导入 TOML" })).toBeNull();
    // Replace: the file becomes the whole list.
    await user.click(screen.getByRole("button", { name: "导入 TOML" }));
    const again = screen.getByRole("dialog", { name: "导入 TOML" });
    await user.click(within(again).getByRole("radio", { name: "替换" }));
    expect(within(again).getByText("文件里的规则成为全部规则")).toBeInTheDocument();
    await user.click(within(again).getByLabelText("TOML 文本"));
    await user.paste('version = 1\n[[rule]]\nname = "only"\npattern = "o"\n');
    await user.click(within(again).getByRole("button", { name: "导入" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["only"]);
    });
  });

  it("regression: no rules shows the empty state and Ctrl N opens the editor and export failures are reported", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW, history: [] });
    vi.spyOn(backend, "rulesExport").mockRejectedValueOnce("rules: 无法导出 TOML");
    renderApp({ path: "/rules", backend });
    expect(await screen.findByText("还没有规则")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "用最近一次听写" })).toBeDisabled();
    await user.keyboard("{Control>}n{/Control}");
    expect(screen.getByTestId("rule-editor")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /取消/ }));
    await user.click(screen.getByRole("button", { name: "新建规则" }));
    expect(screen.getByTestId("rule-editor")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "导出 TOML" }));
    expect(await screen.findByText("导出失败：无法导出 TOML")).toBeInTheDocument();
    vi.spyOn(backend, "vocabularyPreview").mockRejectedValueOnce(
      new Error("rules: 试写文本最多 64 KiB"),
    );
    await user.click(screen.getByRole("button", { name: /^运行/ }));
    expect(await screen.findByTestId("dry-run-error")).toHaveTextContent(
      "试运行失败：试写文本最多 64 KiB",
    );
  });
});
