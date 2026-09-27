import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_SIDEBAR_LAYOUT,
  SIDEBAR_STORAGE_KEY,
  readSidebarLayout,
  useSidebarLayout,
  writeSidebarLayout,
} from "./sidebar-layout";

function memory(entries: Record<string, string> = {}): Storage {
  const map = new Map(Object.entries(entries));
  return {
    get length() {
      return map.size;
    },
    clear: () => {
      map.clear();
    },
    getItem: (key) => map.get(key) ?? null,
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (key) => {
      map.delete(key);
    },
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

describe("sidebar layout", () => {
  it("regression: a hidden sidebar never comes back hidden after a restart; collapsed does", () => {
    const storage = memory();
    writeSidebarLayout({ collapsed: true, hidden: true }, storage);
    expect(JSON.parse(storage.getItem(SIDEBAR_STORAGE_KEY) ?? "")).toEqual({ collapsed: true });
    expect(readSidebarLayout(storage)).toEqual({ collapsed: true, hidden: false });
    // Even a stored `hidden` from elsewhere is not read back.
    storage.setItem(SIDEBAR_STORAGE_KEY, JSON.stringify({ collapsed: false, hidden: true }));
    expect(readSidebarLayout(storage)).toEqual(DEFAULT_SIDEBAR_LAYOUT);
  });

  it("falls back to the default on anything it cannot read", () => {
    expect(readSidebarLayout(memory())).toEqual(DEFAULT_SIDEBAR_LAYOUT);
    expect(readSidebarLayout(memory({ [SIDEBAR_STORAGE_KEY]: "{not json" }))).toEqual(
      DEFAULT_SIDEBAR_LAYOUT,
    );
    expect(readSidebarLayout(memory({ [SIDEBAR_STORAGE_KEY]: '"collapsed"' }))).toEqual(
      DEFAULT_SIDEBAR_LAYOUT,
    );
    expect(readSidebarLayout(memory({ [SIDEBAR_STORAGE_KEY]: '{"collapsed":"yes"}' }))).toEqual(
      DEFAULT_SIDEBAR_LAYOUT,
    );
    expect(readSidebarLayout(undefined)).toEqual(DEFAULT_SIDEBAR_LAYOUT);
    const full: Pick<Storage, "setItem"> = {
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(() => {
      writeSidebarLayout({ collapsed: true, hidden: false }, full);
    }).not.toThrow();
  });

  it("collapse and hide are independent; reveal only ever shows", () => {
    const storage = memory();
    const { result } = renderHook(() => useSidebarLayout(storage));
    act(() => {
      result.current.toggleCollapsed();
    });
    act(() => {
      result.current.toggleHidden();
    });
    expect(result.current.layout).toEqual({ collapsed: true, hidden: true });
    act(() => {
      result.current.reveal();
    });
    expect(result.current.layout).toEqual({ collapsed: true, hidden: false });
    act(() => {
      result.current.reveal();
    });
    expect(result.current.layout).toEqual({ collapsed: true, hidden: false });
    act(() => {
      result.current.toggleCollapsed();
    });
    expect(result.current.layout).toEqual(DEFAULT_SIDEBAR_LAYOUT);
    expect(JSON.parse(storage.getItem(SIDEBAR_STORAGE_KEY) ?? "")).toEqual({ collapsed: false });
  });
});
