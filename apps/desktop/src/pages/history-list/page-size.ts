import { useCallback, useState } from "react";

/** The page sizes the history page offers (user requests 2026-10-13, 2026-10-26). Each is at most
 *  `HISTORY_QUERY_LIMIT` (200), so a page is one `history_query`. */
export const HISTORY_PAGE_SIZES = [10, 20, 50, 100, 200] as const;
export type HistoryPageSize = (typeof HISTORY_PAGE_SIZES)[number];

/** 10 a page (user request 2026-10-26): about what the log shows in a window's height, so the
 *  page buttons sit under the rows without scrolling. A size chosen before stays chosen. */
export const DEFAULT_HISTORY_PAGE_SIZE: HistoryPageSize = 10;

/** Where this window keeps the choice, beside the sidebar's and the appearance knobs
 *  (`voltip.sidebar`, `voltip.appearance`): a way of viewing the list, not a setting of the
 *  app, so it is not part of the settings the phone mirrors or a backup carries. */
export const HISTORY_PAGE_STORAGE_KEY = "voltip.historyPage";

export function isHistoryPageSize(value: unknown): value is HistoryPageSize {
  return HISTORY_PAGE_SIZES.some((size) => size === value);
}

/** The stored page size; the default for nothing stored, a value no longer offered, or a storage
 *  that cannot be read. */
export function readHistoryPageSize(
  storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage,
): HistoryPageSize {
  try {
    const raw = storage?.getItem(HISTORY_PAGE_STORAGE_KEY);
    if (!raw) return DEFAULT_HISTORY_PAGE_SIZE;
    const parsed: unknown = JSON.parse(raw);
    const size =
      typeof parsed === "object" && parsed !== null && "size" in parsed ? parsed.size : undefined;
    return isHistoryPageSize(size) ? size : DEFAULT_HISTORY_PAGE_SIZE;
  } catch (_error) {
    return DEFAULT_HISTORY_PAGE_SIZE;
  }
}

export function writeHistoryPageSize(
  size: HistoryPageSize,
  storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage,
): void {
  try {
    storage?.setItem(HISTORY_PAGE_STORAGE_KEY, JSON.stringify({ size }));
  } catch (_error) {
    // A full or disabled storage keeps the choice for this session only.
  }
}

/** The page size and a setter that stores it. */
export function useHistoryPageSize(
  storage?: Storage,
): [HistoryPageSize, (size: HistoryPageSize) => void] {
  const [size, setSize] = useState<HistoryPageSize>(() => readHistoryPageSize(storage));
  const choose = useCallback(
    (next: HistoryPageSize) => {
      setSize(next);
      writeHistoryPageSize(next, storage);
    },
    [storage],
  );
  return [size, choose];
}
