import {
  HISTORY_QUERY_LIMIT,
  type HistoryEntry,
  type HistoryFilter,
  type HistoryQueryArgs,
  historyQueryArgs,
  startOfDay,
} from "@voltip/shared";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";

/** Entries the history page loads at a time. */
export const HISTORY_PAGE = 100;
/** How long the search waits after the last keystroke before it asks the core. */
export const SEARCH_DEBOUNCE_MS = 200;

/** `value`, once it has not changed for `ms`. */
export function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => {
      setSettled(value);
    }, ms);
    return () => {
      clearTimeout(timer);
    };
  }, [value, ms]);
  return settled;
}

export interface HistoryList {
  /** The entries loaded so far, newest first. */
  entries: HistoryEntry[];
  /** Entries matching the filter and the search, loaded or not. */
  matching: number;
  /** `entries` and `matching` answer the current filter and search (not the previous ones, which
   *  stay on screen until the answer arrives, nor the empty list before the first answer). */
  settled: boolean;
  /** Another page exists. */
  more: boolean;
  /** Load the next page. */
  loadMore: () => void;
}

/** The history page's list, on the desktop and the phone (docs/dictation.md §4.4): `filter` and `search` go to the core as
 *  `history_query`, a page of `HISTORY_PAGE` at a time. Every history event (a new take, a star, a
 *  deletion) reloads what is loaded, so the list never shows an entry the core no longer has.
 *  With `desktop` (a computer's key, on the phone) the list is the phone's copy of that computer's
 *  history (`mirror_history_query`, §20.8), reloaded whenever the copy changes. */
export function useHistoryList(
  filter: HistoryFilter,
  search: string,
  now: number,
  desktop?: string,
): HistoryList {
  const { backend } = useBackend();
  // A new array on every history event, or the copy's view when it changes: the reload signal.
  const { history_recent: own, mirrors } = useUiState();
  const copy = desktop === undefined ? undefined : mirrors.find((m) => m.desktop === desktop);
  const revision =
    desktop === undefined ? own : `${copy?.state}:${copy?.entries}:${copy?.synced_at_ms}`;
  const query = useMemo(
    () =>
      desktop === undefined
        ? backend.historyQuery.bind(backend)
        : (a: HistoryQueryArgs) => backend.mirrorHistoryQuery(desktop, a),
    [backend, desktop],
  );
  const day = startOfDay(now);
  const args = useMemo(() => historyQueryArgs(filter, search, day), [filter, search, day]);
  // The arguments and the computer together: a change of either starts the list over.
  const request = useMemo(() => ({ args, desktop }), [args, desktop]);
  const [list, setList] = useState<{ request: object; entries: HistoryEntry[]; matching: number }>({
    request: {},
    entries: [],
    matching: 0,
  });
  // How many entries the list keeps loaded for `request`: one page, plus one per `loadMore`.
  const wanted = useRef({ request, count: HISTORY_PAGE });

  useEffect(() => {
    if (wanted.current.request !== request) wanted.current = { request, count: HISTORY_PAGE };
    let live = true;
    void loadUpTo(query, request.args, wanted.current.count).then(
      (loaded) => {
        if (live) setList({ request, ...loaded });
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
    // A history event (a new take, a star, a deletion) is a reason to load again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [query, request, revision]);

  // Until the answer for new arguments arrives the previous rows stay: the list changes in one
  // step instead of flashing empty (and the empty state) on every keystroke or filter.
  const current = list.request === request;
  const loaded = list;
  const loadMore = useCallback(() => {
    const offset = loaded.entries.length;
    if (!current || offset >= loaded.matching) return;
    wanted.current = { request, count: offset + HISTORY_PAGE };
    void query({ ...request.args, offset, limit: HISTORY_PAGE }).then(
      (page) => {
        setList((prev) =>
          prev.request === request && prev.entries.length === offset
            ? { request, entries: [...prev.entries, ...page.entries], matching: page.matching }
            : prev,
        );
      },
      () => undefined,
    );
  }, [query, request, current, loaded.entries.length, loaded.matching]);

  return {
    entries: loaded.entries,
    matching: loaded.matching,
    settled: current,
    more: current && loaded.entries.length < loaded.matching,
    loadMore,
  };
}

/** The first `count` entries `args` selects, `HISTORY_QUERY_LIMIT` at a time. */
async function loadUpTo(
  query: (args: HistoryQueryArgs) => Promise<{ entries: HistoryEntry[]; matching: number }>,
  args: Omit<HistoryQueryArgs, "offset" | "limit">,
  count: number,
): Promise<{ entries: HistoryEntry[]; matching: number }> {
  const entries: HistoryEntry[] = [];
  let matching = 0;
  do {
    // One page after another: where the next starts, and whether there is one, depend on this one.
    // oxlint-disable-next-line no-await-in-loop -- sequential by design, see above
    const page = await query({
      ...args,
      offset: entries.length,
      limit: Math.min(HISTORY_QUERY_LIMIT, count - entries.length),
    });
    matching = page.matching;
    entries.push(...page.entries);
    if (page.entries.length === 0) break;
  } while (entries.length < Math.min(count, matching));
  return { entries, matching };
}

/** The entry the detail pane shows: from the loaded list when it is there, otherwise asked for
 *  with `history_entry` (the home table links to any entry). `undefined` for none or a gone one. */
export function useHistoryEntry(
  id: string | undefined,
  loaded: readonly HistoryEntry[],
): HistoryEntry | undefined {
  const { backend } = useBackend();
  const { history_recent: revision } = useUiState();
  const inList = id === undefined ? undefined : loaded.find((e) => e.id === id);
  const [fetched, setFetched] = useState<{ id: string; entry: HistoryEntry | null } | undefined>();
  const missing = id !== undefined && inList === undefined;
  useEffect(() => {
    if (!missing) return;
    let live = true;
    void backend.historyEntry(id).then(
      (entry) => {
        if (live) setFetched({ id, entry });
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
    // A history event (a star, a deletion) is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, id, missing, revision]);
  if (inList !== undefined) return inList;
  return fetched !== undefined && fetched.id === id ? (fetched.entry ?? undefined) : undefined;
}
