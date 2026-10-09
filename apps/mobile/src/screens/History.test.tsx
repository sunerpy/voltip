import { type HistoryEntry, startOfDay } from "@voltip/shared";
import { MockBackend, sampleDevices } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";
import { chooseOption, selectTrigger } from "../test/select";

// The phone's history (user decision 2026-10-01: the phone has the desktop's history, for what it
// recognises itself; user decision 2026-10-03: and for the takes it sends to a computer, which
// that computer keeps in full).
const NOW = Date.now();

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
  it("regression: today, this week and this month count today's takes", async () => {
    // User report 2026-10-03 (「听写次数统计一直为 0」): the page handed the seconds of \`useNow\` to
    // the hooks that take milliseconds, so its days were in January 1970 and only 累计 counted.
    const today = (n: number) => take(n, { at_ms: Math.min(NOW, startOfDay(NOW) + n * 1000) });
    renderApp({ mock: { history: [today(1), today(2), today(3)] }, initialScreen: "history" });
    const page = await screen.findByTestId("phone-history");
    for (const span of ["today", "week", "month", "total"]) {
      await waitFor(() => {
        expect(within(page).getByTestId(`phone-history-stat-${span}`)).toHaveTextContent(
          "听写 3 次",
        );
      });
    }
    // The list's days are today's too.
    expect(within(page).getByRole("list", { name: "听写记录" })).toHaveTextContent("今天");
  });

  it("regression: a take sent to a computer is kept on the phone, counts, and opens with where it went", async () => {
    // User report 2026-10-03: the phone's counts stayed at 0 while its takes went to a computer,
    // which alone recorded them; the user decided the same day that the phone keeps them too
    // (docs/dictation.md §20.7). The phone has the text the computer reported, not its models.
    const user = userEvent.setup();
    const [paired] = sampleDevices(Math.floor(NOW / 1000));
    if (paired === undefined) throw new Error("no sample device");
    const computer = { ...paired, device: { ...paired.device, name: "Studio PC" } };
    const backend = new MockBackend({ role: "phone", devices: [computer] });
    renderApp({ backend });
    await screen.findByTestId("phone-mic");
    await act(() => backend.invoke("phone_take_start", { publicKey: computer.device.public_key }));
    await waitFor(() => {
      expect(backend.peek().phone_take?.state.state).toBe("listening");
    });
    await act(() => backend.invoke("phone_take_stop"));
    // 说话: the recent results list it, and the row opens its page.
    const recent = await screen.findByTestId("phone-recent", {}, { timeout: 3000 });
    expect(within(recent).getAllByTestId("phone-recent-row")).toHaveLength(1);
    await user.click(screen.getByTestId("tab-history"));
    const page = await screen.findByTestId("phone-history");
    await waitFor(() => {
      expect(within(page).getByTestId("phone-history-stats")).toHaveTextContent("听写 1 次");
    });
    const row = within(page).getAllByTestId("phone-history-row")[0] as HTMLElement;
    expect(row).toHaveTextContent("发送到 Studio PC");
    await user.click(row);
    const entry = await screen.findByTestId("phone-entry");
    expect(within(entry).getByText("发送到 Studio PC")).toBeInTheDocument();
    expect(within(entry).getByTestId("phone-entry-sent-note")).toHaveTextContent(
      "识别和润色的详细信息保存在电脑的记录中。",
    );
    // What the phone does not know is not shown as empty or zero.
    expect(within(entry).queryByText("识别模型")).toBeNull();
    expect(within(entry).queryByText("耗时")).toBeNull();
    backend.destroy();
  });

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

    await chooseOption(user, selectTrigger("筛选", page), "starred");
    await waitFor(() => {
      expect(within(page).getAllByTestId("phone-history-row")).toHaveLength(1);
    });
    await chooseOption(user, selectTrigger("筛选", page), "failed");
    expect(await within(page).findByText("未完成暂无结果。")).toBeInTheDocument();
    await chooseOption(user, selectTrigger("筛选", page), "all");

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

  it("docs/dictation.md section 3.6: an entry whose clean-up was turned down says why it is not polished", async () => {
    const user = userEvent.setup();
    const busy = take(1, {
      refined: false,
      refine_model: undefined,
      refine_ms: undefined,
      text: "原文 1",
      refine_failure: "quota",
    });
    const backend = new MockBackend({ role: "phone", history: [busy] });
    renderApp({ backend, initialScreen: "history" });
    await user.click((await screen.findAllByTestId("phone-history-row"))[0] as HTMLElement);
    const entry = await screen.findByTestId("phone-entry");
    expect(within(entry).getByText("未润色 · 额度已用完")).toBeInTheDocument();
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
    await chooseOption(user, selectTrigger("预设", tools), "notes");
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
    await chooseOption(user, selectTrigger("保留最近", page), "2000");
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

  it("regression: a recent result on 说话 opens its entry", async () => {
    // User report 2026-10-03: a result under 最近结果 could be copied or shared but not opened,
    // while a row of 记录 opens the entry's page. The row's text is that row's open target now.
    const user = userEvent.setup();
    const backend = new MockBackend({
      role: "phone",
      history: [take(1), take(2, { text: "明天交周报。" })],
    });
    renderApp({ backend });
    const recent = await screen.findByTestId("phone-recent");
    const second = within(recent).getAllByTestId("phone-recent-row")[1] as HTMLElement;
    await user.click(within(second).getByRole("button", { name: "打开「明天交周报。」" }));
    const entry = await screen.findByTestId("phone-entry");
    expect(screen.getByRole("heading", { name: "记录详情", level: 1 })).toBeInTheDocument();
    expect(within(entry).getByTestId("phone-entry-text")).toHaveTextContent("明天交周报。");
    // 返回 leads back to 说话, where the list was.
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByTestId("tab-talk")).toHaveAttribute("aria-current", "page");
    // Copy and share stay buttons of their own beside the open target, never inside it.
    const row = within(await screen.findByTestId("phone-recent")).getAllByTestId(
      "phone-recent-row",
    )[0] as HTMLElement;
    const open = within(row).getByRole("button", { name: "打开「第 1 条记录。」" });
    expect(within(open).queryAllByRole("button")).toHaveLength(0);
    expect(within(row).getByRole("button", { name: "复制「第 1 条记录。」" })).not.toContainElement(
      open,
    );
    expect(within(row).getByRole("button", { name: "分享「第 1 条记录。」" })).not.toContainElement(
      open,
    );
    backend.destroy();
  });
});
