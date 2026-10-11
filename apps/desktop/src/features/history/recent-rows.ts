import { type RefObject, useLayoutEffect, useState } from "react";

/** A row of the home page's recent results, and the heading of each day they are grouped by. */
export const RECENT_ROW_SELECTOR = "[data-recent-row]";
export const RECENT_DAY_SELECTOR = "[data-recent-day]";

export interface RowFit {
  /** The height the list may take before the page runs past the bottom of its scroll area, in
   *  px: the scroll area less everything else on the page. */
  budget: number;
  /** One row's height and one day heading's, in px; 0 before anything is laid out. */
  row: number;
  heading: number;
  /** For each entry the list could show, newest first, whether it opens a new day (and brings that
   *  day's heading with it). */
  starts: readonly boolean[];
  /** The fewest rows to show, as long as there are that many. */
  min: number;
}

/** How many of the newest entries the home page's recent results show (user request 2026-10-01: a
 *  tall window left a large blank space under the list): as many rows, with the heading of every
 *  day they open, as the list's share of the window holds, but never under `min` (or all there
 *  are, when there are fewer) and never more than there are. Without a layout to measure, `min`.
 *
 *  The answer depends on the window and the entries, never on the rows on screen, so showing them
 *  cannot change it. Counting on from the rows shown made a maximized window flicker for ever
 *  between N and N + 1 rows when row N + 1 opened a new day: its heading pushed the page past the
 *  bottom, the row went, the heading with it, and there was room again (user report 2026-10-11). */
export function rowsThatFit({ budget, row, heading, starts, min }: RowFit): number {
  const fewest = Math.min(min, starts.length);
  if (!(row > 0) || !Number.isFinite(budget)) return fewest;
  let used = 0;
  let fit = 0;
  for (const opens of starts) {
    used += row + (opens ? heading : 0);
    if (used > budget) break;
    fit += 1;
  }
  return Math.max(fewest, fit);
}

/** The element whose content scrolls the page: the nearest ancestor that scrolls vertically. */
function scrollArea(page: HTMLElement): HTMLElement | null {
  for (let el = page.parentElement; el !== null; el = el.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(el).overflowY)) return el;
  }
  return page.parentElement;
}

const height = (el: Element | null | undefined) => el?.getBoundingClientRect().height ?? 0;

/** `rowsThatFit` for the list `list` on the page `page`, measured after layout and again whenever
 *  the window, the scroll area or anything on the page changes size. The heights are rounded up
 *  and the budget down, with a pixel to spare: a page that ran a fraction of a pixel past the
 *  bottom would bring up a scroll bar, whose width could rewrap the page and start over. */
export function useRowsThatFit(
  page: RefObject<HTMLElement | null>,
  list: RefObject<HTMLElement | null>,
  min: number,
  starts: readonly boolean[],
): number {
  const [rows, setRows] = useState(() => Math.min(min, starts.length));
  // The days the entries open, as a value: a new array every render must not measure again.
  const days = starts.map((opens) => (opens ? "1" : "0")).join("");
  useLayoutEffect(() => {
    const root = page.current;
    const area = root === null ? null : scrollArea(root);
    if (root === null || area === null) return undefined;
    const opens = Array.from(days, (day) => day === "1");
    const measure = () => {
      const box = list.current;
      const rest = height(root) - height(box);
      setRows(
        rowsThatFit({
          budget: Math.floor(area.clientHeight - rest) - 1,
          row: Math.ceil(height(box?.querySelector(RECENT_ROW_SELECTOR))),
          heading: Math.ceil(height(box?.querySelector(RECENT_DAY_SELECTOR))),
          starts: opens,
          min,
        }),
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(area);
    observer.observe(root);
    return () => {
      observer.disconnect();
    };
  }, [page, list, min, days]);
  return rows;
}
