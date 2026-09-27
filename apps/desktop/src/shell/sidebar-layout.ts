import { useCallback, useState } from "react";

/** How the sidebar sits in the window (docs/frontend.md §3): expanded with labels, collapsed to a
 *  56 px icon rail, or hidden, when a strip on the left edge brings it back as a floating preview.
 *  Two flags rather than one state, because they combine: a hidden sidebar remembers whether it
 *  comes back as the rail. Only `collapsed` survives a restart: a sidebar that opens hidden reads
 *  as a sidebar that is gone. */
export interface SidebarLayout {
  readonly collapsed: boolean;
  readonly hidden: boolean;
}

export const SIDEBAR_STORAGE_KEY = "voltip.sidebar";

export const DEFAULT_SIDEBAR_LAYOUT: SidebarLayout = { collapsed: false, hidden: false };

/** The layout stored in `storage`, `hidden` never restored. */
export function readSidebarLayout(
  storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage,
): SidebarLayout {
  try {
    const raw = storage?.getItem(SIDEBAR_STORAGE_KEY);
    if (!raw) return DEFAULT_SIDEBAR_LAYOUT;
    const parsed: unknown = JSON.parse(raw);
    const collapsed =
      typeof parsed === "object" &&
      parsed !== null &&
      "collapsed" in parsed &&
      parsed.collapsed === true;
    return { collapsed, hidden: false };
  } catch (_error) {
    return DEFAULT_SIDEBAR_LAYOUT;
  }
}

/** Store what may be restored: `collapsed` only. */
export function writeSidebarLayout(
  layout: SidebarLayout,
  storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage,
): void {
  try {
    storage?.setItem(SIDEBAR_STORAGE_KEY, JSON.stringify({ collapsed: layout.collapsed }));
  } catch (_error) {
    // A full or disabled storage keeps the layout for this session only.
  }
}

export interface SidebarLayoutControls {
  layout: SidebarLayout;
  toggleCollapsed: () => void;
  /** Hide the sidebar, or show it again. */
  toggleHidden: () => void;
  /** Show a hidden sidebar (the edge preview's pin, the title bar's button): never hides. */
  reveal: () => void;
}

export function useSidebarLayout(storage?: Storage): SidebarLayoutControls {
  const [layout, setLayout] = useState<SidebarLayout>(() => readSidebarLayout(storage));
  const update = useCallback(
    (next: (prev: SidebarLayout) => SidebarLayout) => {
      setLayout((prev) => {
        const value = next(prev);
        writeSidebarLayout(value, storage);
        return value;
      });
    },
    [storage],
  );
  const toggleCollapsed = useCallback(() => {
    update((prev) => ({ ...prev, collapsed: !prev.collapsed }));
  }, [update]);
  const toggleHidden = useCallback(() => {
    update((prev) => ({ ...prev, hidden: !prev.hidden }));
  }, [update]);
  const reveal = useCallback(() => {
    update((prev) => (prev.hidden ? { ...prev, hidden: false } : prev));
  }, [update]);
  return { layout, toggleCollapsed, toggleHidden, reveal };
}
