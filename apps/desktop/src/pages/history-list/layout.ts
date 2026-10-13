import { cx } from "@voltip/ui";
import { type RefObject, useLayoutEffect, useRef } from "react";
import { scrollArea } from "../../features/history/recent-rows";

/** The height of the page's scroll area, in px, kept on the page by `useScrollAreaHeight`. */
export const SCROLL_AREA_VAR = "--scroll-area-h";

/** On the history page's two-column grid: the detail, its second child, beside the log (user
 *  request 2026-10-13). From `lg` on it sticks 1.5rem (`top-6`, the page's `p-6`) under the top of
 *  the shell's `<main>`, the element that scrolls the page, and sits at the top of its row instead
 *  of stretching to the log's height, so a long page of the log scrolls past it with no blank card
 *  beside it. It is never taller than the scroll area less that padding above and below; its
 *  552 px floor drops to the same height in a low window, and only what does not fit scrolls,
 *  inside the card, which keeps its own edge and shadow. Set on the grid, so the detail's own
 *  markup stays as it is, whatever renders it. Below `lg` the two stack and nothing sticks. */
export const DETAIL_STICKS = cx(
  "lg:[&>:nth-child(2)]:sticky lg:[&>:nth-child(2)]:top-6 lg:[&>:nth-child(2)]:self-start",
  "lg:[&>:nth-child(2)]:max-h-[calc(var(--scroll-area-h,100vh)_-_3rem)]",
  "lg:[&>:nth-child(2)]:min-h-[min(552px,calc(var(--scroll-area-h,100vh)_-_3rem))]",
  "lg:[&>:nth-child(2)]:overflow-y-auto",
);

/** Keeps `--scroll-area-h` on `page` at the height of the area that scrolls it (the shell's
 *  `<main>`): measured after layout and again whenever that area changes size, written straight to
 *  the element so a resize renders nothing. Without a height (jsdom has no layout) the variable is
 *  left out and the detail falls back to the window's height. */
export function useScrollAreaHeight(page: RefObject<HTMLElement | null>): void {
  useLayoutEffect(() => {
    const root = page.current;
    const area = root === null ? null : scrollArea(root);
    if (root === null || area === null) return undefined;
    const measure = () => {
      const height = area.clientHeight;
      if (height > 0) root.style.setProperty(SCROLL_AREA_VAR, `${height}px`);
      else root.style.removeProperty(SCROLL_AREA_VAR);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(area);
    return () => {
      observer.disconnect();
    };
  }, [page]);
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

/** Brings the top of `element` into view when the page has been scrolled past it (another page
 *  chosen at the foot of a long one); its `scroll-margin-top` sets where it stops. */
export function revealTop(element: HTMLElement | null): void {
  if (element === null || typeof element.scrollIntoView !== "function") return;
  const area = scrollArea(element);
  const top = area === null ? 0 : area.getBoundingClientRect().top;
  if (element.getBoundingClientRect().top < top) element.scrollIntoView({ block: "start" });
}
