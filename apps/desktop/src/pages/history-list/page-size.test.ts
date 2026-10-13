import { HISTORY_QUERY_LIMIT } from "@voltip/shared";
import { act, renderHook } from "@testing-library/react";
import {
  DEFAULT_HISTORY_PAGE_SIZE,
  HISTORY_PAGE_SIZES,
  HISTORY_PAGE_STORAGE_KEY,
  readHistoryPageSize,
  useHistoryPageSize,
  writeHistoryPageSize,
} from "./page-size";

/** A storage that holds what it is given. */
function memory(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => {
      map.clear();
    },
    getItem: (key) => map.get(key) ?? null,
    key: (index) => [...map.keys()][index] ?? null,
    removeItem: (key) => {
      map.delete(key);
    },
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

describe("the history page size (user request 2026-10-13)", () => {
  it("offers 20, 50, 100 and 200 entries a page, each page one history query, 50 by default", () => {
    expect(HISTORY_PAGE_SIZES).toEqual([20, 50, 100, 200]);
    expect(DEFAULT_HISTORY_PAGE_SIZE).toBe(50);
    for (const size of HISTORY_PAGE_SIZES) expect(size).toBeLessThanOrEqual(HISTORY_QUERY_LIMIT);
  });

  it("is kept in this window's storage, and anything else stored there reads as the default", () => {
    const storage = memory();
    expect(readHistoryPageSize(storage)).toBe(DEFAULT_HISTORY_PAGE_SIZE);
    writeHistoryPageSize(100, storage);
    expect(storage.getItem(HISTORY_PAGE_STORAGE_KEY)).toBe('{"size":100}');
    expect(readHistoryPageSize(storage)).toBe(100);
    for (const stored of ['{"size":30}', '{"size":"100"}', "{}", "null", "7", "not json"]) {
      storage.setItem(HISTORY_PAGE_STORAGE_KEY, stored);
      expect(readHistoryPageSize(storage)).toBe(DEFAULT_HISTORY_PAGE_SIZE);
    }
    // A storage that refuses: the default, and the choice lasts for this session only.
    const refusing = memory();
    refusing.getItem = () => {
      throw new Error("denied");
    };
    refusing.setItem = () => {
      throw new Error("full");
    };
    expect(readHistoryPageSize(refusing)).toBe(DEFAULT_HISTORY_PAGE_SIZE);
    expect(() => {
      writeHistoryPageSize(20, refusing);
    }).not.toThrow();
    // A webview without a storage at all: the same.
    vi.stubGlobal("localStorage", undefined);
    try {
      expect(readHistoryPageSize()).toBe(DEFAULT_HISTORY_PAGE_SIZE);
      expect(() => {
        writeHistoryPageSize(20);
      }).not.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("the hook starts from the stored size and stores a new choice", () => {
    const storage = memory();
    writeHistoryPageSize(200, storage);
    const { result } = renderHook(() => useHistoryPageSize(storage));
    expect(result.current[0]).toBe(200);
    act(() => {
      result.current[1](20);
    });
    expect(result.current[0]).toBe(20);
    expect(readHistoryPageSize(storage)).toBe(20);
    // The window's own storage when none is given.
    const own = renderHook(() => useHistoryPageSize());
    expect(own.result.current[0]).toBe(DEFAULT_HISTORY_PAGE_SIZE);
    act(() => {
      own.result.current[1](100);
    });
    expect(readHistoryPageSize(window.localStorage)).toBe(100);
  });
});
