import type { HistoryEntry } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { BackendProvider } from "../../backend/BackendProvider";
import { useDebounced, useHistoryEntry, useHistoryList } from "./useHistoryList";
import { exportName, isLongEntry, useHistoryProcess } from "./useHistoryProcess";
import { useHomeStats } from "./useHomeStats";

const NOW = Date.now();

function take(n: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
    raw_text: `原文 ${n}`,
    text: `第 ${n} 条。`,
    refined: false,
    asr_model: "m",
    duration_ms: 3000,
    asr_ms: 300,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...extra,
  };
}

function wrap(backend: MockBackend) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <BackendProvider backend={backend}>{children}</BackendProvider>;
  };
}

describe("the history hooks (the desktop's and the phone's history pages)", () => {
  it("loads a page at a time, follows the filter and the search, and reloads on a history event", async () => {
    const backend = new MockBackend({
      history: Array.from({ length: 130 }, (_, i) => take(i + 1, { starred: i === 4 })),
    });
    const { result, rerender } = renderHook(
      ({ filter, search }: { filter: "all" | "starred"; search: string }) =>
        useHistoryList(filter, search, NOW),
      { wrapper: wrap(backend), initialProps: { filter: "all", search: "" } },
    );
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.entries).toHaveLength(100);
    expect(result.current.matching).toBe(130);
    expect(result.current.more).toBe(true);
    act(() => {
      result.current.loadMore();
    });
    await waitFor(() => {
      expect(result.current.entries).toHaveLength(130);
    });
    expect(result.current.more).toBe(false);
    // A load past the end asks nothing.
    act(() => {
      result.current.loadMore();
    });
    rerender({ filter: "starred", search: "" });
    expect(result.current.settled).toBe(false);
    await waitFor(() => {
      expect(result.current.entries.map((e) => e.id)).toEqual([take(5).id]);
    });
    rerender({ filter: "all", search: "第 7 条" });
    await waitFor(() => {
      expect(result.current.entries.map((e) => e.id)).toEqual([take(7).id]);
    });
    // A star is a history event: the loaded entries come again.
    await act(() => backend.invoke("history_star", { id: take(7).id, starred: true }));
    await waitFor(() => {
      expect(result.current.entries[0]?.starred).toBe(true);
    });
    backend.destroy();
  });

  it("finds an entry in the list or asks for it, and none once it is gone", async () => {
    const backend = new MockBackend({ history: [take(1), take(2)] });
    const loaded = [take(1)];
    const first: { id: string | undefined } = { id: take(1).id };
    const { result, rerender } = renderHook(
      ({ id }: { id: string | undefined }) => useHistoryEntry(id, loaded),
      { wrapper: wrap(backend), initialProps: first },
    );
    expect(result.current?.id).toBe(take(1).id);
    rerender({ id: take(2).id });
    await waitFor(() => {
      expect(result.current?.id).toBe(take(2).id);
    });
    await act(() => backend.invoke("history_delete", { id: take(2).id }));
    await waitFor(() => {
      expect(result.current).toBeUndefined();
    });
    rerender({ id: undefined });
    expect(result.current).toBeUndefined();
    backend.destroy();
  });

  it("debounces a value", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(({ v }: { v: string }) => useDebounced(v, 200), {
      initialProps: { v: "a" },
    });
    rerender({ v: "ab" });
    expect(result.current).toBe("a");
    act(() => {
      vi.advanceTimersByTime(200);
    });
    expect(result.current).toBe("ab");
    vi.useRealTimers();
  });

  it("processes an entry with a preset, cancels a run and forgets it for another entry", async () => {
    const long = take(1, { text: "今天的会议讨论了三件事。".repeat(300), duration_ms: 600_000 });
    expect(isLongEntry(long)).toBe(true);
    expect(isLongEntry(take(2))).toBe(false);
    expect(exportName(new Date(2026, 8, 30, 15, 30).getTime())).toBe("Voltip 2026-09-30 15.30");
    const backend = new MockBackend({ history: [long, take(2)] });
    const { result, rerender } = renderHook(({ id }: { id: string }) => useHistoryProcess(id), {
      wrapper: wrap(backend),
      initialProps: { id: long.id },
    });
    expect(result.current.view).toEqual({ state: "idle" });
    act(() => {
      result.current.start("notes");
    });
    expect(result.current.view.state).toBe("running");
    await waitFor(
      () => {
        expect(result.current.view.state).toBe("done");
      },
      { timeout: 5000 },
    );
    act(() => {
      result.current.start("proofread");
    });
    act(() => {
      result.current.cancel();
    });
    await waitFor(() => {
      expect(result.current.view.state).toBe("cancelled");
    });
    // Another entry: nothing of the first one shows, and a run still going is cancelled.
    act(() => {
      result.current.start("proofread");
    });
    rerender({ id: take(2).id });
    expect(result.current.view).toEqual({ state: "idle" });
    act(() => {
      result.current.cancel();
    });
    backend.destroy();
  });

  it("counts today, this week, this month and all time from history_stats", async () => {
    const backend = new MockBackend({ history: [take(1), take(2)] });
    const { result } = renderHook(() => useHomeStats(NOW), { wrapper: wrap(backend) });
    expect(result.current.total.count).toBe(0);
    await waitFor(() => {
      expect(result.current.total.count).toBe(2);
    });
    expect(result.current.today.count).toBeLessThanOrEqual(2);
    backend.destroy();
  });
});
