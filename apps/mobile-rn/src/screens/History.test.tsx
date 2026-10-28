// 记录 on the phone (docs/dictation.md §4.4): searched and filtered by the core, a page at a time,
// with what to do when nothing matches.
import { fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type HistoryEntry } from "@voltip/shared";

import { HISTORY_PAGE, SEARCH_DEBOUNCE_MS } from "../hooks/useHistoryList";
import { openTab, renderApp } from "../test/render";

const DAY = 86_400_000;
const LONG = { timeout: SEARCH_DEBOUNCE_MS * 50 };

function entry(n: number, over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: Date.now() - n * 60_000,
    raw_text: `第 ${n} 条`,
    text: `第 ${n} 条。`,
    refined: false,
    asr_model: "whisper-large-v3",
    duration_ms: 3_000,
    asr_ms: 200,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...over,
  };
}

const rows = () => screen.queryAllByTestId("phone-history-row");

describe("记录", () => {
  it("says when a search finds nothing, and clears it", async () => {
    await renderApp({ mock: { history: [entry(1), entry(2)] } });
    await openTab("history");
    await waitFor(() => {
      expect(rows()).toHaveLength(2);
    });
    await fireEvent.changeText(screen.getByTestId("phone-history-search"), "开会");
    expect(await screen.findByText("没有结果匹配「开会」", undefined, LONG)).toBeOnTheScreen();
    expect(screen.getByText("已记录的 2 条都不匹配。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("清除搜索"));
    await waitFor(() => {
      expect(rows()).toHaveLength(2);
    }, LONG);
    expect(screen.getByTestId("phone-history-search")).toHaveDisplayValue("");
  });

  it("says what each empty filter means: stars are made in an entry, a span can be changed", async () => {
    await renderApp({ mock: { history: [entry(1, { at_ms: Date.now() - 40 * DAY })] } });
    await openTab("history");
    await waitFor(() => {
      expect(rows()).toHaveLength(1);
    });
    await fireEvent.press(screen.getByTestId("history-filter-starred"));
    expect(await screen.findByText("已收藏暂无结果。")).toBeOnTheScreen();
    expect(screen.getByText("在记录详情中点「收藏」。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("history-filter-today"));
    expect(await screen.findByText("今天暂无结果。")).toBeOnTheScreen();
    expect(screen.getByText("可选择其他时间范围。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("history-filter-failed"));
    expect(await screen.findByText("未完成暂无结果。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("history-filter-all"));
    await waitFor(() => {
      expect(rows()).toHaveLength(1);
    });
  });

  it("loads the next page from the button and on reaching the end of the list", async () => {
    const many = Array.from({ length: HISTORY_PAGE * 2 + 10 }, (_, i) => entry(i + 1));
    await renderApp({ mock: { history: many } });
    await openTab("history");
    await waitFor(() => {
      expect(rows()).toHaveLength(HISTORY_PAGE);
    });
    expect(screen.getByText(`已显示 ${HISTORY_PAGE} / ${many.length} 条`)).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("加载更多"));
    await waitFor(() => {
      expect(rows()).toHaveLength(HISTORY_PAGE * 2);
    });
    // Scrolling to the end of the page loads the rest.
    await fireEvent.scroll(screen.getByTestId("phone-history"), {
      nativeEvent: {
        layoutMeasurement: { height: 800 },
        contentOffset: { y: 9_000 },
        contentSize: { height: 9_500 },
      },
    });
    await waitFor(() => {
      expect(rows()).toHaveLength(many.length);
    });
    expect(screen.queryByText("加载更多")).toBeNull();
    // A scroll short of the end asks for nothing.
    await fireEvent.scroll(screen.getByTestId("phone-history"), {
      nativeEvent: {
        layoutMeasurement: { height: 800 },
        contentOffset: { y: 0 },
        contentSize: { height: 9_500 },
      },
    });
    expect(
      within(screen.getByTestId("phone-history")).getAllByTestId("phone-history-row"),
    ).toHaveLength(many.length);
  });
});
