import { type CSSProperties, type RefObject, useLayoutEffect, useState } from "react";
import { type Box, type PanelFit, type PanelSide, fitPanel } from "./placement";

/** The part of the window that `el` can be seen in: the window less what each ancestor that clips
 *  its content (a scroll area, `overflow: hidden`) hides, scroll bars included. */
function visibleBox(el: HTMLElement): Box {
  let box: Box = { top: 0, left: 0, bottom: window.innerHeight, right: window.innerWidth };
  for (let a = el.parentElement; a !== null && a !== document.body; a = a.parentElement) {
    const style = getComputedStyle(a);
    if (style.overflowX === "visible" && style.overflowY === "visible") continue;
    const r = a.getBoundingClientRect();
    const top = r.top + a.clientTop;
    const left = r.left + a.clientLeft;
    box = {
      top: Math.max(box.top, top),
      left: Math.max(box.left, left),
      bottom: Math.min(box.bottom, top + a.clientHeight),
      right: Math.min(box.right, left + a.clientWidth),
    };
  }
  return box;
}

const same = (a: PanelFit, b: PanelFit) =>
  a.side === b.side && a.shift === b.shift && a.maxHeight === b.maxHeight;

/** `fitPanel` for the open panel `panel`, placed by its classes against `anchor` (its containing
 *  block, the wrapper around the trigger): measured before the browser paints it, and again when
 *  the window or the panel changes size or something scrolls. `null` while it is closed and while
 *  nothing is laid out (jsdom): the panel then keeps the side and edge its classes give it. */
export function useFitPanel(
  open: boolean,
  anchor: RefObject<HTMLElement | null>,
  panel: RefObject<HTMLElement | null>,
  side: PanelSide,
  align: "start" | "end",
): PanelFit | null {
  const [fit, setFit] = useState<PanelFit | null>(null);
  useLayoutEffect(() => {
    if (!open) return undefined;
    const measure = () => {
      const box = anchor.current;
      const el = panel.current;
      if (box === null || el === null) return;
      // The panel's own size, whatever shift or cap it has already been given: its layout width,
      // and the height of all its content with the borders.
      const width = el.offsetWidth;
      const height = el.scrollHeight + el.offsetHeight - el.clientHeight;
      if (width === 0 && height === 0) return;
      const trigger = box.getBoundingClientRect();
      const next = fitPanel({
        trigger,
        panel: { width, height },
        left: align === "end" ? trigger.right - width : trigger.left,
        bounds: visibleBox(box),
        side,
      });
      // The same answer keeps the same object, so a cap that resizes the panel ends here.
      setFit((was) => (was !== null && same(was, next) ? was : next));
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    const observer =
      typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    if (panel.current !== null) observer?.observe(panel.current);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
      observer?.disconnect();
      setFit(null);
    };
  }, [open, anchor, panel, side, align]);
  return open ? fit : null;
}

/** The inline style that applies `fit` to the panel: the cap on its height and the sideways
 *  shift. */
export function fitStyle(fit: PanelFit | null): CSSProperties | undefined {
  if (fit === null) return undefined;
  return {
    maxHeight: fit.maxHeight,
    transform: fit.shift === 0 ? undefined : `translateX(${fit.shift}px)`,
  };
}
