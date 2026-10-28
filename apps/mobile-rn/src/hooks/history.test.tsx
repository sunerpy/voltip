// The history hooks the phone's 记录 and 记录详情 run on (docs/dictation.md §4.4, §20.8, §22), on the
// mock core: the list a page at a time, the entry the detail asks for, and 用 AI 预设处理.
import { act, renderHook, waitFor } from "@testing-library/react-native";
import { type HistoryEntry, type HistoryPage, type HistoryQueryArgs } from "@voltip/shared";
import { MockBackend, type MockMirror, mockMirrorProfile } from "@voltip/shared/mock";
import type { ReactNode } from "react";

import { BackendProvider } from "../backend/BackendProvider";
import { HISTORY_PAGE, useHistoryEntry, useHistoryList } from "./useHistoryList";
import { exportName, isLongEntry, useHistoryProcess } from "./useHistoryProcess";

const NOW = Date.now();
const DESKTOP = "cd".repeat(32);

function entry(n: number, over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
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

const many = (count: number) => Array.from({ length: count }, (_, i) => entry(i + 1));

const backends: MockBackend[] = [];
afterEach(() => {
  for (const b of backends.splice(0)) b.destroy();
});

function on(backend: MockBackend) {
  backends.push(backend);
  return ({ children }: { children: ReactNode }) => (
    <BackendProvider backend={backend}>{children}</BackendProvider>
  );
}

/** `backend`'s queries, each held until it is answered with what the core says, or failed. The
 *  spy goes in before the hook renders: the hook binds the query once. */
function hold(backend: MockBackend) {
  const real = backend.historyQuery.bind(backend);
  const waiting: { args: HistoryQueryArgs; resolve: (page: HistoryPage) => void }[] = [];
  let mode: "hold" | "pass" | "fail" = "pass";
  const spy = jest.spyOn(backend, "historyQuery").mockImplementation((args) => {
    if (mode === "fail") return Promise.reject(new Error("busy"));
    if (mode === "pass") return real(args);
    return new Promise<HistoryPage>((resolve) => {
      waiting.push({ args, resolve });
    });
  });
  return {
    spy,
    waiting,
    set: (next: typeof mode) => {
      mode = next;
    },
    /** Answer the held query at `index` (its place among those still held). */
    answer: async (index: number) => {
      const [w] = waiting.splice(index, 1);
      if (w !== undefined) w.resolve(await real(w.args));
    },
  };
}

describe("useHistoryList", () => {
  it("loads a page at a time, reloads what is loaded on a history event, and stops at the end", async () => {
    const backend = new MockBackend({ role: "phone", history: many(HISTORY_PAGE + 30) });
    const { result } = await renderHook(() => useHistoryList("all", "", NOW), {
      wrapper: on(backend),
    });
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.entries).toHaveLength(HISTORY_PAGE);
    expect(result.current.matching).toBe(HISTORY_PAGE + 30);
    expect(result.current.more).toBe(true);
    await act(async () => {
      result.current.loadMore();
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(result.current.entries).toHaveLength(HISTORY_PAGE + 30);
    });
    expect(result.current.more).toBe(false);
    // Nothing left to load: no query.
    const query = jest.spyOn(backend, "historyQuery");
    await act(async () => {
      result.current.loadMore();
      await Promise.resolve();
    });
    expect(query).not.toHaveBeenCalled();
    // A star is a history event: everything loaded so far is loaded again.
    await act(async () => {
      await backend.invoke("history_star", { id: entry(1).id, starred: true });
    });
    await waitFor(() => {
      expect(result.current.entries[0]?.starred).toBe(true);
    });
    expect(result.current.entries).toHaveLength(HISTORY_PAGE + 30);
  });

  it("keeps the previous rows until a new search answers, and drops a page asked for the old one", async () => {
    const backend = new MockBackend({ role: "phone", history: many(HISTORY_PAGE + 5) });
    const slow = hold(backend);
    const { result, rerender } = await renderHook(
      ({ search }: { search: string }) => useHistoryList("all", search, NOW),
      { wrapper: on(backend), initialProps: { search: "" } },
    );
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    slow.set("hold");
    await act(async () => {
      result.current.loadMore();
      await Promise.resolve();
    });
    await rerender({ search: "第 1" });
    expect(result.current.settled).toBe(false);
    expect(result.current.more).toBe(false);
    expect(result.current.entries).toHaveLength(HISTORY_PAGE);
    // Not settled: a further page is not asked for.
    const asked = slow.spy.mock.calls.length;
    await act(async () => {
      result.current.loadMore();
      await Promise.resolve();
    });
    expect(slow.spy.mock.calls.length).toBe(asked);
    expect(slow.waiting.map((w) => w.args.query ?? "")).toEqual(["", "第 1"]);
    // The new search answers first; the old search's page arrives late and is dropped.
    await act(async () => {
      await slow.answer(1);
    });
    expect(result.current.settled).toBe(true);
    const found = result.current.entries.length;
    expect(found).toBeGreaterThan(0);
    expect(result.current.entries.every((e) => e.text.includes("第 1"))).toBe(true);
    await act(async () => {
      await slow.answer(0);
    });
    expect(result.current.entries).toHaveLength(found);
  });

  it("stays unsettled while the core does not answer, and keeps its rows when a further page fails", async () => {
    const backend = new MockBackend({ role: "phone", history: many(HISTORY_PAGE + 1) });
    const slow = hold(backend);
    slow.set("fail");
    const { result, rerender } = await renderHook(
      ({ filter }: { filter: "all" | "today" }) => useHistoryList(filter, "", NOW),
      { wrapper: on(backend), initialProps: { filter: "all" } },
    );
    await waitFor(() => {
      expect(slow.spy.mock.calls.length).toBeGreaterThan(1);
    });
    expect(result.current.settled).toBe(false);
    expect(result.current.entries).toEqual([]);
    slow.set("pass");
    await rerender({ filter: "today" });
    await waitFor(() => {
      expect(result.current.settled).toBe(true);
    });
    await rerender({ filter: "all" });
    await waitFor(() => {
      expect(result.current.entries).toHaveLength(HISTORY_PAGE);
    });
    slow.set("fail");
    await act(async () => {
      result.current.loadMore();
      await Promise.resolve();
    });
    expect(result.current.entries).toHaveLength(HISTORY_PAGE);
    expect(result.current.more).toBe(true);
  });

  it("lists a computer's copy, reloaded when the copy changes", async () => {
    const mirror: MockMirror = {
      view: { desktop: DESKTOP, name: "办公室电脑", state: "up_to_date", entries: 2 },
      history: many(2),
      profile: mockMirrorProfile(),
    };
    const backend = new MockBackend({ role: "phone", history: many(5), mirrors: [mirror] });
    const { result } = await renderHook(() => useHistoryList("all", "", NOW, DESKTOP), {
      wrapper: on(backend),
    });
    await waitFor(() => {
      expect(result.current.matching).toBe(2);
    });
    await act(async () => {
      backend.simulateMirror({
        ...mirror,
        view: { ...mirror.view, entries: 3 },
        history: many(3),
      });
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(result.current.entries).toHaveLength(3);
    });
  });
});

describe("useHistoryEntry", () => {
  it("takes a loaded entry from the list, asks the core for another, and has none for a gone one", async () => {
    const backend = new MockBackend({ role: "phone", history: many(3) });
    const loaded = [entry(1)];
    const { result, rerender } = await renderHook(
      ({ id }: { id: string | undefined }) => useHistoryEntry(id, loaded),
      { wrapper: on(backend), initialProps: { id: undefined as string | undefined } },
    );
    expect(result.current).toBeUndefined();
    await rerender({ id: entry(1).id });
    expect(result.current).toBe(loaded[0]);
    await rerender({ id: entry(3).id });
    await waitFor(() => {
      expect(result.current?.id).toBe(entry(3).id);
    });
    await rerender({ id: entry(9).id });
    // Until the answer (and after it: the core has none) there is no entry.
    expect(result.current).toBeUndefined();
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current).toBeUndefined();
    jest.spyOn(backend, "historyEntry").mockRejectedValueOnce(new Error("busy"));
    await rerender({ id: entry(2).id });
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current).toBeUndefined();
  });
});

describe("useHistoryProcess", () => {
  const long = entry(1, { duration_ms: 180_000 });

  it("follows its request to the processed text and ignores other requests", async () => {
    const backend = new MockBackend({ role: "phone", history: [long] });
    const { result } = await renderHook(() => useHistoryProcess(long.id), {
      wrapper: on(backend),
    });
    expect(result.current.view).toEqual({ state: "idle" });
    // Cancelling with nothing running asks nothing.
    const invoke = jest.spyOn(backend, "invoke");
    result.current.cancel();
    expect(invoke).not.toHaveBeenCalled();
    await act(async () => {
      result.current.start("proofread");
      await Promise.resolve();
    });
    expect(result.current.view.state).toBe("running");
    await act(async () => {
      backend.publish({
        type: "history_process",
        request_id: -1,
        id: long.id,
        state: { state: "failed", reason: "别人的" },
      });
      await Promise.resolve();
    });
    expect(result.current.view.state).toBe("running");
    await waitFor(() => {
      expect(result.current.view.state).toBe("done");
    });
    const view = result.current.view;
    expect(view.state === "done" && view.processed.preset.id).toBe("proofread");
  });

  it("cancels on request and when the page leaves, and shows a failure", async () => {
    const backend = new MockBackend({ role: "phone", history: [long] });
    const { result, rerender, unmount } = await renderHook(
      ({ id }: { id: string }) => useHistoryProcess(id),
      { wrapper: on(backend), initialProps: { id: long.id } },
    );
    await act(async () => {
      result.current.start("proofread");
      await Promise.resolve();
    });
    await act(async () => {
      result.current.cancel();
      await Promise.resolve();
    });
    expect(result.current.view).toEqual({ state: "cancelled" });
    // Another entry shows nothing of this one's request.
    await rerender({ id: entry(2).id });
    expect(result.current.view).toEqual({ state: "idle" });
    await act(async () => {
      result.current.start("proofread");
      await Promise.resolve();
    });
    expect(result.current.view).toEqual({ state: "failed", reason: "这条记录已删除" });
    await rerender({ id: long.id });
    await act(async () => {
      result.current.start("proofread");
      await Promise.resolve();
    });
    const invoke = jest.spyOn(backend, "invoke");
    await unmount();
    expect(invoke).toHaveBeenCalledWith("history_process_cancel", {
      requestId: expect.any(Number),
    });
  });

  it("knows a long entry and names an export by its local time", () => {
    expect(isLongEntry(long)).toBe(true);
    expect(isLongEntry(entry(1, { text: "长".repeat(2001) }))).toBe(true);
    expect(isLongEntry(entry(1))).toBe(false);
    expect(isLongEntry({ ...long, kind: "edit" })).toBe(false);
    expect(exportName(new Date(2026, 8, 3, 7, 5).getTime())).toBe("Voltip 2026-09-03 07.05");
  });
});
