import { cx } from "@voltip/ui";
import { type RefObject, useLayoutEffect, useRef } from "react";
import { scrollArea } from "../../features/history/recent-rows";

/** The height of the page's scroll area, in px, kept on the page by `useSplitHeight`. */
export const SCROLL_AREA_VAR = "--scroll-area-h";
/** How far down the page the two columns start, in px (the page's top padding and everything
 *  above them: the banner, the recordings no text came from), kept by `useSplitHeight`. */
export const SPLIT_TOP_VAR = "--split-top";

/** On the history page's two-column grid (user requests 2026-10-13 and 2026-10-26): from `lg` on
 *  the log and the detail fill the window under what is above them — the scroll area less the
 *  grid's top and the page's bottom padding (`p-6`), and never less than 480 px, below which a
 *  low window scrolls the page instead — side by side and equally tall, each scrolling inside
 *  itself (the log's rows above its page buttons, the detail as a whole), so a long page of the
 *  log no longer runs down the page and the detail is not a card shorter than the window. Below
 *  `lg` the two stack and grow with what they hold. */
export const SPLIT_FILLS = cx(
  "lg:h-[max(480px,calc(var(--scroll-area-h,100vh)_-_var(--split-top,0px)_-_1.5rem))]",
  "lg:grid-rows-[minmax(0,1fr)]",
);

/** Keeps `--scroll-area-h` and `--split-top` on `page`: the height of the area that scrolls it
 *  (the shell's `<main>`) and where `split` starts in it, measured after layout and again whenever
 *  the area or the page changes size (the recordings above the grid come and go), written straight
 *  to the element so a resize renders nothing. Without a layout (jsdom) both are left out and the
 *  grid falls back to the window's height. The page then ends where the area does: the grid's top
 *  is rounded up, so it never runs a fraction of a pixel past it and brings up a scroll bar. */
export function useSplitHeight(
  page: RefObject<HTMLElement | null>,
  split: RefObject<HTMLElement | null>,
): void {
  useLayoutEffect(() => {
    const root = page.current;
    const area = root === null ? null : scrollArea(root);
    if (root === null || area === null) return undefined;
    const measure = () => {
      const height = area.clientHeight;
      if (height > 0) root.style.setProperty(SCROLL_AREA_VAR, `${height}px`);
      else root.style.removeProperty(SCROLL_AREA_VAR);
      const grid = split.current;
      const top =
        grid === null ? 0 : grid.getBoundingClientRect().top - root.getBoundingClientRect().top;
      if (height > 0 && top > 0) root.style.setProperty(SPLIT_TOP_VAR, `${Math.ceil(top)}px`);
      else root.style.removeProperty(SPLIT_TOP_VAR);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(area);
    observer.observe(root);
    return () => {
      observer.disconnect();
    };
  }, [page, split]);
}

/** The ref for the two-column grid: its detail (the second child, which scrolls inside itself)
 *  starts at its top whenever it shows another entry, so a long entry read to its end leaves the
 *  next one at its beginning, not where the last one ended. Another view of the same entry keeps
 *  its place. */
export function useDetailStartsAtTop(entry: string | undefined): RefObject<HTMLDivElement | null> {
  const grid = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (entry === undefined) return;
    const detail = grid.current?.children[1];
    if (detail !== undefined) detail.scrollTop = 0;
  }, [entry]);
  return grid;
}

/** The ref for the log's rows, which scroll inside the log from `lg` on: they start at their top
 *  whenever `view` changes (another page, filter, search or page size), not where the last page
 *  was read to. New entries arriving keep the view, and the place. */
export function useRowsStartAtTop(view: string): RefObject<HTMLUListElement | null> {
  const rows = useRef<HTMLUListElement>(null);
  const shown = useRef(view);
  useLayoutEffect(() => {
    if (shown.current === view) return;
    shown.current = view;
    if (rows.current !== null) rows.current.scrollTop = 0;
  }, [view]);
  return rows;
}

/** Brings the top of `element` into view when the page has been scrolled past it (another page
 *  chosen at the foot of a long one); its `scroll-margin-top` sets where it stops. */
export function revealTop(element: HTMLElement | null): void {
  if (element === null || typeof element.scrollIntoView !== "function") return;
  const area = scrollArea(element);
  const top = area === null ? 0 : area.getBoundingClientRect().top;
  if (element.getBoundingClientRect().top < top) element.scrollIntoView({ block: "start" });
}
