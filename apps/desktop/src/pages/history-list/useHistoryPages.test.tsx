import type { HistoryEntry, HistoryFilter } from "@voltip/shared";
import { MOCK_ASR_MS, MOCK_MIC_READY_MS, MOCK_REFINE_MS, MockBackend } from "@voltip/shared/mock";
import { BackendProvider } from "@voltip/ui";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { pageCount, useHistoryPages } from "./useHistoryPages";

const NOW = Date.now();

/** `n` dictations a minute apart, the newest first; every tenth one starred. */
function rows(n: number): HistoryEntry[] {
  return Array.from({ length: n }, (_, i) => ({
    id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
    at_ms: NOW - i * 60_000,
    raw_text: `第${i}句`,
    text: `第${i}句。`,
    refined: false,
    asr_model: "m",
    duration_ms: 1000,
    asr_ms: 100,
    outcome: { kind: "inserted", via: "paste" },
    starred: i % 10 === 0,
    mode: "whole_take",
    kind: "dictation",
  }));
}

/** The ids of rows `from` to `to` (exclusive). */
const ids = (from: number, to: number) =>
  rows(to)
    .slice(from)
    .map((e) => e.id);

function wrap(backend: MockBackend) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <BackendProvider backend={backend}>{children}</BackendProvider>;
  };
}

interface Props {
  filter: HistoryFilter;
  search: string;
  size: number;
}

function mount(
  backend: MockBackend,
  initialProps: Props = { filter: "all", search: "", size: 50 },
) {
  return renderHook(
    ({ filter, search, size }: Props) => useHistoryPages(filter, search, NOW, size),
    { wrapper: wrap(backend), initialProps },
  );
}

/** One dictation through the in-memory core, as far as its history row (fake timers). */
async function dictate(backend: MockBackend) {
  await act(async () => {
    await backend.invoke("dictation_start");
    await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + 1000);
    await backend.invoke("dictation_stop");
    await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS + 500);
  });
}

describe("useHistoryPages (the desktop history page, user request 2026-10-13)", () => {
  it("counts the pages a number of entries fills, never fewer than one", () => {
    expect(pageCount(0, 50)).toBe(1);
    expect(pageCount(1, 50)).toBe(1);
    expect(pageCount(50, 50)).toBe(1);
    expect(pageCount(51, 50)).toBe(2);
    expect(pageCount(250, 20)).toBe(13);
  });

  it("shows one page at a time, in pages of the size asked for, and goes to any page within the range", async () => {
    const backend = new MockBackend({ history: rows(130) });
    const { result } = mount(backend);
    expect(result.current.settled).toBe(false);
    expect(result.current.entries).toEqual([]);
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 50));
    expect(result.current).toMatchObject({ page: 1, pages: 3, matching: 130 });
    act(() => {
      result.current.goTo(3);
    });
    // The page number moves at once; the rows stay until the page's answer.
    expect(result.current.page).toBe(3);
    expect(result.current.settled).toBe(false);
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 50));
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(100, 130));
    // Past either end: the nearest end.
    act(() => {
      result.current.goTo(9);
    });
    expect(result.current.page).toBe(3);
    act(() => {
      result.current.goTo(-2);
    });
    expect(result.current.page).toBe(1);
    await waitFor(() => {
      expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 50));
    });
    backend.destroy();
  });

  it("starts at page 1 for another filter, search or page size, and keeps the page through a day's turn", async () => {
    const backend = new MockBackend({ history: rows(130) });
    const { result, rerender } = mount(backend);
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    const at = async (page: number) => {
      act(() => {
        result.current.goTo(page);
      });
      await waitFor(() => {
        expect(result.current.settled).toBe(true);
      });
    };
    await at(2);
    // A search and back: page 1 again, not the page the list had before the search.
    rerender({ filter: "all", search: "第1", size: 50 });
    expect(result.current.page).toBe(1);
    rerender({ filter: "all", search: "", size: 50 });
    expect(result.current.page).toBe(1);
    await at(2);
    const before = result.current.view;
    rerender({ filter: "all", search: "", size: 20 });
    expect(result.current.page).toBe(1);
    expect(result.current.view).not.toBe(before);
    await waitFor(() => {
      expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 20));
    });
    expect(result.current.pages).toBe(7);
    await at(4);
    rerender({ filter: "all", search: "第1", size: 20 });
    expect(result.current.page).toBe(1);
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    // 第1句, 第10–19句, 第100–129句.
    expect(result.current.matching).toBe(1 + 10 + 30);
    await at(2);
    rerender({ filter: "starred", search: "第1", size: 20 });
    expect(result.current.page).toBe(1);
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    // 第10句 and 第100–120句 by tens.
    expect(result.current.entries.map((e) => e.text)).toEqual([
      "第10句。",
      "第100句。",
      "第110句。",
      "第120句。",
    ]);
    // The same filter and search with a new array of arguments (the clock passing midnight for
    // 全部): the page stays.
    rerender({ filter: "all", search: "", size: 50 });
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    await at(3);
    const tomorrow = renderHook(({ now }: { now: number }) => useHistoryPages("all", "", now, 50), {
      wrapper: wrap(backend),
      initialProps: { now: NOW },
    });
    await waitFor(() => {
      expect(tomorrow.result.current.settled).toBe(true);
    });
    act(() => {
      tomorrow.result.current.goTo(2);
    });
    tomorrow.rerender({ now: NOW + 86_400_000 });
    expect(tomorrow.result.current.page).toBe(2);
    tomorrow.unmount();
    backend.destroy();
  });

  it("regression: stays on its page when a new dictation arrives, and page 1 starts with it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({ history: rows(120) });
      const { result } = mount(backend);
      await waitFor(() => {
        expect(result.current.settled).toBe(true);
      });
      act(() => {
        result.current.goTo(2);
      });
      await waitFor(() => {
        expect(result.current.entries.map((e) => e.id)).toEqual(ids(50, 100));
      });
      await dictate(backend);
      // Still page 2, now one entry further down the list.
      await waitFor(() => {
        expect(result.current.matching).toBe(121);
      });
      expect(result.current.page).toBe(2);
      expect(result.current.settled).toBe(true);
      expect(result.current.entries.map((e) => e.id)).toEqual(ids(49, 99));
      const newest = backend.peek().history_recent[0];
      act(() => {
        result.current.goTo(1);
      });
      await waitFor(() => {
        expect(result.current.entries[0]?.id).toBe(newest?.id);
      });
      backend.destroy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("moves to the last page once the page it shows has no entries left, without showing it empty", async () => {
    const backend = new MockBackend({ history: rows(101) });
    // The number of rows of every render once page 3 is on screen.
    const shown: number[] = [];
    let recording = false;
    const { result } = renderHook(
      () => {
        const pages = useHistoryPages("all", "", NOW, 50);
        if (recording) shown.push(pages.entries.length);
        return pages;
      },
      { wrapper: wrap(backend) },
    );
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    act(() => {
      result.current.goTo(3);
    });
    await waitFor(() => {
      expect(result.current.entries.map((e) => e.id)).toEqual(ids(100, 101));
    });
    recording = true;
    const last = rows(101)[100];
    if (!last) throw new Error("fixture");
    await act(() => backend.invoke("history_delete", { id: last.id }));
    await waitFor(() => {
      expect(result.current.page).toBe(2);
    });
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    recording = false;
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(50, 100));
    expect(result.current).toMatchObject({ pages: 2, matching: 100 });
    expect(shown.length).toBeGreaterThan(0);
    expect(shown.filter((n) => n === 0)).toEqual([]);
    // Everything gone: one empty page.
    await act(() => backend.invoke("history_clear"));
    await waitFor(() => {
      expect(result.current.matching).toBe(0);
    });
    expect(result.current).toMatchObject({ page: 1, pages: 1, entries: [], settled: true });
    backend.destroy();
  });

  it("keeps the rows of the last answer when a query fails, and drops an answer that came too late", async () => {
    const backend = new MockBackend({ history: rows(130) });
    const answer = backend.historyQuery.bind(backend);
    const { result } = mount(backend);
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    const query = vi.spyOn(backend, "historyQuery").mockRejectedValueOnce(new Error("busy"));
    act(() => {
      result.current.goTo(2);
    });
    await waitFor(() => {
      expect(query).toHaveBeenCalledTimes(1);
    });
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 50));
    expect(result.current.settled).toBe(false);
    // Page 3's answer is held back until page 1 has been asked for and answered.
    let release: () => void = () => undefined;
    query.mockImplementationOnce(
      (args) =>
        new Promise((resolve, reject) => {
          release = () => {
            answer(args).then(resolve, reject);
          };
        }),
    );
    act(() => {
      result.current.goTo(3);
    });
    act(() => {
      result.current.goTo(1);
    });
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    release();
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.page).toBe(1);
    expect(result.current.entries.map((e) => e.id)).toEqual(ids(0, 50));
    backend.destroy();
  });
});
