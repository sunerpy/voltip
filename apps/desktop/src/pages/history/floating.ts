// Where the history quick fix's floating pieces sit (docs/dictation.md §16.9): the selection
// toolbar 8 px above the selection and centred on it, the card 8 px below its anchor with its
// fields lined up with the words — each flipped to the other side when it does not fit there and
// kept inside the window (`fitPanel`). They are `position: fixed` in the page (nothing above the
// page transforms it), so the page's dialogs cover them like the rest of it.
import { type PanelSide, fitPanel } from "@voltip/ui";
import { type CSSProperties, type RefObject, useLayoutEffect, useState } from "react";

/** Between the anchor and the panel (§16.9 3.2, 3.3). */
export const FLOAT_GAP = 8;
/** Kept free between a panel and the window's edges. */
export const FLOAT_MARGIN = 8;
/** The card's fields start this far right of its edge: the card opens this far left of the words. */
export const CARD_INSET = 20;

/** A box in window coordinates, as `getBoundingClientRect()` gives it. */
export interface FloatBox {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface FloatOptions {
  side: PanelSide;
  /** `center`: centred on the anchor (the toolbar); `start`: `CARD_INSET` left of it (the card). */
  align: "center" | "start";
}

/** The fixed position of `panel` next to `anchor()`: measured before the browser paints, again
 *  when the window or the panel changes size or something scrolls. `anchor` returning `undefined`
 *  (the words went away) leaves the last position. */
export function useFloatingStyle(
  panel: RefObject<HTMLElement | null>,
  anchor: () => FloatBox | undefined,
  { side, align }: FloatOptions,
): { style: CSSProperties; side: PanelSide } {
  const [placed, setPlaced] = useState<{ style: CSSProperties; side: PanelSide }>({
    style: { position: "fixed", top: 0, left: 0, visibility: "hidden" },
    side,
  });
  useLayoutEffect(() => {
    const measure = () => {
      const el = panel.current;
      const trigger = anchor();
      if (el === null || trigger === undefined) return;
      const width = el.offsetWidth;
      const height = el.scrollHeight + el.offsetHeight - el.clientHeight;
      const left =
        align === "center"
          ? (trigger.left + trigger.right) / 2 - width / 2
          : trigger.left - CARD_INSET;
      const bounds = { top: 0, left: 0, bottom: window.innerHeight, right: window.innerWidth };
      const fit = fitPanel({
        trigger,
        panel: { width, height },
        left,
        bounds,
        side,
        gap: FLOAT_GAP,
        margin: FLOAT_MARGIN,
      });
      const shown = Math.min(height, fit.maxHeight);
      const top =
        fit.side === "bottom" ? trigger.bottom + FLOAT_GAP : trigger.top - FLOAT_GAP - shown;
      const next = { top, left: left + fit.shift, maxHeight: fit.maxHeight };
      // The same place keeps the same object: a re-render does not measure again for nothing.
      setPlaced((was) =>
        was.side === fit.side &&
        was.style.top === next.top &&
        was.style.left === next.left &&
        was.style.maxHeight === next.maxHeight &&
        was.style.visibility === undefined
          ? was
          : { style: { position: "fixed", ...next }, side: fit.side },
      );
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
    };
  }, [panel, anchor, side, align]);
  return placed;
}

/** `el`'s box while it is in the page. */
export function boxOf(el: Element | null | undefined): FloatBox | undefined {
  return el?.isConnected === true ? el.getBoundingClientRect() : undefined;
}
