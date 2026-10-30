import { type DictionaryEntry, type HistoryEntry, type ReplacementRule } from "@voltip/shared";
import { MockBackend, type MockBackendOptions, sampleDevices } from "@voltip/shared/mock";
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
      source: { kind: "manual" },
      pending: false,
    };
    renderApp({ path: "/dictionary", backend: seeded([rule]) });
    await screen.findByRole("table", { name: "词条表" });
    await user.click(screen.getByRole("button", { name: "用最近一次听写" }));
    expect(screen.getByLabelText("识别文本")).toHaveValue("用沃提普写一个谷歌IDR");
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
    await user.type(screen.getByLabelText("识别文本"), "没有要改的");
    expect(await screen.findByText("没有命中任何曾听成。")).toBeInTheDocument();
    expect(screen.queryByTestId("corrected-rules")).toBeNull();
  });

  it("regression: an empty dictionary shows its empty state and Ctrl N opens a new entry", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", mock: { history: [] } });
    expect(await screen.findByText("暂无词条。")).toBeInTheDocument();
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
    await user.type(screen.getByLabelText("识别文本"), "x");
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
    expect(await screen.findByText("暂无词条。")).toBeInTheDocument();
  });
});

describe("Dictionary page · entries an AI tool wrote (docs/dictation.md §25)", () => {
  it("their source names the tool, 外部工具 shows only them, and the filter goes once none is left", async () => {
    const user = userEvent.setup();
    const backend = seeded();
    renderApp({ path: "/dictionary", backend });
    const table = await screen.findByRole("table", { name: "词条表" });
    // No such entry yet: no such filter.
    expect(screen.queryByRole("radio", { name: "外部工具" })).toBeNull();
    let id = "";
    act(() => {
      id = backend.simulateMcpEntry("Claude Code", {
        term: "Paseo",
        heard_as: ["帕赛欧"],
        enabled: true,
      });
    });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(4);
    const source = within(rows[3] ?? table).getByText("外部工具 · Claude Code");
    expect(source).toHaveAttribute("data-user-text");
    expect(source.closest("[title]")).toHaveAttribute("title", "外部工具 · Claude Code");
    await user.click(screen.getByRole("radio", { name: "外部工具" }));
    expect(
      within(table)
        .getAllByRole("row")
        .slice(1)
        .map((r) => within(r).getAllByRole("cell")[1]?.textContent),
    ).toEqual(["Paseo"]);
    // Undone from the settings (or deleted here): the whole list shows again.
    await act(async () => {
      await backend.invoke("dictionary_remove", { id });
    });
    expect(screen.queryByRole("radio", { name: "外部工具" })).toBeNull();
    expect(screen.getByRole("radio", { name: "全部" })).toBeChecked();
    expect(within(table).getAllByRole("row")).toHaveLength(4);
  });

  it("the English source reads the same", async () => {
    const backend = seeded();
    backend.simulateMcpEntry("Codex", { term: "Paseo", heard_as: ["帕赛欧"], enabled: true });
    renderApp({ path: "/dictionary", backend, systemLanguage: "en-US" });
    const table = await screen.findByRole("table", { name: "Entries" });
    // Regression (make verify 2026-10-13): the table can render before the core's lists arrive,
    // so the entry and its filter are waited for, not read at once.
    expect(await within(table).findByText("External tool · Codex")).toBeInTheDocument();
    expect(await screen.findByRole("radio", { name: "External tools" })).toBeInTheDocument();
  });
});

describe("Dictionary page · learning suggestions (docs/dictation.md §16.10)", () => {
  // The history's times are relative to the real clock: the page reads 「2 小时前」 from `useNow`.
  const AT = Date.now();
  const HOUR = 3_600_000;
  const SA = "00000000-0000-4000-8000-0000000000b1";
  const SB = "00000000-0000-4000-8000-0000000000b2";
  const SC = "00000000-0000-4000-8000-0000000000b3";
  const SD = "00000000-0000-4000-8000-0000000000b4";
  const SE = "00000000-0000-4000-8000-0000000000b5";
  const FEI = "00000000-0000-4000-8000-000000000004";
  const BACK = "00000000-0000-4000-9000-0000000000c1";

  function take(id: string, atMs: number, raw: string, text: string, refined = true): HistoryEntry {
    return {
      id,
      at_ms: atMs,
      raw_text: raw,
      text,
      refined,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 1000,
      asr_ms: 300,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
    };
  }

  /** Newest first: 非 → 飞 twice, 维他命 → Voltip (an entry already has that spelling), 库伯内特斯 →
   *  Kubernetes, and a take not polished whose final text keeps 「非」. */
  function takes(): HistoryEntry[] {
    return [
      take(SA, AT - 2 * HOUR, "发到非书群", "发到飞书群。"),
      take(SB, AT - 3 * HOUR, "又是非书", "又是飞书。"),
      take(SE, AT - 4 * HOUR, "这个非常好", "这个非常好", false),
      take(SC, AT - 5 * HOUR, "用维他命测了一下", "用 Voltip 测了一下。"),
      take(SD, AT - 72 * HOUR, "库伯内特斯部署", "Kubernetes 部署"),
    ];
  }

  function learning(options: Partial<MockBackendOptions> = {}) {
    return new MockBackend({
      devices: sampleDevices(1_758_700_000),
      now: () => NOW,
      dictionary: dictionary(),
      history: takes(),
      ...options,
    });
  }

  function suggestionTable(): HTMLElement {
    return screen.getByRole("table", { name: "学习建议" });
  }

  /** The table's suggestion rows (not the detail rows under them): pair, count, last time. */
  function shownPairs(): string[][] {
    return within(suggestionTable())
      .getAllByRole("row")
      .slice(1)
      .filter((r) => r.querySelector("[data-testid='suggestion-pair']") !== null)
      .map((r) =>
        within(r)
          .getAllByRole("cell")
          .slice(0, 3)
          .map((c) => c.textContent ?? ""),
      );
  }

  function rowOf(heard: string): HTMLElement {
    const row = within(suggestionTable())
      .getAllByRole("row")
      .find((r) =>
        r.querySelector("[data-testid='suggestion-pair']")?.textContent?.startsWith(`${heard}→`),
      );
    if (row === undefined) throw new Error(`no row for ${heard}`);
    return row;
  }

  async function openSuggestions(user: ReturnType<typeof userEvent.setup>, name = /^学习建议/) {
    await user.click(await screen.findByRole("tab", { name }));
  }

  it("the page has two views; 学习建议 counts what the clean-up changed and lists it, most frequent first", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", backend: learning() });
    const tabs = await screen.findByRole("tablist", { name: "词典视图" });
    const entries = within(tabs).getByRole("tab", { name: "词条" });
    expect(entries).toHaveAttribute("aria-selected", "true");
    const suggestions = await within(tabs).findByRole("tab", { name: "学习建议（3）" });
    expect(suggestions).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("tabpanel", { name: "词条" })).toContainElement(
      screen.getByRole("table", { name: "词条表" }),
    );
    // The arrow keys move between the views, as a tab list's do.
    entries.focus();
    await user.keyboard("{ArrowRight}");
    expect(suggestions).toHaveFocus();
    expect(suggestions).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("table", { name: "词条表" })).toBeNull();
    const panel = screen.getByRole("tabpanel", { name: "学习建议（3）" });
    expect(panel).toContainElement(suggestionTable());
    expect(shownPairs()).toEqual([
      ["非→飞", "2", "2 小时前"],
      ["维他命→Voltip", "1", "5 小时前"],
      ["库伯内特斯→Kubernetes", "1", "3 天前"],
    ]);
    expect(screen.getByTestId("suggestions-explain")).toHaveTextContent("只逐字比较，不调用 AI");
    expect(screen.getByTestId("suggestions-explain")).toHaveTextContent(
      "加入或忽略后可以立即撤销；离开词典页后无法撤销。",
    );
    expect(screen.getByTestId("suggestions-footer")).toHaveTextContent(
      "最近 5 条记录 · 4 条经过 AI 润色",
    );
    await user.keyboard("{Home}");
    expect(entries).toHaveFocus();
    await user.keyboard("{End}");
    expect(suggestions).toHaveFocus();
    // 新建词条 (and Ctrl N) goes back to the entries.
    await user.click(screen.getByRole("button", { name: /新建词条/ }));
    expect(entries).toHaveAttribute("aria-selected", "true");
    expect(screen.getByTestId("add-row")).toBeInTheDocument();
  });

  it("regression: 学习建议 takes the page's whole width, and the test panel comes back with the entries", async () => {
    // The real app at its default 1152 × 720 window (2026-10-30): beside the test panel the pair
    // column was cut to 「维… → Vol…」 and its header ran into 次数.
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", backend: learning() });
    await user.type(await screen.findByLabelText("识别文本"), "又是非书");
    await openSuggestions(user, /^学习建议（3）$/);
    const page = screen.getByTestId("page-dictionary");
    expect(screen.queryByLabelText("识别文本")).toBeNull();
    expect(page.querySelector(".grid")?.className).not.toMatch(/lg:grid-cols-/);
    expect(fixedSizeOffenders(page)).toEqual([]);
    const header = within(suggestionTable()).getByRole("columnheader", {
      name: "识别结果 → 润色结果",
    });
    expect(header.style.minWidth).toBe("160px");
    await user.click(screen.getByRole("tab", { name: "词条" }));
    expect(screen.getByLabelText("识别文本")).toHaveValue("又是非书");
    expect(page.querySelector(".grid")?.className).toMatch(
      /lg:grid-cols-\[minmax\(0,1fr\)_minmax\(280px,360px\)\]/,
    );
  });

  it("加入词典 saves the pair, the row stays in place with 撤销, and 撤销 takes it back even after switching views", async () => {
    const user = userEvent.setup();
    const backend = learning();
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议（3）$/);
    // Into the entry with that spelling.
    await user.click(screen.getByRole("button", { name: "加入词典：维他命 → Voltip" }));
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃提普", "维他命"]);
    });
    expect(within(rowOf("维他命")).getByText("已加入词典")).toBeInTheDocument();
    expect(await screen.findByRole("tab", { name: "学习建议（2）" })).toBeInTheDocument();
    // A new entry, from the take the pair was first seen in.
    await user.click(screen.getByRole("button", { name: "加入词典：库伯内特斯 → Kubernetes" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.at(-1)).toMatchObject({
        term: "Kubernetes",
        heard_as: ["库伯内特斯"],
        enabled: true,
        source: { kind: "history", history_id: SD },
      });
    });
    expect(await screen.findByRole("tab", { name: "学习建议（1）" })).toBeInTheDocument();
    // The rows stay where they were while the page is open.
    expect(shownPairs().map((r) => r[0])).toEqual([
      "非→飞",
      "维他命→Voltip",
      "库伯内特斯→Kubernetes",
    ]);
    // 撤销 still works after a look at the entries.
    await user.click(screen.getByRole("tab", { name: "词条" }));
    expect(
      within(screen.getByRole("table", { name: "词条表" })).getByText("Kubernetes"),
    ).toBeInTheDocument();
    await openSuggestions(user, /^学习建议（1）$/);
    await user.click(screen.getByRole("button", { name: "撤销：维他命 → Voltip" }));
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃提普"]);
    });
    expect(
      await within(rowOf("维他命")).findByRole("button", { name: "加入词典：维他命 → Voltip" }),
    ).toBeEnabled();
    expect(await screen.findByRole("tab", { name: "学习建议（2）" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "撤销：库伯内特斯 → Kubernetes" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual([
        "Voltip",
        "good idea",
        "fetchUser",
      ]);
    });
    expect(await screen.findByRole("tab", { name: "学习建议（3）" })).toBeInTheDocument();
  });

  it("撤销 that comes too late says why and leaves the entry", async () => {
    const user = userEvent.setup();
    const backend = learning();
    vi.spyOn(backend, "vocabularySuggestionUndo").mockResolvedValueOnce({ kind: "changed" });
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议（3）$/);
    await user.click(screen.getByRole("button", { name: "加入词典：维他命 → Voltip" }));
    await user.click(await screen.findByRole("button", { name: "撤销：维他命 → Voltip" }));
    expect(await screen.findByText("无法撤销：词条已被修改。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "撤销：维他命 → Voltip" })).toBeNull();
    expect(within(rowOf("维他命")).getByText("已加入词典")).toBeInTheDocument();
    expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃提普", "维他命"]);
  });

  it("忽略 leaves a pair out for good, and 撤销 brings it back", async () => {
    const user = userEvent.setup();
    const backend = learning();
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议（3）$/);
    await user.click(screen.getByRole("button", { name: "忽略：非 → 飞" }));
    await waitFor(() => {
      expect(backend.peek().vocabulary_ignored).toMatchObject([{ heard: "非", polished: "飞" }]);
    });
    expect(within(rowOf("非")).getByText("已忽略")).toBeInTheDocument();
    expect(await screen.findByRole("tab", { name: "学习建议（2）" })).toBeInTheDocument();
    expect(shownPairs().map((r) => r[0])).toEqual([
      "非→飞",
      "维他命→Voltip",
      "库伯内特斯→Kubernetes",
    ]);
    await user.click(screen.getByRole("button", { name: "撤销：非 → 飞" }));
    await waitFor(() => {
      expect(backend.peek().vocabulary_ignored).toEqual([]);
    });
    expect(await within(rowOf("非")).findByRole("button", { name: "忽略：非 → 飞" })).toBeEnabled();
    expect(await screen.findByRole("tab", { name: "学习建议（3）" })).toBeInTheDocument();
  });

  it("a row's details show the takes it came from and what the entry would change before it is saved", async () => {
    const user = userEvent.setup();
    const backend = learning();
    const backtest = vi.spyOn(backend, "vocabularyBacktest");
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议（3）$/);
    const details = screen.getByRole("button", { name: "详情：非 → 飞" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    await user.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    const box = screen.getByTestId("suggestion-details");
    expect(
      within(within(box).getByRole("list", { name: "识别原文中的例子" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["发到非书群", "又是非书"]);
    expect(box).toHaveTextContent("将新建词条「飞」，曾听成「非」。");
    expect(box).toHaveTextContent("「非」只有 1 个字，容易误改其他词语。");
    // The backtest the history page's card runs, over the recent takes, none left out.
    expect(
      await within(box).findByText("加入后，最近 5 条记录中有 3 条会被改动。"),
    ).toBeInTheDocument();
    expect(
      within(box).getByText("其中 1 条的最终文本仍为「非」，可能是正确用法。"),
    ).toBeInTheDocument();
    expect(backtest).toHaveBeenLastCalledWith({
      draft: {
        kind: "dictionary",
        id: null,
        entry: { term: "飞", heard_as: ["非"], enabled: true },
      },
    });
    // One row's details at a time; this one merges into the entry with that spelling.
    await user.click(screen.getByRole("button", { name: "详情：维他命 → Voltip" }));
    expect(details).toHaveAttribute("aria-expanded", "false");
    const other = screen.getByTestId("suggestion-details");
    expect(other).toHaveTextContent("将把「维他命」加入词条「Voltip」的曾听成写法（现有 1 个）。");
    expect(
      await within(other).findByText("加入后，最近 5 条记录中有 1 条会被改动。"),
    ).toBeInTheDocument();
    expect(other).not.toHaveTextContent("容易误改");
    await user.click(screen.getByRole("button", { name: "详情：维他命 → Voltip" }));
    expect(screen.queryByTestId("suggestion-details")).toBeNull();
  });

  it("a pair the dictionary or a rule refuses says why and leads to what is in the way", async () => {
    const user = userEvent.setup();
    const back: ReplacementRule = {
      id: BACK,
      name: "改回去",
      kind: "literal",
      pattern: "Kubernetes",
      replacement: "库伯内特斯",
      case_sensitive: true,
      enabled: true,
      created_at_ms: NOW,
      updated_at_ms: NOW,
      source: { kind: "manual" },
      pending: false,
    };
    const backend = learning({
      dictionary: [...dictionary(), entry(FEI, "飞", [], { enabled: false })],
      rules: [back],
    });
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议（3）$/);
    await user.click(screen.getByRole("button", { name: "加入词典：非 → 飞" }));
    expect(await screen.findByText("词条「飞」已停用，启用后才能加入。")).toBeInTheDocument();
    expect(backend.peek().dictionary.find((e) => e.id === FEI)?.heard_as).toEqual([]);
    // The row keeps its buttons: 忽略 is still a way out.
    expect(within(rowOf("非")).getByRole("button", { name: "忽略：非 → 飞" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "查看词条" }));
    expect(screen.getByRole("tab", { name: "词条" })).toHaveAttribute("aria-selected", "true");
    const table = screen.getByRole("table", { name: "词条表" });
    const selected = within(table)
      .getAllByRole("row")
      .find((r) => r.getAttribute("aria-selected") === "true");
    expect(selected).toHaveTextContent("飞");
    await openSuggestions(user, /^学习建议（3）$/);
    await user.click(screen.getByRole("button", { name: "加入词典：库伯内特斯 → Kubernetes" }));
    expect(
      await screen.findByText(
        "替换规则「改回去」会把「Kubernetes」改成「库伯内特斯」，加入词典后也不会生效。",
      ),
    ).toBeInTheDocument();
    expect(backend.peek().dictionary.map((e) => e.term)).not.toContain("Kubernetes");
    await user.click(screen.getByRole("button", { name: "打开替换规则" }));
    expect(await screen.findByTestId("page-rules")).toBeInTheDocument();
  });

  it("regression: a failed answer, a failed action or a failed backtest is reported and not swallowed", async () => {
    const user = userEvent.setup();
    const backend = learning();
    const answers = vi
      .spyOn(backend, "vocabularySuggestions")
      .mockRejectedValue(new Error("core down"));
    vi.spyOn(backend, "vocabularySuggestionAccept").mockRejectedValueOnce(new Error("not now"));
    vi.spyOn(backend, "vocabularyBacktest").mockRejectedValue(new Error("busy"));
    renderApp({ path: "/dictionary", backend });
    await openSuggestions(user, /^学习建议$/);
    expect(await screen.findByText("无法读取学习建议：core down")).toBeInTheDocument();
    // The next change asks again.
    answers.mockRestore();
    await act(async () => {
      await backend.invoke("vocabulary_suggestion_ignore", { heard: "无", polished: "关" });
    });
    await user.click(await screen.findByRole("button", { name: "加入词典：非 → 飞" }));
    expect(await screen.findByText("操作失败：not now")).toBeInTheDocument();
    expect(within(rowOf("非")).getByRole("button", { name: "加入词典：非 → 飞" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "详情：非 → 飞" }));
    expect(await screen.findByText("无法计算影响范围：busy")).toBeInTheDocument();
  });

  it("with history recording off, 学习建议 says it needs the history and leads to its setting", async () => {
    const user = userEvent.setup();
    renderApp({
      path: "/dictionary",
      backend: learning({ settings: { history: { enabled: false, keep: 2000 } } }),
    });
    await openSuggestions(user, /^学习建议$/);
    expect(screen.getByText("需要开启历史记录")).toBeInTheDocument();
    expect(screen.queryByRole("table", { name: "学习建议" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "打开隐私设置" }));
    expect(await screen.findByTestId("settings-content")).toHaveAttribute(
      "data-section",
      "privacy",
    );
  });

  it("without polished takes, 学习建议 says the clean-up is what it learns from", async () => {
    const user = userEvent.setup();
    renderApp({
      path: "/dictionary",
      backend: learning({ history: [take(SA, AT - HOUR, "发到非书群", "发到非书群", false)] }),
    });
    await openSuggestions(user, /^学习建议$/);
    expect(await screen.findByText("最近的记录里没有经过 AI 润色的听写")).toBeInTheDocument();
    expect(screen.queryByRole("table", { name: "学习建议" })).toBeNull();
  });

  it("when the dictionary already corrects everything polished, 学习建议 says there is nothing new", async () => {
    const user = userEvent.setup();
    renderApp({
      path: "/dictionary",
      backend: learning({
        dictionary: [entry(VOLTIP, "Voltip", ["沃提普", "维他命"])],
        history: [take(SC, AT - HOUR, "用维他命测了一下", "用 Voltip 测了一下。")],
      }),
    });
    await openSuggestions(user, /^学习建议$/);
    expect(await screen.findByText("暂无学习建议")).toBeInTheDocument();
    expect(
      screen.getByText("最近 1 条经过 AI 润色的记录里，没有新的写法可以加入词典。"),
    ).toBeInTheDocument();
  });

  it("the English view reads the same", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/dictionary", backend: learning(), systemLanguage: "en-US" });
    await user.click(await screen.findByRole("tab", { name: "Suggestions (3)" }));
    expect(screen.getByRole("table", { name: "Suggestions" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add to dictionary: 非 → 飞" })).toBeEnabled();
    expect(screen.getByTestId("suggestions-footer")).toHaveTextContent(
      "Last 5 entries · 4 polished",
    );
  });
});
