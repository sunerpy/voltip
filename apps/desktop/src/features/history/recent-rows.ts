import { type RefObject, useLayoutEffect, useState } from "react";

export interface RowFit {
  /** Rows the table shows now. */
  shown: number;
  /** The blank space under the page in its scroll area, in px; negative when the page runs past
   *  the bottom. */
  slack: number;
  /** The height of one row, in px; 0 before anything is laid out. */
  row: number;
  /** The fewest rows to show, as long as there are that many. */
  min: number;
  /** The rows there are. */
  max: number;
}

/** How many rows the home page's recent table shows (user request 2026-10-01: a tall window left a
 *  large blank space under six rows): the rows on screen plus as many as the blank space under the
 *  page holds, fewer when the page runs past the bottom, but never under `min` (or all there are,
 *  when there are fewer) and never over `max`. Without a layout to measure, `min`. */
export function rowsThatFit({ shown, slack, row, min, max }: RowFit): number {
  const fewest = Math.min(min, max);
  if (!(row > 0)) return fewest;
  return Math.max(fewest, Math.min(max, shown + Math.floor(slack / row)));
}

/** The element whose content scrolls the page: the nearest ancestor that scrolls vertically. */
function scrollArea(page: HTMLElement): HTMLElement | null {
  for (let el = page.parentElement; el !== null; el = el.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(el).overflowY)) return el;
  }
  return page.parentElement;
}

/** `rowsThatFit` for the table inside `table` on the page `page`, measured after layout and again
 *  whenever the window, the scroll area or anything on the page changes size. */
export function useRowsThatFit(
  page: RefObject<HTMLElement | null>,
  table: RefObject<HTMLElement | null>,
  min: number,
  max: number,
): number {
  const [rows, setRows] = useState(min);
  useLayoutEffect(() => {
    const root = page.current;
    const area = root === null ? null : scrollArea(root);
    if (root === null || area === null) return undefined;
    const measure = () => {
      const shown = table.current?.querySelectorAll("tbody tr") ?? [];
      setRows(
        rowsThatFit({
          shown: shown.length,
          slack: area.clientHeight - root.offsetHeight,
          row: shown[0]?.getBoundingClientRect().height ?? 0,
          min,
          max,
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
  }, [page, table, min, max]);
  return rows;
}
