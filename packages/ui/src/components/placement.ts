// Where a list of options opens next to its trigger, as pure arithmetic (no DOM): the touch
// `Select` measures, calls these, and applies the result to a `position: fixed` list.

/** Between the trigger and the list (`Menu`'s `mt-1`). */
export const LIST_GAP = 4;
/** Kept free between the list and the screen's edges. */
export const LIST_MARGIN = 8;
/** The tallest the list grows before it scrolls: seven 44 px rows and the padding. */
export const LIST_MAX_HEIGHT = 320;

/** A box in viewport coordinates, as `getBoundingClientRect()` reports it. */
export interface Box {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface ListPlacementInput {
  trigger: Box;
  viewport: Size;
  /** The list's natural size: as wide as its longest row, as tall as all its rows. */
  content: Size;
  gap?: number;
  margin?: number;
  maxHeight?: number;
}

/** The CSS of a `position: fixed` list. Opened below it has a `top`; opened above it has a
 *  `bottom` instead, so it grows upwards from the trigger whatever height it ends up with. The
 *  width is the content's, between `minWidth` (the trigger's) and `maxWidth` (the screen's). */
export type ListPlacement = {
  left: number;
  minWidth: number;
  maxWidth: number;
  maxHeight: number;
} & ({ side: "below"; top: number } | { side: "above"; bottom: number });

/** Places the list under the trigger when it fits there, else on the side with more room (so it
 *  flips above a trigger near the bottom of the screen), never past the screen's edges: it is at
 *  most as tall as that room and `maxHeight`, and scrolls inside. It starts at the trigger's left
 *  edge, or ends at its right edge when it would not fit that way, and is shifted to stay on the
 *  screen. A trigger partly off the screen still gets a list on it. */
export function placeList({
  trigger,
  viewport,
  content,
  gap = LIST_GAP,
  margin = LIST_MARGIN,
  maxHeight = LIST_MAX_HEIGHT,
}: ListPlacementInput): ListPlacement {
  const cap = Math.max(0, Math.min(maxHeight, viewport.height - 2 * margin));
  const want = Math.min(content.height, cap);
  const top = Math.max(trigger.bottom + gap, margin);
  const roomBelow = Math.max(viewport.height - margin - top, 0);
  const bottom = Math.max(viewport.height - trigger.top + gap, margin);
  const roomAbove = Math.max(viewport.height - margin - bottom, 0);

  const triggerWidth = trigger.right - trigger.left;
  const available = Math.max(viewport.width - 2 * margin, 0);
  const width = Math.min(Math.max(content.width, triggerWidth), available);
  const startAligned = trigger.left + width <= viewport.width - margin;
  const preferred = startAligned ? trigger.left : trigger.right - width;
  const left = Math.max(margin, Math.min(preferred, viewport.width - margin - width));
  const maxWidth = Math.max(viewport.width - margin - left, 0);
  const across = { left, minWidth: Math.min(triggerWidth, maxWidth), maxWidth };

  if (want <= roomBelow || roomBelow >= roomAbove)
    return { side: "below", top, ...across, maxHeight: Math.min(cap, roomBelow) };
  return { side: "above", bottom, ...across, maxHeight: Math.min(cap, roomAbove) };
}

/** The `scrollTop` that shows a row of a scrolled list: `nearest` moves the list as little as it
 *  can (the arrow keys), `center` puts the row in the middle (the chosen row when the list opens).
 *  Never past either end of the list. */
export function revealScrollTop(
  row: { top: number; height: number },
  view: { scrollTop: number; height: number; scrollHeight: number },
  mode: "nearest" | "center",
): number {
  let next = view.scrollTop;
  if (mode === "center") next = row.top - (view.height - row.height) / 2;
  else if (row.top < view.scrollTop) next = row.top;
  else if (row.top + row.height > view.scrollTop + view.height)
    next = row.top + row.height - view.height;
  return Math.min(Math.max(next, 0), Math.max(view.scrollHeight - view.height, 0));
}
