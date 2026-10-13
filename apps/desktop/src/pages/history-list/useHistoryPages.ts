import {
  type Backend,
  type HistoryEntry,
  type HistoryFilter,
  type HistoryQueryArgs,
  historyQueryArgs,
  startOfDay,
} from "@voltip/shared";
import { useBackend, useUiState } from "@voltip/ui";
import { useCallback, useEffect, useMemo, useState } from "react";

/** How many pages `matching` entries fill at `size` a page; one for none. */
export function pageCount(matching: number, size: number): number {
  return Math.max(1, Math.ceil(matching / size));
}

export interface HistoryPages {
  /** The entries of the page shown, newest first. */
  entries: HistoryEntry[];
  /** Entries matching the filter and the search, on every page. */
  matching: number;
  /** The page shown, from 1: the one asked for, or the last page once that one has gone. */
  page: number;
  /** How many pages `matching` fills, at least 1. */
  pages: number;
  /** `entries` and `matching` answer the current filter, search, page size and page; until they
   *  do the previous rows stay on screen (and before the first answer there are none). */
  settled: boolean;
  /** What the list shows apart from what is in it: the filter, the search, the page size and the
   *  page. A new dictation, a star or a deletion leaves it as it is. */
  view: string;
  /** Show another page; a page past either end shows that end. */
  goTo: (page: number) => void;
}

type Query = Omit<HistoryQueryArgs, "offset" | "limit">;

/** Page `page` of `size` entries, or the last page when `page` lies past it: when the entries of
 *  the page shown went (the last one deleted), the page before it, without an empty step. */
async function loadPage(
  backend: Backend,
  args: Query,
  page: number,
  size: number,
): Promise<{ page: number; entries: HistoryEntry[]; matching: number }> {
  const at = (p: number) => backend.historyQuery({ ...args, offset: (p - 1) * size, limit: size });
  const asked = await at(page);
  const last = pageCount(asked.matching, size);
  if (page <= last) return { page, entries: asked.entries, matching: asked.matching };
  const end = await at(last);
  return { page: last, entries: end.entries, matching: end.matching };
}

/** The desktop history page's list (docs/dictation.md §4.4, user request 2026-10-13): one page of
 *  `size` entries at a time through `history_query` (`offset` + `limit`), the filter and the search
 *  run by the core. Another filter, search or page size starts again at page 1. Every history event
 *  (a new take, a star, a deletion) asks for the same page again, so the user stays where they are
 *  while new entries push the rows down by one; a page that has no entries left gives way to the
 *  last one. The phone keeps `useHistoryList`, which loads more as it scrolls. */
export function useHistoryPages(
  filter: HistoryFilter,
  search: string,
  now: number,
  size: number,
): HistoryPages {
  const { backend } = useBackend();
  // A new array on every history event: the reload signal.
  const { history_recent: revision } = useUiState();
  const day = startOfDay(now);
  const args = useMemo(() => historyQueryArgs(filter, search, day), [filter, search, day]);
  // The list by value: the clock passing midnight gives `args` a new identity, not 全部 a new list.
  const list = `${JSON.stringify(args)}|${size}`;
  const [asked, setAsked] = useState({ list, page: 1 });
  // Another list starts at page 1, and so does a list come back to (a search cleared): the page
  // asked for belongs to the list it was asked in, adjusted during render.
  if (asked.list !== list) setAsked({ list, page: 1 });
  const page = asked.list === list ? asked.page : 1;
  const view = `${list}|${page}`;
  const [shown, setShown] = useState<{ view: string; entries: HistoryEntry[]; matching: number }>({
    view: "",
    entries: [],
    matching: 0,
  });

  useEffect(() => {
    let live = true;
    void loadPage(backend, args, page, size).then(
      (loaded) => {
        if (!live) return;
        setShown({
          view: `${list}|${loaded.page}`,
          entries: loaded.entries,
          matching: loaded.matching,
        });
        if (loaded.page !== page) setAsked({ list, page: loaded.page });
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
    // A history event (a new take, a star, a deletion) is a reason to load again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, args, list, page, size, revision]);

  const pages = pageCount(shown.matching, size);
  const goTo = useCallback(
    (next: number) => {
      setAsked({ list, page: Math.min(Math.max(1, Math.trunc(next)), pages) });
    },
    [list, pages],
  );
  return {
    entries: shown.entries,
    matching: shown.matching,
    page,
    pages,
    settled: shown.view === view,
    view,
    goTo,
  };
}
