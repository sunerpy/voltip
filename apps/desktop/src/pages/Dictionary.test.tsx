import { type DictionaryEntry, type HistoryEntry, type ReplacementRule } from "@voltip/shared";
import { MockBackend, sampleDevices } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
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
const VOLTIP = "00000000-0000-4000-8000-000000000001";
const GOOD = "00000000-0000-4000-8000-000000000002";
const FETCH = "00000000-0000-4000-8000-000000000003";
const ROW = "00000000-0000-4000-8000-0000000000a1";

function entry(id: string, term: string, heardAs: string[], extra: Partial<DictionaryEntry> = {}) {
  return {
    id,
    term,
    heard_as: heardAs,
    enabled: true,
    source: { kind: "manual" as const },
    created_at_ms: NOW,
    updated_at_ms: NOW,
    ...extra,
  };
}

function dictionary(): DictionaryEntry[] {
  return [
    entry(VOLTIP, "Voltip", ["沃提普"]),
    entry(GOOD, "good idea", ["谷歌IDR"], { source: { kind: "history", history_id: ROW } }),
    entry(FETCH, "fetchUser", ["fetch user"], { enabled: false }),
  ];
}

function history(): HistoryEntry[] {
  const base = {
    refined: false,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 1000,
    asr_ms: 300,
    outcome: { kind: "inserted" as const, via: "paste" as const },
    starred: false,
    mode: "whole_take" as const,
    kind: "dictation" as const,
  };
  return [
    {
      ...base,
      id: ROW,
      at_ms: NOW - 1000,
      raw_text: "用沃提普写一个谷歌IDR",
      text: "用Voltip写一个good idea",
      vocabulary: {
        corrections: [
          { id: VOLTIP, count: 2 },
          { id: GOOD, count: 1 },
        ],
        rules: [],
      },
    },
    {
      ...base,
      id: "00000000-0000-4000-8000-0000000000a2",
      at_ms: NOW - 60_000,
      raw_text: "沃提普",
      text: "Voltip",
      vocabulary: { corrections: [{ id: VOLTIP, count: 1 }], rules: [] },
    },
  ];
}

function seeded(rules: ReplacementRule[] = []) {
  return new MockBackend({
    devices: sampleDevices(1_758_700_000),
    now: () => NOW,
    dictionary: dictionary(),
    history: history(),
    rules,
  });
}

describe("Dictionary page", () => {
  it("regression: dictionary is fluid (no fixed-width panels)", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", backend: seeded() });
    const page = await screen.findByTestId("page-dictionary");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6");
    // Allow-list: none (the search Input keeps a spacing width; the table stretches).
    expect(fixedSizeOffenders(page)).toEqual([]);
    const split = page.querySelector(".grid");
    expect(split?.className).toContain("grid-cols-1");
    expect(split?.className).toMatch(/lg:grid-cols-\[minmax\(0,1fr\)_minmax\(280px,360px\)\]/);
    await user.click(screen.getByRole("button", { name: /新建词条/ }));
    await user.click(screen.getByRole("button", { name: "编辑 Voltip" }));
    expect(fixedSizeOffenders(page)).toEqual([]);
  });

  it("regression: the dictionary page renders the core entries with counts and history hits and no sample data or deferred controls", async () => {
    renderApp({ path: "/dictionary", backend: seeded() });
    expect(await screen.findByRole("heading", { name: "个人词典", level: 2 })).toBeInTheDocument();
    expect(screen.getByText("3 条")).toBeInTheDocument();
    expect(screen.getByText("2 条启用")).toBeInTheDocument();
    expect(screen.getByTestId("dictionary-explain")).toHaveTextContent(
      "最多 500 条 · 每条最多 10 个曾听成",
    );
    const table = screen.getByRole("table", { name: "词条表" });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows.map((r) => within(r).getAllByRole("cell")[1]?.textContent)).toEqual([
      "Voltip",
      "good idea",
      "fetchUser",
    ]);
    expect(within(rows[1] ?? table).getByText("来自历史")).toBeInTheDocument();
    expect(within(rows[0] ?? table).getByText("手动")).toBeInTheDocument();
    // Hits: summed over the retained history rows, strongest first.
    expect(within(rows[0] ?? table).getByText("3")).toBeInTheDocument();
    const chips = screen.getByTestId("dictionary-hits");
    expect(chips.textContent).toBe("历史记录里的命中Voltip×3good idea×1");
    expect(
      within(table)
        .getAllByRole("switch")
        .map((s) => s.getAttribute("aria-checked")),
    ).toEqual(["true", "true", "false"]);
    // Nothing fixture-shaped or deferred is left.
    expect(screen.queryByTestId("sample-footnote")).toBeNull();
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
    expect(screen.queryByRole("button", { name: /CSV/ })).toBeNull();
    expect(document.body.textContent).not.toMatch(/热词|尚未接入|score|示例/);
    expect(screen.getByText("输入文本后，这里会显示词典纠正后的结果。")).toBeInTheDocument();
  });

  it("regression: add edit enable reorder and delete go through the dictionary commands and the list follows the core", async () => {
    const user = userEvent.setup();
    const backend = seeded();
    renderApp({ path: "/dictionary", backend });
    await screen.findByRole("table", { name: "词条表" });
    // Add: the separators split the misheard forms; a duplicate term is caught while typing.
    await user.click(screen.getByRole("button", { name: /新建词条/ }));
    const add = screen.getByTestId("add-row");
    const term = within(add).getByLabelText("正确写法");
    await user.type(term, "voltip");
    expect(within(add).getByText("词典里已有这个写法")).toBeInTheDocument();
    expect(within(add).getByRole("button", { name: "保存" })).toBeDisabled();
    await user.clear(term);
    await user.type(term, "Teams");
    await user.type(within(add).getByLabelText("曾听成"), "听写，teems · 听 写");
    await user.click(within(add).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.at(-1)).toMatchObject({
        term: "Teams",
        heard_as: ["听写", "teems", "听 写"],
        enabled: true,
        source: { kind: "manual" },
      });
    });
    expect(screen.queryByTestId("add-row")).toBeNull();
    // A draft the core refuses on its own keeps the editor open with the core's words.
    await user.click(screen.getByRole("button", { name: /新建词条/ }));
    await user.type(screen.getByLabelText("正确写法"), "World");
    await user.type(screen.getByLabelText("曾听成"), "World");
    await user.click(within(screen.getByTestId("add-row")).getByRole("button", { name: "保存" }));
    expect(await screen.findByText("误识别写法「World」和正确写法相同")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("add-row")).toBeNull();
    // A clash with another entry is the core's error event (a toast); the list stays.
    await user.click(screen.getByRole("button", { name: /新建词条/ }));
    await user.type(screen.getByLabelText("正确写法"), "Planet");
    await user.type(screen.getByLabelText("曾听成"), "沃提普{Enter}");
    expect(
      await screen.findByText(/出错了 · 「沃提普」已是「Voltip」的误识别写法/),
    ).toBeInTheDocument();
    expect(backend.peek().dictionary.map((e) => e.term)).not.toContain("Planet");
    // Edit in place: Enter saves.
    await user.click(screen.getByRole("button", { name: "编辑 Voltip" }));
    const heard = screen.getByLabelText("曾听成");
    await user.clear(heard);
    await user.type(heard, "沃提普, volt ip{Enter}");
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃提普", "volt ip"]);
    });
    // Enable, reorder, delete.
    await user.click(screen.getByRole("switch", { name: "启用 fetchUser" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.find((e) => e.id === FETCH)?.enabled).toBe(true);
    });
    expect(screen.getByRole("button", { name: "上移 Voltip" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "下移 Voltip" }));
    await waitFor(() => {
      expect(
        backend
          .peek()
          .dictionary.map((e) => e.term)
          .slice(0, 2),
      ).toEqual(["good idea", "Voltip"]);
    });
    await user.click(screen.getByRole("button", { name: "删除 good idea" }));
    await user.click(screen.getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual([
        "Voltip",
        "fetchUser",
        "Teams",
      ]);
    });
    // Search and the filter.
    await user.type(screen.getByLabelText("搜索词条"), "volt ip");
    const table = screen.getByRole("table", { name: "词条表" });
    expect(within(table).getAllByRole("row")).toHaveLength(2);
    await user.clear(screen.getByLabelText("搜索词条"));
    await user.click(screen.getByRole("radio", { name: "已停用" }));
    expect(screen.getByText("没有匹配的词条")).toBeInTheDocument();
  });

  it("regression: the test panel corrects through the core preview and shows what fired including the rules after the dictionary", async () => {
    const user = userEvent.setup();
    const rule: ReplacementRule = {
      id: "00000000-0000-4000-9000-000000000001",
      name: "idea",
      kind: "literal",
      pattern: "good idea",
      replacement: "好主意",
      case_sensitive: true,
      enabled: true,
      created_at_ms: NOW,
      updated_at_ms: NOW,
    };
    renderApp({ path: "/dictionary", backend: seeded([rule]) });
    await screen.findByRole("table", { name: "词条表" });
    await user.click(screen.getByRole("button", { name: "用最近一次听写" }));
    expect(screen.getByLabelText("识别出的文字")).toHaveValue("用沃提普写一个谷歌IDR");
    expect(await screen.findByTestId("corrected")).toHaveTextContent("用Voltip写一个good idea");
    expect(screen.getByTestId("dictionary-test-summary")).toHaveTextContent("命中 2 处");
    const hits = screen.getByRole("list", { name: "命中列表" });
    expect(
      within(hits)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Voltip×1", "good idea×1"]);
    expect(screen.getByTestId("corrected-rules")).toHaveTextContent("用Voltip写一个好主意");
    await user.click(screen.getByRole("button", { name: "清空" }));
    await user.type(screen.getByLabelText("识别出的文字"), "没有要改的");
    expect(await screen.findByText("没有命中任何曾听成。")).toBeInTheDocument();
    expect(screen.queryByTestId("corrected-rules")).toBeNull();
  });

  it("regression: an empty dictionary shows its empty state and Ctrl N opens a new entry", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", mock: { history: [] } });
    expect(await screen.findByText("还没有词条。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "用最近一次听写" })).toBeDisabled();
    expect(screen.queryByTestId("dictionary-hits")).toBeNull();
    await user.keyboard("{Control>}n{/Control}");
    expect(screen.getByTestId("add-row")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(
      within(screen.getByRole("table", { name: "词条表" })).getByRole("button", {
        name: "新建词条",
      }),
    );
    expect(screen.getByTestId("add-row")).toBeInTheDocument();
  });

  it("regression: a failed preview or a failed toggle is reported and not swallowed", async () => {
    const user = userEvent.setup();
    const backend = seeded();
    const preview = vi
      .spyOn(backend, "vocabularyPreview")
      .mockRejectedValue("rules: 试写文本最多 64 KiB");
    renderApp({ path: "/dictionary", backend });
    await screen.findByRole("table", { name: "词条表" });
    await user.type(screen.getByLabelText("识别出的文字"), "x");
    expect(await screen.findByTestId("dictionary-test-error")).toHaveTextContent(
      "试运行失败：试写文本最多 64 KiB",
    );
    expect(preview).toHaveBeenCalled();
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("dictionary: 没有 id"));
    await user.click(screen.getByRole("switch", { name: "启用 Voltip" }));
    expect(await screen.findByText("出错了 · 没有 id")).toBeInTheDocument();
    act(() => {
      backend.publish({ type: "dictionary", entries: [] });
    });
    expect(await screen.findByText("还没有词条。")).toBeInTheDocument();
  });
});
