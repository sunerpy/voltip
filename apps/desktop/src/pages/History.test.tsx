import {
  type DictionaryEntry,
  HISTORY_LIMIT,
  type HistoryEntry,
  formatCount,
} from "@voltip/shared";
import {
  MOCK_ASR_MS,
  MOCK_MIC_READY_MS,
  MOCK_PUBLIC_KEYS,
  MOCK_REFINE_MS,
  MockBackend,
  desktopIdentity,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";
import { writeHistoryPageSize } from "./history-list/page-size";

/** Fixed pixel panel sizes and two-fixed-column grids broke the 1440 / 1920 px windows (Windows
 *  test 2026-09-24); `max-w-[…]` / `min-w-[…]` caps stay allowed (the root is `max-w-[1440px]`). */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

/** The page groups by the real clock, so the sample rows must be dated by the same clock. */
function liveClock() {
  return { now: () => Date.now() };
}

describe("History page keys and retention", () => {
  it("regression: the banner says the history goes to the phones that sync, and only on this computer once none does", async () => {
    // 2026-10-02 (docs/dictation.md §20.8): the banner said 「仅保存在本机」 while a phone with
    // Sync on received a copy of every entry.
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    const title = await screen.findByTestId("history-banner-title");
    await waitFor(() => {
      expect(title).toHaveTextContent("历史记录 · 保存在本机，并同步到已配对的手机");
    });
    await act(async () => {
      await backend.invoke("device_sync_set", { publicKey: MOCK_PUBLIC_KEYS.phone, on: false });
    });
    await waitFor(() => {
      expect(title).toHaveTextContent("历史记录 · 仅保存在本机");
    });
  });

  it("regression: the banner follows 隐私与历史 (kept count, recording off) and links to it", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    await screen.findByTestId("page-history");
    const saved = backend.peek().history_recent.length;
    await backend.invoke("settings_set_history", { enabled: true, keep: 50 });
    expect(await screen.findByText(`${saved} / 50`)).toBeInTheDocument();
    expect(screen.getByTestId("history-retention")).toHaveTextContent(
      "保留最近 50 条，超出后自动删除最早的记录。",
    );
    await backend.invoke("settings_set_history", { enabled: false, keep: 50 });
    await waitFor(() => {
      expect(screen.getByTestId("history-retention")).toHaveAttribute("data-enabled", "false");
    });
    expect(screen.getByTestId("history-retention")).toHaveTextContent(
      `历史记录已关闭 · 不再保存新的听写，已保存的 ${saved} 条在清空前会一直保留。`,
    );
    await user.click(screen.getByRole("button", { name: "保留设置" }));
    expect(
      await screen.findByRole("heading", { name: "隐私与历史", level: 2 }),
    ).toBeInTheDocument();
  });

  it("regression: a clipboard fallback shows a plain reason below the text, not the raw injector error in the header", async () => {
    const user = userEvent.setup();
    const raw = "enigo: the application does not have the permission to simulate input";
    const now = Date.now();
    const base = {
      asr_model: "whisper-large-v3-turbo",
      duration_ms: 1400,
      asr_ms: 380,
      refined: false,
      starred: false,
      mode: "whole_take",
      kind: "dictation",
    } as const;
    const rows: HistoryEntry[] = [
      {
        ...base,
        id: "coded",
        at_ms: now - 60_000,
        raw_text: "发给产品",
        text: "发给产品。",
        outcome: { kind: "clipboard", reason: raw, code: "no_permission" },
      },
      {
        ...base,
        id: "legacy",
        at_ms: now - 120_000,
        raw_text: "旧记录",
        text: "旧记录。",
        outcome: { kind: "clipboard", reason: "目标窗口没有焦点" },
      },
    ];
    const backend = new MockBackend({
      now: () => Date.now(),
      history: rows,
      identity: { ...desktopIdentity(), platform: "macos" },
    });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    // One sentence for the reason, with the macOS paste keys and the setting to open.
    const note = screen.getByTestId("history-clipboard-note");
    expect(note).toHaveAttribute("data-code", "no_permission");
    expect(note).toHaveTextContent(
      "无法粘贴到光标处：Voltip 尚未获得「辅助功能」权限。文字已复制到剪贴板，可按 ⌘V 粘贴。",
    );
    // The raw message is only under the technical details: not in the header, the readout or the row.
    expect(screen.getAllByText(raw)).toHaveLength(1);
    expect(within(note).getByTestId("history-clipboard-detail")).toHaveTextContent(raw);
    expect(within(log).queryByText(raw, { exact: false })).toBeNull();
    expect(screen.getAllByText("已复制到剪贴板").length).toBeGreaterThanOrEqual(2);
    await user.click(within(note).getByRole("button", { name: "打开辅助功能设置" }));
    expect(backend.permissionRequests).toEqual(["accessibility"]);
    // An entry written before the codes: the general sentence, its message under the details.
    await user.click(within(log).getByRole("button", { name: /旧记录/ }));
    const legacy = screen.getByTestId("history-clipboard-note");
    expect(legacy).toHaveAttribute("data-code", "other");
    expect(legacy).toHaveTextContent("无法直接粘贴，文字已复制到剪贴板，可按 ⌘V 粘贴。");
    expect(within(legacy).getByTestId("history-clipboard-detail")).toHaveTextContent(
      "目标窗口没有焦点",
    );
    expect(within(legacy).queryByRole("button", { name: "打开辅助功能设置" })).toBeNull();
  });

  it("regression: a row's copy and paste buttons act on that row without selecting it", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    const log = await screen.findByRole("list", { name: "听写记录" });
    const [newest, second] = backend.peek().history_recent;
    if (!newest || !second) throw new Error("fixture");
    const actions = within(log).getAllByTestId("result-actions");
    expect(actions).toHaveLength(backend.peek().history_recent.length);
    const secondRow = actions[1];
    if (!secondRow) throw new Error("row");
    await user.click(within(secondRow).getByRole("button", { name: "粘贴到上一个窗口" }));
    expect(backend.pastes).toEqual([second.text]);
    expect(await screen.findByText("已粘贴到上一个窗口")).toBeInTheDocument();
    await user.click(within(secondRow).getByRole("button", { name: "复制这条结果" }));
    expect(writeText).toHaveBeenCalledWith(second.text);
    // The selection stayed on the newest row.
    expect(within(log).getByRole("button", { pressed: true })).toHaveTextContent(newest.text);
  });

  it("regression: the footer's Ctrl F, Ctrl C and Del are real; they stand down in a field, over a selection and under the settings dialog", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    await screen.findByTestId("page-history");
    const newest = backend.peek().history_recent[0];
    if (!newest) throw new Error("fixture");
    // Ctrl F focuses the search field; typing there never copies or deletes.
    await user.keyboard("{Control>}f{/Control}");
    const search = screen.getByRole("textbox", { name: "搜索历史" });
    expect(search).toHaveFocus();
    await user.keyboard("{Control>}c{/Control}{Delete}");
    expect(writeText).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
    search.blur();
    // Ctrl C copies the entry in the detail, like 复制.
    await user.keyboard("{Control>}c{/Control}");
    expect(writeText).toHaveBeenCalledWith(newest.text);
    expect(
      await screen.findByText(`已复制到剪贴板 · ${Array.from(newest.text).length} 字`),
    ).toBeInTheDocument();
    // …unless text is selected: then the copy is the selection's (the browser's own).
    writeText.mockClear();
    // selectAllChildren replaces the selection; addRange is ignored while one exists (jsdom 30
    // keeps the caret the search field left behind, as browsers do).
    window.getSelection()?.selectAllChildren(screen.getByTestId("entry-text"));
    await user.keyboard("{Control>}c{/Control}");
    expect(writeText).not.toHaveBeenCalled();
    window.getSelection()?.removeAllRanges();
    // Del asks first, then deletes through the core.
    await user.keyboard("{Delete}");
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().history_recent.some((e) => e.id === newest.id)).toBe(false);
    });
  });

  it("regression: page keys do nothing while the settings dialog floats over the page", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    await screen.findByTestId("page-history");
    const before = backend.peek().history_recent.length;
    await user.keyboard("{Control>},{/Control}");
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    (document.activeElement as HTMLElement | null)?.blur();
    await user.keyboard("{Delete}{Control>}c{/Control}");
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(writeText).not.toHaveBeenCalled();
    expect(backend.peek().history_recent).toHaveLength(before);
  });
});

describe("History page", () => {
  it("regression: history is fluid (no fixed-width panels)", async () => {
    renderApp({ path: "/history" });
    const page = await screen.findByTestId("page-history");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6");
    expect(fixedSizeOffenders(page)).toEqual([]);
    const split = page.querySelector(".grid");
    expect(split?.className).toContain("grid-cols-1");
    expect(split?.className).toMatch(/lg:grid-cols-\[minmax\(280px,2fr\)_minmax\(0,3fr\)\]/);
  });

  it("regression: the log grows with the window instead of stopping at 360 px, and a row's model, time and result read whole on hover", async () => {
    // User feedback 2026-09-29 (plan 1.6): at 1440 px the 360 px log cut 「Qwen3-ASR-1.7B · 622 ms ·
    // 已插入 · 粘贴」 to 「Qwen3-ASR-1.7B · 622 ms · 已…」 with the detail pane half empty.
    renderApp({ path: "/history" });
    const page = await screen.findByTestId("page-history");
    expect(page.querySelector(".grid")?.className).not.toContain("360px");
    const metas = screen.getAllByTestId("history-row-meta");
    expect(metas.length).toBeGreaterThan(0);
    for (const meta of metas) {
      expect(meta.textContent).toMatch(/ · /);
      expect(meta).toHaveAttribute("title", meta.textContent);
    }
  });

  it("renders the factual retention banner, the day-grouped log from state.history_recent and the newest entry's detail", async () => {
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    // 2026-10-02 (docs/dictation.md §20.8): the sample phone syncs, so the history is on it too.
    expect(
      await screen.findByText("历史记录 · 保存在本机，并同步到已配对的手机"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("history-retention")).toHaveTextContent(
      `保留最近 ${formatCount(HISTORY_LIMIT)} 条，超出后自动删除最早的记录。`,
    );
    const history = backend.peek().history_recent;
    expect(
      await screen.findByText(`${history.length} / ${formatCount(HISTORY_LIMIT)}`),
    ).toBeInTheDocument();
    const log = screen.getByRole("list", { name: "听写记录" });
    expect(within(log).getByText(/^今天 · /)).toBeInTheDocument();
    expect(within(log).getByText(/^昨天 · /)).toBeInTheDocument();
    expect(within(log).getAllByRole("button", { pressed: true })).toHaveLength(1);
    const newest = history[0];
    if (!newest) throw new Error("fixture");
    expect(screen.getByTestId("entry-text")).toHaveTextContent(newest.text);
    expect(screen.getByRole("img", { name: "耗时拆解" })).toBeInTheDocument();
    expect(screen.getByText(/总计 622 ms/)).toBeInTheDocument();
    expect(screen.getByText("Qwen/Qwen3-ASR-1.7B")).toBeInTheDocument();
    expect(screen.getByText("qwen/qwen3.8-27b")).toBeInTheDocument();
    expect(screen.getAllByText("已插入 · 粘贴").length).toBeGreaterThan(0);
    // No sample chips, no phase talk, no controls that pretend.
    const page = screen.getByTestId("page-history");
    expect(page.textContent).not.toMatch(/第二阶段|示例|sqlite/);
    expect(screen.queryByRole("button", { name: "重新插入" })).toBeNull();
    // 加入词典 is real (docs/dictation.md §16): it opens the dictionary dialog.
    expect(screen.getByRole("button", { name: "加入词典" })).toBeEnabled();
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
  });

  it("docs/dictation.md section 3.6: an entry whose clean-up was turned down says why it is not polished", async () => {
    const user = userEvent.setup();
    const now = Date.now();
    const base = new MockBackend({ now: () => now }).peek().history_recent[0];
    if (!base) throw new Error("fixture");
    const busy: HistoryEntry = {
      ...base,
      id: "00000000-0000-4000-8000-0000000000b1",
      at_ms: now - 1_000,
      raw_text: "把重试次数改成三次",
      text: "把重试次数改成三次",
      refined: false,
      refine_model: undefined,
      refine_ms: undefined,
      refine_failure: "rate_limited",
      preset: undefined,
    };
    renderApp({ path: "/history", mock: { now: () => now, history: [busy] } });
    await user.click(await screen.findByText(/把重试次数改成三次/, { selector: "li *" }));
    expect(await screen.findByText("未润色 · 服务繁忙")).toBeInTheDocument();
  });

  it("regression: section 3.6 a clean-up the service turned down says so in the timings and keeps the service's words", async () => {
    // 2026-10-24, Google AI Studio: the model answered 503 "high demand"; the timing row said
    // 「润色 — · 未启用」 and nothing showed why.
    const user = userEvent.setup();
    const now = Date.now();
    const base = new MockBackend({ now: () => now }).peek().history_recent[0];
    if (!base) throw new Error("fixture");
    const reason =
      "服务繁忙：refine service overloaded (HTTP 503): This model is currently experiencing high demand.";
    const busy: HistoryEntry = {
      ...base,
      id: "00000000-0000-4000-8000-0000000000b2",
      at_ms: now - 1_000,
      raw_text: "现在可以吗",
      text: "现在可以吗",
      refined: false,
      refine_model: undefined,
      refine_ms: undefined,
      refine_failure: "rate_limited",
      refine_error: reason,
      preset: undefined,
    };
    const unconfigured: HistoryEntry = {
      ...busy,
      id: "00000000-0000-4000-8000-0000000000b3",
      at_ms: now - 2_000,
      raw_text: "没有配置润色",
      text: "没有配置润色",
      refine_failure: "unconfigured",
      refine_error: undefined,
    };
    renderApp({ path: "/history", mock: { now: () => now, history: [busy, unconfigured] } });
    await user.click(await screen.findByText(/现在可以吗/, { selector: "li *" }));
    expect(await screen.findByText("润色 — · 服务繁忙")).toBeInTheDocument();
    expect(screen.queryByText("润色 — · 未启用")).toBeNull();
    const note = screen.getByTestId("history-refine-note");
    expect(note).toHaveTextContent(
      "润色服务繁忙，本次使用了识别原文。可以稍后再试，或在「AI 模型」页换一个模型。",
    );
    // The client's own words in the interface's language, the service's as it sent them (§3.12).
    expect(within(note).getByTestId("history-refine-detail")).toHaveTextContent(
      "原因：服务繁忙：This model is currently experiencing high demand.",
    );
    expect(within(note).getByTestId("history-refine-detail")).not.toHaveTextContent(
      "refine service",
    );

    await user.click(screen.getByText(/没有配置润色/, { selector: "li *" }));
    expect(await screen.findByText("润色 — · 未配置 AI 服务")).toBeInTheDocument();
    expect(screen.queryByTestId("history-refine-note")).toBeNull();
  });

  it("regression: section 20.6 a phone's text and a phone's take are badged with the phone, and a text shows no model or timings", async () => {
    const now = Date.now();
    const base = new MockBackend({ now: () => now }).peek().history_recent[0];
    if (!base) throw new Error("fixture");
    const text: HistoryEntry = {
      ...base,
      id: "00000000-0000-4000-8000-0000000000a1",
      at_ms: now - 1_000,
      raw_text: "会议改到三点",
      text: "会议改到三点",
      refined: false,
      asr_model: "",
      refine_model: undefined,
      asr_ms: 0,
      refine_ms: undefined,
      duration_ms: 0,
      vocabulary: undefined,
      app: undefined,
      scene: undefined,
      origin: { device: "Pixel 8", kind: "typed" },
    };
    const take: HistoryEntry = {
      ...base,
      id: "00000000-0000-4000-8000-0000000000a2",
      at_ms: now - 2_000,
      origin: { device: "Pixel 8", kind: "take" },
    };
    renderApp({ path: "/history", mock: { now: () => now, history: [text, take] } });
    const log = await screen.findByRole("list", { name: "听写记录" });
    const origins = within(log).getAllByTestId("history-origin");
    expect(origins.map((o) => [o.dataset.origin, o.textContent])).toEqual([
      ["typed", "手机输入 · Pixel 8"],
      ["take", "手机 · Pixel 8"],
    ]);
    // The newest (the text) is selected: its detail names the phone and draws no timing bar.
    expect(screen.getByTestId("history-detail-origin")).toHaveTextContent("手机输入 · Pixel 8");
    expect(screen.queryByRole("img", { name: "耗时拆解" })).toBeNull();
    expect(within(log).getAllByRole("button")[0]).not.toHaveTextContent(/ms ·/);
  });

  it("filters by search and range, switches text views with diff, and copies the real text", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    renderApp({ path: "/history", mock: liveClock() });
    await screen.findByTestId("entry-text");
    await user.type(screen.getByLabelText("搜索历史"), "latency");
    expect(screen.getByText(/attach the latency report/)).toBeInTheDocument();
    await user.type(screen.getByLabelText("搜索历史"), "zzz");
    // The search goes to the core once the typing pauses (docs/dictation.md §4.4).
    expect(await screen.findByText("没有结果匹配「latencyzzz」")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "清除搜索" }));
    await user.click(screen.getByRole("radio", { name: "今天" }));
    await waitFor(() => {
      expect(
        within(screen.getByRole("list", { name: "听写记录" })).getAllByRole("listitem"),
      ).toHaveLength(1 + 2);
    });
    await user.click(screen.getByRole("radio", { name: "未插入" }));
    // The clipboard row says so in two words; its reason is only in the entry's note.
    const unsent = screen.getByRole("list", { name: "听写记录" });
    expect(await within(unsent).findByText(/· 已复制到剪贴板$/)).toBeInTheDocument();
    expect(within(unsent).queryByText(/目标窗口没有焦点/)).toBeNull();
    expect(screen.getByText(/失败 · 目标窗口已丢失/)).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "已收藏" }));
    await user.click(await screen.findByText(/返回值类型改成/));
    expect(screen.getByRole("radio", { name: "润色后" })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "对比" }));
    expect(
      within(screen.getByTestId("entry-text")).getByText(/option string/, {
        selector: ".line-through",
      }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("entry-text")).getByText(/Option<String>/, {
        selector: ".bg-diff-add",
      }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "原文" }));
    expect(screen.getByTestId("entry-text")).toHaveTextContent(
      "这个函数的返回值类型改成 option string",
    );
    // docs/dictation.md §21: a cleaned-up row names the preset it ran with.
    expect(screen.getByTestId("history-detail-preset")).toHaveTextContent("校对");
    await user.click(screen.getByRole("button", { name: "复制" }));
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining("option string"));
    expect(await screen.findByText(/已复制到剪贴板 · \d+ 字/)).toBeInTheDocument();
    // An unrefined entry offers 插入文本 instead of 润色后 and its timing bar says so.
    await user.click(screen.getByRole("radio", { name: "全部" }));
    await user.click(screen.getByText(/attach the latency report/));
    expect(screen.getByRole("radio", { name: "插入文本" })).toBeInTheDocument();
    expect(screen.getByText(/润色 — · 未启用/)).toBeInTheDocument();
    expect(screen.getByText("未润色")).toBeInTheDocument();
    expect(screen.queryByTestId("history-detail-preset")).toBeNull();
  });

  it("regression: star, delete and clear are real core commands; deleting selects the neighbour and there is no fake success toast", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/history", mock: liveClock() });
    await screen.findByTestId("entry-text");
    const first = backend.peek().history_recent[0];
    if (!first) throw new Error("fixture");
    // Star from the detail pane.
    const detailStar = screen
      .getAllByRole("button", { name: first.starred ? "取消收藏" : "收藏" })
      .at(-1);
    if (!detailStar) throw new Error("no star");
    await user.click(detailStar);
    await waitFor(() => {
      expect(backend.peek().history_recent[0]?.starred).toBe(!first.starred);
    });
    // Star from the row, by keyboard.
    const rowStar = within(screen.getByRole("list", { name: "听写记录" })).getAllByRole("button", {
      name: /收藏/,
    })[0];
    if (!rowStar) throw new Error("no row star");
    rowStar.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(backend.peek().history_recent[0]?.starred).toBe(first.starred);
    });
    // Delete asks first, then the core drops the row and the next one is selected.
    await user.click(screen.getByRole("button", { name: "删除" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("删除这条记录？");
    expect(dialog).toHaveTextContent("此操作无法撤销");
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().history_recent.find((e) => e.id === first.id)).toBeUndefined();
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText(/已删除/)).toBeNull();
    const second = backend.peek().history_recent[0];
    if (!second) throw new Error("fixture");
    expect(screen.getByTestId("entry-text")).toHaveTextContent(second.text);
    expect(
      await screen.findByText(`${backend.peek().history_total} / ${formatCount(HISTORY_LIMIT)}`),
    ).toBeInTheDocument();
    // Clear everything.
    await user.click(screen.getByRole("button", { name: "全部清空" }));
    const clear = screen.getByRole("dialog");
    expect(clear).toHaveTextContent("全部历史记录将被清空");
    expect(clear.textContent).not.toMatch(/示例|sqlite/);
    await user.click(within(clear).getByRole("button", { name: "清空" }));
    await waitFor(() => {
      expect(backend.peek().history_recent).toEqual([]);
    });
    expect(screen.getByText("暂无记录。")).toBeInTheDocument();
    expect(screen.getByText(/按住 Ctrl Alt Space 说一句/)).toBeInTheDocument();
    expect(screen.getByText("从左侧选择一条结果")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "全部清空" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /历史记录/ }).textContent).not.toMatch(/\d/);
  });

  it("opens with a home tile filter or a selected entry id, and says when a range is empty", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => Date.now() });
    const target = backend.peek().history_recent.at(-1);
    if (!target) throw new Error("fixture");
    renderApp({ path: `/history?filter=${target.id}`, backend });
    expect(await screen.findByTestId("entry-text")).toHaveTextContent(target.text);
    renderApp({
      path: "/history?filter=starred",
      backend: new MockBackend({ now: () => Date.now() }),
    });
    const starred = (await screen.findAllByRole("radio", { name: "已收藏" })).at(-1);
    expect(starred).toHaveAttribute("aria-checked", "true");
    // A list with rows but none starred explains itself.
    const unstarred = new MockBackend({
      now: () => Date.now(),
      history: [
        {
          id: "only",
          at_ms: Date.now(),
          raw_text: "r",
          text: "唯一一条",
          refined: false,
          asr_model: "m",
          duration_ms: 1000,
          asr_ms: 0,
          outcome: { kind: "inserted", via: "clipboard" },
          starred: false,
          mode: "whole_take",
          kind: "dictation",
        },
      ],
    });
    renderApp({ path: "/history?filter=starred", backend: unstarred });
    expect((await screen.findAllByText("已收藏暂无结果。")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("在任意一条上点 ★ 收藏。").length).toBeGreaterThan(0);
    await user.click(screen.getAllByRole("radio", { name: "全部" }).at(-1) as HTMLElement);
    expect(screen.getAllByText("未记录耗时").length).toBeGreaterThan(0);
    expect(screen.getAllByText("已插入 · 剪贴板").length).toBeGreaterThan(0);
  });

  it("regression: streaming rows carry a mode badge (边说边识别 / 边说边打字) while whole takes stay quiet, and the detail shows the streaming fallback reason (docs/dictation.md §12)", async () => {
    const user = userEvent.setup();
    const now = Date.now();
    const base: Omit<HistoryEntry, "id" | "at_ms" | "text" | "raw_text" | "mode"> = {
      refined: false,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 3200,
      asr_ms: 45,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      kind: "dictation",
    };
    const rows: HistoryEntry[] = [
      {
        ...base,
        id: "live",
        at_ms: now - 60_000,
        raw_text: "逐句打进去的一段",
        text: "逐句打进去的一段",
        mode: "live_inject",
        segments: [{ text: "逐句打进去的一段", start_ms: 0, end_ms: 3200 }],
        live_error: "live tap overrun: the decoder fell behind the microphone",
      },
      {
        ...base,
        id: "stream",
        at_ms: now - 120_000,
        raw_text: "流式定稿的一段",
        text: "流式定稿的一段",
        mode: "streaming_final",
      },
      {
        ...base,
        id: "whole",
        at_ms: now - 180_000,
        raw_text: "整段识别的一段",
        text: "整段识别的一段",
        mode: "whole_take",
      },
    ];
    renderApp({
      path: "/history",
      backend: new MockBackend({ now: () => Date.now(), history: rows }),
    });
    const log = await screen.findByRole("list", { name: "听写记录" });
    const badges = within(log).getAllByTestId("history-mode");
    expect(badges.map((b) => b.getAttribute("data-mode"))).toEqual([
      "live_inject",
      "streaming_final",
    ]);
    expect(badges.map((b) => b.textContent)).toEqual(["边说边输入", "边说边识别"]);
    // The newest row (live_inject) is the detail: badge in the header, the reason under the text.
    expect(screen.getByTestId("history-detail-mode")).toHaveTextContent("边说边输入");
    expect(screen.getByTestId("history-live-error")).toHaveTextContent(
      "实时识别中断，已改为整段识别：live tap overrun: the decoder fell behind the microphone",
    );
    // A streaming take without a fallback: badge, no reason.
    await user.click(within(log).getByRole("button", { name: /流式定稿的一段/ }));
    expect(screen.getByTestId("history-detail-mode")).toHaveTextContent("边说边识别");
    expect(screen.queryByTestId("history-live-error")).toBeNull();
    // The whole take: no badge anywhere.
    await user.click(within(log).getByRole("button", { name: /整段识别的一段/ }));
    expect(screen.queryByTestId("history-detail-mode")).toBeNull();
    expect(screen.queryByTestId("history-live-error")).toBeNull();
    expect(screen.queryByText("整段输出")).toBeNull();
  });

  it("regression: a take with a context shows its app and scene in the row and the detail, search finds them, and a take without one shows neither (docs/dictation.md section 18.6)", async () => {
    const user = userEvent.setup();
    const now = Date.now();
    const base: Omit<HistoryEntry, "id" | "at_ms" | "text" | "raw_text"> = {
      refined: false,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 1200,
      asr_ms: 45,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
    };
    const rows: HistoryEntry[] = [
      {
        ...base,
        id: "chat",
        at_ms: now - 60_000,
        raw_text: "今晚开会",
        text: "今晚开会",
        app: { id: "slack", name: "Slack" },
        scene: { id: "00000000-0000-4000-a000-000000000001", name: "聊天" },
      },
      {
        ...base,
        id: "code",
        at_ms: now - 120_000,
        raw_text: "加一个重试",
        text: "加一个重试",
        app: { id: "code", name: "Code" },
      },
      { ...base, id: "none", at_ms: now - 180_000, raw_text: "没有上下文", text: "没有上下文" },
    ];
    renderApp({
      path: "/history",
      backend: new MockBackend({ now: () => Date.now(), history: rows }),
    });
    const log = await screen.findByRole("list", { name: "听写记录" });
    const contexts = within(log).getAllByTestId("history-context");
    expect(contexts.map((c) => c.textContent)).toEqual(["Slack聊天", "Code"]);
    expect(contexts[0]).toHaveAttribute("title", "应用与场景");
    // The newest row is the detail: app (name and id) and scene.
    expect(screen.getByTestId("history-detail-app")).toHaveTextContent("Slack · slack");
    expect(screen.getByTestId("history-detail-scene")).toHaveTextContent("聊天");
    // An app without a scene says so.
    await user.click(within(log).getByRole("button", { name: /加一个重试/ }));
    expect(screen.getByTestId("history-detail-app")).toHaveTextContent("Code · code");
    expect(screen.queryByTestId("history-detail-scene")).toBeNull();
    expect(screen.getByText("未匹配场景")).toBeInTheDocument();
    // No context at all: neither line.
    await user.click(within(log).getByRole("button", { name: /没有上下文/ }));
    expect(screen.queryByTestId("history-detail-app")).toBeNull();
    expect(screen.queryByText("未匹配场景")).toBeNull();
    // Search matches the app and the scene.
    await user.type(screen.getByRole("textbox", { name: "搜索历史" }), "聊天");
    await waitFor(() => {
      expect(within(log).queryByRole("button", { name: /加一个重试/ })).toBeNull();
    });
    expect(within(log).getByRole("button", { name: /今晚开会/ })).toBeInTheDocument();
  });

  it("regression: a voice edit is badged and reads instruction then rewrite in the row and its detail shows the instruction and the rewrite and the original selection behind a disclosure and copies the rewrite and search finds it by instruction and selection (section 19)", async () => {
    const user = userEvent.setup();
    const now = Date.now();
    const writes: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: (text: string) => {
          writes.push(text);
          return Promise.resolve();
        },
      },
    });
    const base: Omit<HistoryEntry, "id" | "at_ms" | "text" | "raw_text" | "kind"> = {
      refined: true,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      refine_model: "llama-3.3-70b-versatile",
      duration_ms: 1400,
      asr_ms: 380,
      refine_ms: 900,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
    };
    const rows: HistoryEntry[] = [
      {
        ...base,
        id: "edit",
        at_ms: now - 60_000,
        raw_text: "改的更正式",
        text: "各位同事：会议改至周四上午十点。",
        kind: "edit",
        edit: { instruction: "改得更正式", selection: "大家好，会议改到周四十点哈" },
        app: { id: "slack", name: "Slack" },
      },
      {
        ...base,
        id: "dictation",
        at_ms: now - 120_000,
        raw_text: "今晚开会",
        text: "今晚开会。",
        kind: "dictation",
      },
    ];
    renderApp({
      path: "/history",
      backend: new MockBackend({ now: () => Date.now(), history: rows }),
    });
    const log = await screen.findByRole("list", { name: "听写记录" });
    // The row: badge, instruction → rewrite; the dictation row has neither.
    expect(
      within(log)
        .getAllByTestId("history-kind")
        .map((b) => b.textContent),
    ).toEqual(["编辑"]);
    const editRow = within(log).getByTestId("history-edit-row");
    expect(editRow).toHaveTextContent("改得更正式→各位同事：会议改至周四上午十点。");
    expect(within(log).getAllByTestId("history-edit-row")).toHaveLength(1);
    // The newest row is the detail: instruction, rewrite, the selection behind a disclosure.
    expect(screen.getByTestId("history-detail-kind")).toHaveTextContent("编辑");
    expect(screen.getByTestId("history-edit-instruction")).toHaveTextContent("改得更正式");
    expect(screen.getByTestId("entry-text")).toHaveTextContent("各位同事：会议改至周四上午十点。");
    const disclosure = screen.getByTestId("history-edit-selection");
    expect(disclosure).not.toHaveAttribute("open");
    expect(within(disclosure).getByText("原选中文字 · 13 字")).toBeInTheDocument();
    await user.click(within(disclosure).getByText("原选中文字 · 13 字"));
    expect(disclosure).toHaveAttribute("open");
    expect(within(disclosure).getByText("大家好，会议改到周四十点哈")).toBeVisible();
    // No raw / polished / diff switch for an edit; the app context still shows.
    expect(screen.queryByRole("radiogroup", { name: "文本视图" })).toBeNull();
    expect(screen.getByTestId("history-detail-app")).toHaveTextContent("Slack · slack");
    // Copy takes the rewrite.
    await user.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => {
      expect(writes).toEqual(["各位同事：会议改至周四上午十点。"]);
    });
    // A dictation row keeps the view switch.
    await user.click(within(log).getByRole("button", { name: /今晚开会/ }));
    expect(screen.queryByTestId("history-edit")).toBeNull();
    expect(screen.queryByTestId("history-detail-kind")).toBeNull();
    expect(screen.getByRole("radiogroup", { name: "文本视图" })).toBeInTheDocument();
    // Search finds an edit by its instruction and by its original selection.
    const search = screen.getByRole("textbox", { name: "搜索历史" });
    await user.type(search, "周四十点哈");
    await waitFor(() => {
      expect(within(log).queryByRole("button", { name: /今晚开会/ })).toBeNull();
    });
    expect(within(log).getByTestId("history-edit-row")).toBeInTheDocument();
    await user.clear(search);
    await user.type(search, "改得更正式");
    await waitFor(() => {
      expect(within(log).queryByRole("button", { name: /今晚开会/ })).toBeNull();
    });
    expect(within(log).getByTestId("history-edit-row")).toBeInTheDocument();
  });

  it("regression: the detail names the corrections and rules that fired and add to dictionary sends dictionary_add with the row id", async () => {
    const user = userEvent.setup();
    const now = Date.now();
    const voltip: DictionaryEntry = {
      id: "00000000-0000-4000-8000-000000000001",
      term: "Voltip",
      heard_as: ["沃提普"],
      enabled: true,
      source: { kind: "manual" },
      created_at_ms: now,
      updated_at_ms: now,
    };
    const row: HistoryEntry = {
      id: "00000000-0000-4000-8000-0000000000a1",
      at_ms: now - 1000,
      raw_text: "用沃提普写一个谷歌IDR",
      text: "用Voltip写一个谷歌IDR",
      refined: false,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 1000,
      asr_ms: 300,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      vocabulary: {
        corrections: [
          { id: voltip.id, count: 2 },
          { id: "00000000-0000-4000-8000-00000000dead", count: 1 },
        ],
        rules: [{ id: "00000000-0000-4000-9000-00000000dead", count: 1 }],
      },
    };
    const plain: HistoryEntry = {
      ...row,
      id: "00000000-0000-4000-8000-0000000000a2",
      at_ms: now - 5000,
    };
    delete plain.vocabulary;
    const backend = new MockBackend({
      now: () => now,
      history: [row, plain],
      dictionary: [voltip],
    });
    renderApp({ path: "/history", backend });
    const block = await screen.findByTestId("history-vocabulary");
    expect(block).toHaveTextContent("词典纠正Voltip ×2、已删除的词条 ×1");
    expect(block).toHaveTextContent("替换规则已删除的规则 ×1");
    // A fragment selected inside the entry pre-fills the misheard form.
    const selection = vi.spyOn(window, "getSelection").mockReturnValue({
      toString: () => " 谷歌IDR ",
    } as unknown as Selection);
    await user.click(screen.getByRole("button", { name: "加入词典" }));
    selection.mockRestore();
    // docs/dictation.md §16.9: the button opens the quick-fix card, whose save is 「加入词典」.
    const dialog = screen.getByRole("dialog", { name: "加入词典" });
    expect(within(dialog).getByLabelText("曾听成")).toHaveValue("谷歌IDR");
    expect(within(dialog).getByRole("button", { name: "加入词典" })).toBeDisabled();
    // The same spelling twice is refused before anything is sent (the core refuses it too).
    const term = within(dialog).getByLabelText("正确写法");
    await user.type(term, "谷歌IDR");
    expect(within(dialog).getByText("曾听成写法与正确写法相同。")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "加入词典" })).toBeDisabled();
    // The core refuses a draft that is wrong on its own: the card stays with its words.
    await user.clear(term);
    await user.type(term, "good idea");
    const refuse = vi
      .spyOn(backend, "invoke")
      .mockRejectedValueOnce(new Error("dictionary: 误识别写法不能包含换行或控制字符"));
    await user.click(within(dialog).getByRole("button", { name: "加入词典" }));
    expect(await within(dialog).findByText("误识别写法不能包含换行或控制字符")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("正确写法")).toHaveValue("good idea");
    refuse.mockRestore();
    await user.type(term, "{Enter}");
    await waitFor(() => {
      expect(backend.peek().dictionary.at(-1)).toMatchObject({
        term: "good idea",
        heard_as: ["谷歌IDR"],
        source: { kind: "history", history_id: row.id },
      });
    });
    expect(screen.getByText("已加入词典 · good idea")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "加入词典" })).toBeNull();
    // A selection outside the entry is ignored; a row where nothing fired has no block.
    const other = vi.spyOn(window, "getSelection").mockReturnValue({
      toString: () => "somewhere else",
    } as unknown as Selection);
    const rows = within(screen.getByRole("list", { name: "听写记录" })).getAllByRole("button", {
      pressed: false,
    });
    await user.click(rows[0] ?? document.body);
    expect(screen.queryByTestId("history-vocabulary")).toBeNull();
    await user.click(screen.getByRole("button", { name: "加入词典" }));
    other.mockRestore();
    const second = screen.getByRole("dialog", { name: "加入词典" });
    expect(within(second).getByLabelText("曾听成")).toHaveValue("");
    await user.click(within(second).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog", { name: "加入词典" })).toBeNull();
  });
});

describe("History page with a long history (docs/dictation.md section 4.4)", () => {
  const T = Date.now();
  /** `n` dictations a minute apart, the newest first; with `starredEvery`, every such one starred. */
  const rows = (n: number, starredEvery = 0): HistoryEntry[] =>
    Array.from({ length: n }, (_, i) => ({
      id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
      at_ms: T - i * 60_000,
      raw_text: `第${i}句`,
      text: `第${i}句。`,
      refined: false,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      duration_ms: 1000,
      asr_ms: 100,
      outcome: { kind: "inserted", via: "paste" },
      starred: starredEvery > 0 && i % starredEvery === 0,
      mode: "whole_take",
      kind: "dictation",
    }));
  /** The ids of rows `from` to `to` (exclusive). */
  const ids = (from: number, to: number) =>
    rows(to)
      .slice(from)
      .map((e) => e.id);
  /** The entry ids of the log's rows, top to bottom. */
  const rowIds = (log: HTMLElement) =>
    within(log)
      .queryAllByTestId("history-row")
      .map((row) => row.dataset.entryId);
  const pager = () => screen.getByRole("navigation", { name: "分页" });
  const pageButton = (name: string) => within(pager()).getByRole("button", { name });
  const pageCount = () => screen.getByTestId("history-page-count");
  /** 50 entries a page, as chosen before (the page size this window remembers; the default is
   *  10): for the tests of how paging behaves rather than of the default. */
  const fiftyAPage = () => {
    writeHistoryPageSize(50);
  };

  it("regression: shows the history a page at a time, 10 entries by default, with first, previous, numbered, next and last pages and where the page is (user requests 2026-10-13, 2026-10-26)", async () => {
    // Until 0.1.4 the list loaded 100 more entries each time its end came into view; until 0.1.9
    // a page held 50.
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(50) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(0, 10));
    });
    const count = pageCount();
    expect(count).toHaveTextContent("第 1 / 5 页 · 共 50 条");
    expect(
      within(pager())
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual([
      "第一页",
      "上一页",
      "第 1 页",
      "第 2 页",
      "第 3 页",
      "第 4 页",
      "第 5 页",
      "下一页",
      "最后一页",
    ]);
    expect(pageButton("第 1 页")).toHaveAttribute("aria-current", "page");
    expect(pageButton("上一页")).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByRole("button", { name: "加载更多" })).toBeNull();
    expect(screen.queryByTestId("history-loaded")).toBeNull();
    for (const [name, from, page] of [
      ["下一页", 10, 2],
      ["最后一页", 40, 5],
      ["第 3 页", 20, 3],
      ["上一页", 10, 2],
      ["第一页", 0, 1],
    ] as const) {
      await user.click(pageButton(name));
      await waitFor(() => {
        expect(rowIds(log)).toEqual(ids(from, from + 10));
      });
      expect(count).toHaveTextContent(`第 ${page} / 5 页 · 共 50 条`);
      expect(pageButton(`第 ${page} 页`)).toHaveAttribute("aria-current", "page");
    }
    // The keyboard: the pages are buttons in the tab order, Enter turns them, the focus stays.
    pageButton("下一页").focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(10, 20));
    });
    expect(pageButton("下一页")).toHaveFocus();
    backend.destroy();
  });

  it("the page size sits beside the pages: another size starts again at page 1 and is remembered", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(250) });
    const view = renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(10);
    });
    const size = screen.getByRole("combobox", { name: "每页显示条数" });
    expect(size).toHaveValue("10");
    expect(
      within(size)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["10 条", "20 条", "50 条", "100 条", "200 条"]);
    await user.click(pageButton("第 3 页"));
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(20, 30));
    });
    await user.selectOptions(size, "100");
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(0, 100));
    });
    expect(pageCount()).toHaveTextContent("第 1 / 3 页 · 共 250 条");
    await user.selectOptions(size, "20");
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(0, 20));
    });
    expect(pageCount()).toHaveTextContent("第 1 / 13 页 · 共 250 条");
    // Thirteen pages: the first five, a gap and the last.
    expect(pager()).toHaveTextContent("12345…13");
    // The next visit to the page keeps the choice.
    view.unmount();
    renderApp({ path: "/history", backend });
    const again = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(again)).toEqual(ids(0, 20));
    });
    expect(screen.getByRole("combobox", { name: "每页显示条数" })).toHaveValue("20");
    backend.destroy();
  });

  it("regression: a search goes back to page 1 and counts what it found, the filters narrow it, and nothing found has its own empty state (user request 2026-10-13)", async () => {
    fiftyAPage();
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(250, 10) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(50);
    });
    await user.click(pageButton("第 3 页"));
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(100, 150));
    });
    const box = screen.getByRole("textbox", { name: "搜索历史" });
    await user.type(box, "第1");
    // 第1句, 第10–19句 and 第100–199句, from the first page of what was found.
    await waitFor(() => {
      expect(pageCount()).toHaveTextContent("第 1 / 3 页 · 找到 111 条");
    });
    expect(within(log).getAllByTestId("history-row")[0]).toHaveTextContent("第1句。");
    expect(pageButton("第 1 页")).toHaveAttribute("aria-current", "page");
    // 已收藏 with the same search: 第10句 and 第100–190句 by tens, one page, no page buttons.
    await user.click(screen.getByRole("radio", { name: "已收藏" }));
    await waitFor(() => {
      expect(pageCount()).toHaveTextContent("第 1 / 1 页 · 找到 11 条");
    });
    expect(screen.queryByRole("navigation", { name: "分页" })).toBeNull();
    // Nothing found among the starred: it says where it looked and offers both ways out.
    await user.type(box, "zzz");
    expect(await screen.findByText("「已收藏」中没有结果匹配「第1zzz」")).toBeInTheDocument();
    expect(screen.getByText("可清除搜索，或在全部记录中查找。")).toBeInTheDocument();
    expect(screen.queryByTestId("history-pager")).toBeNull();
    expect(rowIds(log)).toEqual([]);
    await user.click(screen.getByRole("button", { name: "在全部记录中查找" }));
    expect(screen.getByRole("radio", { name: "全部" })).toHaveAttribute("aria-checked", "true");
    expect(await screen.findByText("没有结果匹配「第1zzz」")).toBeInTheDocument();
    expect(screen.getByText("已记录的 250 条都不匹配。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "清除搜索" }));
    await waitFor(() => {
      expect(pageCount()).toHaveTextContent("第 1 / 5 页 · 共 250 条");
    });
    backend.destroy();
  });

  it("regression: a new dictation keeps the page the user is on, and page 1 starts with it", async () => {
    fiftyAPage();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const backend = new MockBackend({ history: rows(120) });
      renderApp({ path: "/history", backend });
      const log = await screen.findByRole("list", { name: "听写记录" });
      await waitFor(() => {
        expect(rowIds(log)).toHaveLength(50);
      });
      await user.click(pageButton("第 2 页"));
      await waitFor(() => {
        expect(rowIds(log)).toEqual(ids(50, 100));
      });
      await act(async () => {
        await backend.invoke("dictation_start");
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + 1000);
        await backend.invoke("dictation_stop");
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS + 500);
      });
      // Still page 2, one entry further down the list.
      await waitFor(() => {
        expect(pageCount()).toHaveTextContent("第 2 / 3 页 · 共 121 条");
      });
      expect(rowIds(log)).toEqual(ids(49, 99));
      expect(pageButton("第 2 页")).toHaveAttribute("aria-current", "page");
      const newest = backend.peek().history_recent[0];
      if (!newest) throw new Error("take");
      await user.click(pageButton("第 1 页"));
      await waitFor(() => {
        expect(rowIds(log)[0]).toBe(newest.id);
      });
      backend.destroy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: deleting the only entry on the last page moves to the page before it", async () => {
    fiftyAPage();
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(101) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(50);
    });
    await user.click(pageButton("最后一页"));
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(100, 101));
    });
    // The page's only row is the detail.
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第100句。");
    await user.click(screen.getByRole("button", { name: "删除" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(pageCount()).toHaveTextContent("第 2 / 2 页 · 共 100 条");
    });
    expect(rowIds(log)).toEqual(ids(50, 100));
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第50句。");
    expect(within(pager()).queryByRole("button", { name: "第 3 页" })).toBeNull();
    backend.destroy();
  });

  it("the detail keeps a chosen entry while it is on the page; after another page or a search that leaves it out, it shows the first row there", async () => {
    fiftyAPage();
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(120) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(50);
    });
    await user.click(within(log).getByRole("button", { name: /第3句。/ }));
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第3句。");
    await user.click(pageButton("第 2 页"));
    await waitFor(() => {
      expect(screen.getByTestId("entry-text")).toHaveTextContent("第50句。");
    });
    expect(within(log).getByRole("button", { pressed: true })).toHaveTextContent("第50句。");
    // A pick on the page stays through a search that finds it…
    await user.click(within(log).getByRole("button", { name: /第61句。/ }));
    const box = screen.getByRole("textbox", { name: "搜索历史" });
    await user.type(box, "第6");
    await waitFor(() => {
      expect(pageCount()).toHaveTextContent("第 1 / 1 页 · 找到 11 条");
    });
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第61句。");
    expect(within(log).getByRole("button", { pressed: true })).toHaveTextContent("第61句。");
    // …and gives way to the first match of one that leaves it out.
    await user.clear(box);
    await user.type(box, "第7");
    await waitFor(() => {
      expect(screen.getByTestId("entry-text")).toHaveTextContent("第7句。");
    });
    expect(within(log).getByRole("button", { pressed: true })).toHaveTextContent("第7句。");
    backend.destroy();
  });

  it("regression: the log and the detail fill the window under what is above them, side by side and equally tall, each scrolling inside itself (user requests 2026-10-13, 2026-10-26)", async () => {
    // jsdom lays nothing out: the scroll area (the shell's <main>) gets a height and the grid a
    // place on the page, and a smaller window or a taller banner is played through
    // ResizeObserver. Until 0.1.9 the detail stuck beside a log that ran down the page, a card
    // shorter than the window.
    let height = 640;
    let gridTop = 180;
    const observers = new Set<() => void>();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(private readonly callback: () => void) {}
        observe() {
          observers.add(this.callback);
        }
        unobserve() {}
        disconnect() {
          observers.delete(this.callback);
        }
      },
    );
    vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
      return this.querySelector(':scope > [data-testid="page-history"]') === null ? 0 : height;
    });
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (
      this: Element,
    ) {
      const top =
        this.classList.contains("grid") && this.parentElement?.dataset.testid === "page-history"
          ? gridTop
          : 0;
      return new DOMRect(0, top, 800, 400);
    });
    try {
      const backend = new MockBackend({ history: rows(120) });
      renderApp({ path: "/history", backend });
      const page = await screen.findByTestId("page-history");
      const area = page.parentElement;
      expect(area?.tagName).toBe("MAIN");
      expect(area).toHaveClass("overflow-auto");
      expect(page).toHaveClass("p-6");
      expect(page.style.getPropertyValue("--scroll-area-h")).toBe("640px");
      expect(page.style.getPropertyValue("--split-top")).toBe("180px");
      // From lg on the grid is the area's height less its top and the page's bottom padding,
      // never under 480 px, in one row the two columns fill.
      const split = page.querySelector(":scope > .grid");
      if (!(split instanceof HTMLElement)) throw new Error("grid");
      const [logPanel, detail, ...rest] = [...split.children];
      expect(rest).toEqual([]);
      expect(logPanel).toBe(screen.getByTestId("history-log"));
      expect(detail).toContainElement(screen.getByTestId("entry-text"));
      expect(split).toHaveClass(
        "lg:h-[max(480px,calc(var(--scroll-area-h,100vh)_-_var(--split-top,0px)_-_1.5rem))]",
        "lg:grid-rows-[minmax(0,1fr)]",
      );
      expect(split.className).not.toMatch(/sticky|self-start|(?:^|\s)(?:lg:)?items-/);
      // The log's rows scroll inside it, its page buttons under them; the detail scrolls as a
      // whole. Both may shrink to the row.
      expect(logPanel).toHaveClass("lg:min-h-0");
      const list = screen.getByTestId("history-rows");
      expect(list).toHaveClass("lg:min-h-0", "lg:flex-1", "lg:overflow-y-auto");
      const pagerBox = screen.getByTestId("history-pager");
      expect(
        list.compareDocumentPosition(pagerBox) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      expect(logPanel).toContainElement(pagerBox);
      expect(detail).toHaveClass("lg:min-h-0", "lg:overflow-y-auto");
      // Below lg the two stack and grow with what they hold: every height rule is an lg rule.
      expect(
        split.className
          .split(/\s+/)
          .filter((c) => /(?:^|:)h-|grid-rows/.test(c) && !c.startsWith("lg:")),
      ).toEqual([]);
      // A smaller window, and more above the grid: both follow.
      height = 480;
      gridTop = 260;
      act(() => {
        for (const measure of observers) measure();
      });
      expect(page.style.getPropertyValue("--scroll-area-h")).toBe("480px");
      expect(page.style.getPropertyValue("--split-top")).toBe("260px");
      backend.destroy();
    } finally {
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
    }
  });

  it("another page, filter or search starts the log's rows at their top", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(120) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(10);
    });
    const list = screen.getByTestId("history-rows");
    list.scrollTop = 300;
    await user.click(pageButton("下一页"));
    await waitFor(() => {
      expect(rowIds(log)).toEqual(ids(10, 20));
    });
    expect(list.scrollTop).toBe(0);
    list.scrollTop = 200;
    await user.click(screen.getByRole("radio", { name: "已收藏" }));
    await waitFor(() => {
      expect(list.scrollTop).toBe(0);
    });
    backend.destroy();
  });

  it("the detail starts at its top for each entry it shows, wherever the last one was scrolled to", async () => {
    fiftyAPage();
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(120) });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(rowIds(log)).toHaveLength(50);
    });
    const detail = screen.getByTestId("page-history").querySelector(".grid")?.children[1];
    if (!(detail instanceof HTMLElement)) throw new Error("detail");
    detail.scrollTop = 480;
    await user.click(within(log).getByRole("button", { name: /第2句。/ }));
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第2句。");
    expect(detail.scrollTop).toBe(0);
    // The same entry in another view keeps its place.
    detail.scrollTop = 120;
    await user.click(screen.getByRole("radio", { name: "原文" }));
    expect(screen.getByTestId("entry-text")).toHaveTextContent("第2句");
    expect(detail.scrollTop).toBe(120);
    backend.destroy();
  });

  it("another page brings the top of the log into view when the page was scrolled past it", async () => {
    fiftyAPage();
    const user = userEvent.setup();
    const scrollIntoView = vi.fn();
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      value: scrollIntoView,
      configurable: true,
      writable: true,
    });
    try {
      const backend = new MockBackend({ history: rows(250) });
      renderApp({ path: "/history", backend });
      const log = await screen.findByRole("list", { name: "听写记录" });
      await waitFor(() => {
        expect(rowIds(log)).toHaveLength(50);
      });
      const panel = screen.getByTestId("history-log");
      // It stops where the detail sticks, the page's padding under the top.
      expect(panel).toHaveClass("scroll-mt-6");
      // Scrolled down to the page buttons: the log's top is above the scroll area's.
      const box = vi
        .spyOn(panel, "getBoundingClientRect")
        .mockReturnValue(new DOMRect(0, -1800, 400, 2600));
      await user.click(pageButton("下一页"));
      expect(scrollIntoView).toHaveBeenCalledTimes(1);
      expect(scrollIntoView.mock.contexts[0]).toBe(panel);
      expect(scrollIntoView).toHaveBeenLastCalledWith({ block: "start" });
      // With its top in view nothing moves, for a page or a page size.
      box.mockReturnValue(new DOMRect(0, 120, 400, 2600));
      await user.click(pageButton("下一页"));
      await user.selectOptions(screen.getByRole("combobox", { name: "每页显示条数" }), "20");
      expect(scrollIntoView).toHaveBeenCalledTimes(1);
      box.mockReturnValue(new DOMRect(0, -40, 400, 2600));
      await user.selectOptions(screen.getByRole("combobox", { name: "每页显示条数" }), "100");
      expect(scrollIntoView).toHaveBeenCalledTimes(2);
      backend.destroy();
    } finally {
      delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });

  it("regression: says nothing before the first answer and keeps the rows on screen while a new search is answered", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ history: rows(3) });
    const answer = backend.historyQuery.bind(backend);
    // Every query waits until the test lets it through.
    const waiting: (() => void)[] = [];
    vi.spyOn(backend, "historyQuery").mockImplementation(
      (args) =>
        new Promise((resolve, reject) => {
          waiting.push(() => {
            answer(args).then(resolve, reject);
          });
        }),
    );
    const releaseAll = () => {
      for (const release of waiting.splice(0)) release();
    };
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(waiting.length).toBeGreaterThan(0);
    });
    // Nothing is known yet: no rows and no 「暂无结果」.
    expect(screen.queryByText("全部暂无结果。")).toBeNull();
    expect(within(log).queryAllByRole("button", { name: /第\d句。/ })).toHaveLength(0);
    releaseAll();
    expect(await within(log).findByRole("button", { name: /第0句。/ })).toBeInTheDocument();
    // A search that matches nothing: the old rows stay until its answer, then the empty state
    // names the search.
    await user.type(screen.getByRole("textbox", { name: "搜索历史" }), "zzz");
    await waitFor(() => {
      expect(waiting.length).toBeGreaterThan(0);
    });
    expect(within(log).getByRole("button", { name: /第0句。/ })).toBeInTheDocument();
    expect(screen.queryByText("没有结果匹配「zzz」")).toBeNull();
    releaseAll();
    expect(await screen.findByText("没有结果匹配「zzz」")).toBeInTheDocument();
    expect(within(log).queryAllByRole("button", { name: /第\d句。/ })).toHaveLength(0);
    backend.destroy();
  });

  it("opens an entry past the loaded page by its id (the home table's row link)", async () => {
    const backend = new MockBackend({ history: rows(150) });
    renderApp({ path: `/history?filter=${rows(150)[140]?.id ?? ""}`, backend });
    expect(await screen.findByTestId("entry-text")).toHaveTextContent("第140句。");
    backend.destroy();
  });
});

describe("History page · long entries (docs/dictation.md section 22)", () => {
  const segment = (n: number) => ({
    text: `第${n}段${"字".repeat(100)}。`,
    start_ms: (n - 1) * 30_000,
    end_ms: n * 30_000,
  });
  /** A ten-minute take of 20 segments, about 2100 characters: not cleaned up at the take. */
  function longEntry(overrides: Partial<HistoryEntry> = {}): HistoryEntry {
    const segments = Array.from({ length: 20 }, (_, i) => segment(i + 1));
    const text = segments.map((s) => s.text).join("");
    return {
      id: "long-take",
      at_ms: Date.now() - 60_000,
      raw_text: text,
      text,
      refined: false,
      asr_model: "Qwen3-ASR-1.7B",
      duration_ms: 600_000,
      asr_ms: 20_000,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      segments,
      ...overrides,
    };
  }

  it("processes a long entry with a preset in parts, shows the progress, keeps the result beside the original and exports subtitles and text", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const backend = new MockBackend({ now: () => Date.now(), history: [longEntry()] });
      renderApp({ path: "/history", backend });
      const tools = await screen.findByTestId("history-long-tools");
      expect(tools).toHaveTextContent("全文按句子分成不超过 1500 字的部分依次处理");
      expect(tools).toHaveTextContent("内置 AI 服务受免费额度限制，长文处理较慢。");
      expect(screen.queryByRole("radio", { name: "处理后" })).toBeNull();
      await user.selectOptions(within(tools).getByRole("combobox", { name: "预设" }), "notes");
      await user.click(within(tools).getByTestId("history-long-start"));
      expect(await within(tools).findByTestId("history-long-progress")).toHaveTextContent(
        "已处理 0/3 部分",
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS);
      });
      expect(within(tools).getByTestId("history-long-progress")).toHaveTextContent(
        "已处理 1/3 部分",
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS * 2);
      });
      expect(within(tools).queryByTestId("history-long-progress")).toBeNull();
      expect(within(tools).getByTestId("history-long-start")).toHaveTextContent("重新处理");
      await user.click(screen.getByRole("radio", { name: "处理后" }));
      const processed = backend.peek().history_recent[0]?.processed;
      expect(processed?.preset).toEqual({ id: "notes", name: "要点纪要" });
      expect(screen.getByTestId("entry-processed")).toHaveTextContent(processed?.text ?? "none");
      expect(screen.getByTestId("entry-text")).toHaveTextContent("由「要点纪要」处理 · 原文保留");
      expect(backend.peek().history_recent[0]?.text).toBe(longEntry().text);
      // Ten minutes read as such, not as 600.0 s.
      expect(screen.getByText("10 分")).toBeInTheDocument();
      // Exports: the page's name for the file, the shell's answer in a toast.
      await user.click(within(tools).getByTestId("history-export-srt"));
      expect(
        await screen.findByText(/已保存：\/home\/you\/Documents\/Voltip .*\.srt/),
      ).toBeInTheDocument();
      await user.click(within(tools).getByTestId("history-export-txt"));
      await waitFor(() => {
        expect(backend.exports.map((e) => e.format)).toEqual(["srt", "txt"]);
      });
      expect(backend.exports[0]?.fileName).toMatch(/^Voltip \d{4}-\d{2}-\d{2} \d{2}\.\d{2}$/);
      expect(within(tools).getByTestId("history-export-txt-note")).toHaveTextContent(
        "导出文本时使用处理后文本。",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("a cancel keeps nothing; without segments there are no subtitles; a short entry has no tools; without an AI service processing says so", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const backend = new MockBackend({
        now: () => Date.now(),
        history: [
          longEntry({ segments: undefined }),
          {
            ...longEntry(),
            id: "short",
            at_ms: Date.now() - 120_000,
            duration_ms: 3000,
            text: "短句。",
            raw_text: "短句",
            segments: undefined,
          },
        ],
      });
      renderApp({ path: "/history", backend });
      const tools = await screen.findByTestId("history-long-tools");
      await user.click(within(tools).getByTestId("history-long-start"));
      await within(tools).findByTestId("history-long-progress");
      await user.click(within(tools).getByTestId("history-long-cancel"));
      expect(await within(tools).findByTestId("history-long-cancelled")).toHaveTextContent(
        "已取消，未保存结果。",
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS * 5);
      });
      expect(backend.peek().history_recent[0]?.processed).toBeUndefined();
      const srt = within(tools).getByTestId("history-export-srt");
      expect(srt).toBeDisabled();
      expect(srt).toHaveAttribute("title", "这条记录没有分段，无法导出字幕。");
      // A short entry: the tools are not there.
      await user.click(
        within(screen.getByRole("list", { name: "听写记录" })).getByRole("button", {
          name: /短句。/,
        }),
      );
      expect(screen.queryByTestId("history-long-tools")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
    const unconfigured = new MockBackend({ history: [longEntry()], builtIn: {} });
    renderApp({ path: "/history", backend: unconfigured });
    const clicks = userEvent.setup();
    const shown = await screen.findByTestId("history-long-tools");
    await clicks.click(within(shown).getByTestId("history-long-start"));
    expect(await within(shown).findByTestId("history-long-failed")).toHaveTextContent(
      "处理失败：尚未配置 AI 润色服务，无法用预设处理",
    );
  });
});
