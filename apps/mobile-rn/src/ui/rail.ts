// The tabs on a phone on its side (docs/mobile-rn.md §5): a window wider than it is tall and shorter
// than RAIL_BELOW_HEIGHT puts the tab dock in a rail at the left. A dock along the bottom would cover
// the middle of a page barely taller than itself (2026-10-28, Device Farm: on a Pixel 9 on its side,
// 411 dp tall, the hold-to-talk button sat under the dock). A tablet keeps the dock at the bottom.

/** A window shorter than this (and wider than tall) has the rail: phones on their side are 360–430
 *  dp tall, a tablet or an unfolded phone on its side 600 dp and more. */
export const RAIL_BELOW_HEIGHT = 480;
/** The rail's width, and its distance from the left edge (and the inset there). */
export const RAIL_WIDTH = 72;
export const RAIL_GAP = 16;

export function isRailWindow(width: number, height: number): boolean {
  return width > height && height < RAIL_BELOW_HEIGHT;
}

/** Where a page's content starts beside the rail: the rail, then a page's own 16 dp. */
export function besideRail(insetLeft: number): number {
  return insetLeft + RAIL_GAP + RAIL_WIDTH + 16;
}
