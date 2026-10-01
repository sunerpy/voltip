import type { HistoryEntry } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, configure, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// The phone's history (user decision 2026-10-01: the phone has the desktop's history, for what it
// recognises itself; a take sent to a computer is in that computer's history).
const NOW = Date.now();

// A page opened here shows the core's answers asynchronously; on a busy machine (CI's coverage
// run, 2026-10-01: a page opened after 返回 had not drawn its rows within the default second)
// they take longer, so the waits for them do too. Nothing here times how fast a page reacts.
configure({ asyncUtilTimeout: 5000 });

function take(n: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
    raw_text: `原文 ${n}`,
    text: `第 ${n} 条记录。`,
    refined: true,
    refine_model: "clean-up",
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 3000,
    asr_ms: 300,
    refine_ms: 200,
    outcome: { kind: "inserted", via: "clipboard" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...extra,
  };
}

/** A take past two minutes with segments: the long-entry tools apply (docs/dictation.md §22). */
const LONG = take(9, {
  text: "今天的会议讨论了三件事。".repeat(300),
  raw_text: "今天的会议讨论了三件事".repeat(300),
  duration_ms: 600_000,
  segments: [{ text: "第一段。", start_ms: 0, end_ms: 1000 }],
});

describe("the phone's history", () => {
  it("the 记录 tab counts, searches, filters and opens what the phone recognised", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      role: "phone",
      history: [take(1, { starred: true }), take(2), take(3, { text: "明天交周报。" })],
    });
    renderApp({ backend });
    await user.click(await screen.findByTestId("tab-history"));
    const page = screen.getByTestId("phone-history");
    expect(screen.getByRole("heading", { name: "记录", level: 1 })).toBeInTheDocument();
    expect(screen.getByTestId("tab-history")).toHaveAttribute("aria-current", "page");
    expect(await within(page).findAllByTestId("phone-history-row")).toHaveLength(3);
    await waitFor(() => {
      expect(within(page).getByTestId("phone-history-stats")).toHaveTextContent("听写 3 次");
    });
    expect(within(page).getByTestId("phone-history-retention")).toHaveTextContent("保留最近");

    await user.type(within(page).getByRole("textbox", { name: "搜索历史" }), "周报");
    await waitFor(() => {
      expect(within(page).getAllByTestId("phone-history-row")).toHaveLength(1);
    });
    await user.clear(within(page).getByRole("textbox", { name: "搜索历史" }));
    await user.type(within(page).getByRole("textbox", { name: "搜索历史" }), "没有这个");
    expect(await within(page).findByText("没有结果匹配「没有这个」")).toBeInTheDocument();
    await user.click(within(page).getByRole("button", { name: "清除搜索" }));
    await waitFor(() => {
      expect(within(page).getAllByTestId("phone-history-row")).toHaveLength(3);
    });

    await user.selectOptions(within(page).getByRole("combobox", { name: "筛选" }), "starred");
    await waitFor(() => {
      expect(within(page).getAllByTestId("phone-history-row")).toHaveLength(1);
    });
    await user.selectOptions(within(page).getByRole("combobox", { name: "筛选" }), "failed");
    expect(await within(page).findByText("未完成暂无结果。")).toBeInTheDocument();
    await user.selectOptions(within(page).getByRole("combobox", { name: "筛选" }), "all");

    const rows = await within(page).findAllByTestId("phone-history-row");
    await user.click(rows[2] as HTMLElement);
    const entry = await screen.findByTestId("phone-entry");
    expect(screen.getByRole("heading", { name: "记录详情", level: 1 })).toBeInTheDocument();
    expect(within(entry).getByTestId("phone-entry-text")).toHaveTextContent("明天交周报。");
    await user.click(within(entry).getByRole("radio", { name: "原文" }));
    expect(within(entry).getByTestId("phone-entry-text")).toHaveTextContent("原文 3");
    expect(within(entry).getByText("clean-up")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("phone-history")).toBeInTheDocument();
    backend.destroy();
  });

  it("an entry copies, shares, stars and deletes after a confirmation", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [take(1), take(2)] });
    renderApp({ backend, initialScreen: "history" });
    const rows = await screen.findAllByTestId("phone-history-row");
    await user.click(rows[0] as HTMLElement);
    const entry = await screen.findByTestId("phone-entry");
    await user.click(within(entry).getByRole("button", { name: "复制" }));
    await waitFor(() => {
      expect(backend.phoneClipboard).toBe("第 1 条记录。");
    });
    expect(await screen.findByText("已复制到剪贴板 · 8 字")).toBeInTheDocument();
    await user.click(within(entry).getByRole("button", { name: "分享" }));
    await waitFor(() => {
      expect(backend.shared).toEqual(["第 1 条记录。"]);
    });
    await user.click(within(entry).getByRole("button", { name: "收藏" }));
    await waitFor(() => {
      expect(within(entry).getByRole("button", { name: "取消收藏" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });
    expect((await backend.historyEntry(take(1).id))?.starred).toBe(true);

    await user.click(within(entry).getByRole("button", { name: "删除" }));
    const confirm = screen.getByRole("dialog", { name: "删除这条记录？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    expect(await screen.findByTestId("phone-history")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getAllByTestId("phone-history-row")).toHaveLength(1);
    });
    expect(await backend.historyEntry(take(1).id)).toBeNull();
    backend.destroy();
  });

  it("a refused action is a message, and an entry that is gone says so", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [take(1)] });
    renderApp({ backend, initialScreen: "history" });
    await user.click((await screen.findAllByTestId("phone-history-row"))[0] as HTMLElement);
    const entry = await screen.findByTestId("phone-entry");
    vi.spyOn(backend, "pasteText").mockResolvedValueOnce({ kind: "failed", reason: "inject" });
    await user.click(within(entry).getByRole("button", { name: "复制" }));
    expect(await screen.findByText("复制失败")).toBeInTheDocument();
    vi.spyOn(backend, "pasteText").mockRejectedValueOnce(new Error("no clipboard"));
    await user.click(within(entry).getByRole("button", { name: "复制" }));
    await waitFor(() => {
      expect(screen.getAllByText("复制失败")).toHaveLength(2);
    });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("share: 没有可以分享的应用"));
    await user.click(within(entry).getByRole("button", { name: "分享" }));
    expect(await screen.findByText("出错了 · 没有可以分享的应用")).toBeInTheDocument();
    // Cleared elsewhere: the page says the entry is gone.
    await act(() => backend.invoke("history_clear"));
    expect(await screen.findByText("这条记录已删除。")).toBeInTheDocument();
    backend.destroy();
  });

  it("a long entry is processed with a preset and shared as subtitles or text", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [LONG, take(1)] });
    renderApp({ backend, initialScreen: "history" });
    const rows = await screen.findAllByTestId("phone-history-row");
    // A short entry has no long-entry tools.
    await user.click(rows[0] as HTMLElement);
    expect(
      within(await screen.findByTestId("phone-entry")).queryByTestId("phone-entry-long"),
    ).toBeNull();
    await user.click(screen.getByRole("button", { name: "返回" }));
    await user.click((await screen.findAllByTestId("phone-history-row"))[1] as HTMLElement);
    const entry = await screen.findByTestId("phone-entry");
    const tools = within(entry).getByTestId("phone-entry-long");
    await user.selectOptions(within(tools).getByRole("combobox", { name: "预设" }), "notes");
    await user.click(within(tools).getByRole("button", { name: "开始处理" }));
    expect(await within(tools).findByRole("button", { name: "取消" })).toBeInTheDocument();
    await waitFor(
      () => {
        expect(within(entry).getByRole("radio", { name: "处理后" })).toBeInTheDocument();
      },
      { timeout: 5000 },
    );
    await user.click(within(entry).getByRole("radio", { name: "处理后" }));
    expect(within(entry).getByText(/由「要点纪要」处理/)).toBeInTheDocument();
    expect(within(tools).getByText("导出文本时使用处理后文本。")).toBeInTheDocument();

    await user.click(within(tools).getByRole("button", { name: "分享字幕（SRT）" }));
    await user.click(within(tools).getByRole("button", { name: "分享文本（TXT）" }));
    await waitFor(() => {
      expect(backend.exports.map((e) => e.format)).toEqual(["srt", "txt"]);
    });
    vi.spyOn(backend, "historyExport").mockResolvedValueOnce({
      kind: "failed",
      code: "share",
      detail: "没有可以分享的应用",
    });
    await user.click(within(tools).getByRole("button", { name: "分享文本（TXT）" }));
    expect(await screen.findByText("无法打开分享：没有可以分享的应用")).toBeInTheDocument();
    backend.destroy();
  });

  it("a long entry without segments has no subtitles, and a run can be cancelled", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [{ ...LONG, segments: undefined }] });
    renderApp({ backend, initialScreen: "history" });
    await user.click((await screen.findAllByTestId("phone-history-row"))[0] as HTMLElement);
    const tools = within(await screen.findByTestId("phone-entry")).getByTestId("phone-entry-long");
    expect(within(tools).getByRole("button", { name: "分享字幕（SRT）" })).toBeDisabled();
    await user.click(within(tools).getByRole("button", { name: "开始处理" }));
    await user.click(await within(tools).findByRole("button", { name: "取消" }));
    expect(await within(tools).findByText("已取消，未保存结果。")).toBeInTheDocument();
    backend.destroy();
  });

  it("more entries load on request, and an empty history says where takes appear", async () => {
    const user = userEvent.setup();
    const many = Array.from({ length: 120 }, (_, i) => take(i + 1));
    const backend = new MockBackend({ role: "phone", history: many });
    renderApp({ backend, initialScreen: "history" });
    expect(await screen.findAllByTestId("phone-history-row")).toHaveLength(100);
    expect(screen.getByText("已显示 100 / 120 条")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "加载更多" }));
    await waitFor(() => {
      expect(screen.getAllByTestId("phone-history-row")).toHaveLength(120);
    });
    expect(screen.queryByRole("button", { name: "加载更多" })).toBeNull();
    backend.destroy();

    const empty = new MockBackend({ role: "phone" });
    renderApp({ backend: empty, initialScreen: "history" });
    expect(await screen.findByText(/在手机上识别的结果会出现在这里/)).toBeInTheDocument();
    empty.destroy();
  });

  it("the history settings switch recording off, keep fewer entries and clear after a confirmation", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [take(1), take(2)] });
    renderApp({ backend, initialScreen: "settings" });
    const row = await screen.findByTestId("settings-historySettings");
    expect(row).toHaveTextContent("保留最近");
    await user.click(row);
    const page = screen.getByTestId("phone-history-settings");
    expect(within(page).getByTestId("phone-history-count")).toHaveTextContent("2 /");
    await user.selectOptions(within(page).getByRole("combobox", { name: "保留最近" }), "2000");
    await waitFor(() => {
      expect(backend.peek().settings.history.keep).toBe(2000);
    });
    await user.click(within(page).getByRole("switch", { name: "保存听写历史" }));
    await waitFor(() => {
      expect(backend.peek().settings.history.enabled).toBe(false);
    });
    await user.click(within(page).getByRole("button", { name: "清空历史" }));
    const confirm = screen.getByRole("dialog", { name: "清空 2 条历史记录？" });
    await user.click(within(confirm).getByRole("button", { name: "清空" }));
    await waitFor(() => {
      expect(backend.peek().history_total).toBe(0);
    });
    expect(within(page).getByRole("button", { name: "清空历史" })).toBeDisabled();
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("settings: 写不进去"));
    await user.click(within(page).getByRole("switch", { name: "保存听写历史" }));
    expect(await screen.findByText("出错了 · 写不进去")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("settings-historySettings")).toHaveTextContent("不保存新的记录");
    // The talk tab's recent results lead to the history.
    backend.destroy();
  });

  it("the talk tab's recent results lead to the whole history", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", history: [take(1)] });
    renderApp({ backend });
    const recent = await screen.findByTestId("phone-recent");
    await user.click(within(recent).getByRole("button", { name: "全部记录" }));
    expect(await screen.findByTestId("phone-history")).toBeInTheDocument();
    backend.destroy();
  });
});
