// The app mark 「声波光标」 (2026-10-08) on its 1024 canvas, and its fit to the pixels of one size.
// `@voltip/ui`'s Logo and the React Native app's draw it; the icon files come from the same
// geometry (scripts/render-icons.py) and so does the tray icon (crates/voltip-platform/src/tray.rs).

/** The sound bars on the 1024 canvas: left edge and height; each is 84 wide, centred on y 512. */
export const LOGO_BARS: ReadonlyArray<readonly [x: number, height: number]> = [
  [244, 220],
  [376, 420],
  [508, 300],
];
const BAR_WIDTH = 84;
const BAR_GAP = 48;
const CURSOR_LEFT = 708;
const CURSOR_WIDTH = 72;
const CURSOR_GAP = 116;
const CURSOR_HEIGHT = 560;
/** The bars' heights and the cursor's, shortest first. */
const HEIGHTS_IN_ORDER = [220, 300, 420, CURSOR_HEIGHT];
/** Below this the mark is drawn as designed: a pixel per bar would make it too heavy. */
const FIT_FROM = 16;

/** A bar or the cursor, in pixels of the mark's size. */
export interface LogoPart {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The whole numbers either side of `target`, at least 1. */
function near(target: number): number[] {
  const low = Math.max(1, Math.floor(target));
  const high = Math.max(1, Math.ceil(target));
  return low === high ? [low] : [low, high];
}

/** The bars and the cursor of a `size` px mark, fitted to whole pixels as `fit` in
 *  `scripts/render-icons.py` fits the app icon and `Mark::fitted` the tray icon: the bar width,
 *  the gaps and the heights are whole pixels, every straight edge lies on a pixel boundary and
 *  each part stays centred, so a two-pixel bar is white instead of a grey smear (user
 *  2026-10-08: 清晰度要提高下). Only the round ends are anti-aliased. At 1024 px it is the design
 *  itself; below 16 px, or at a fractional size, it is the design scaled. */
export function fitLogo(size: number): { bars: LogoPart[]; cursor: LogoPart } {
  const px = size / 1024;
  if (!Number.isInteger(size) || size < FIT_FROM) {
    const part = (x: number, width: number, height: number) => ({
      x: x * px,
      y: (512 - height / 2) * px,
      width: width * px,
      height: height * px,
    });
    return {
      bars: LOGO_BARS.map(([x, height]) => part(x, BAR_WIDTH, height)),
      cursor: part(CURSOR_LEFT, CURSOR_WIDTH, CURSOR_HEIGHT),
    };
  }
  const [wT, gT, cT, g2T] = [BAR_WIDTH * px, BAR_GAP * px, CURSOR_WIDTH * px, CURSOR_GAP * px];
  const span = (3 * BAR_WIDTH + 2 * BAR_GAP + CURSOR_GAP + CURSOR_WIDTH) * px;
  let best:
    | { cost: number; w: number; g: number; wc: number; g2: number; total: number }
    | undefined;
  for (const w of near(wT)) {
    for (const g of near(gT)) {
      for (const wc of near(cT)) {
        if (wc > w) continue;
        const low = Math.max(g + 1, Math.floor(g2T) - 1);
        for (let g2 = low; g2 <= Math.max(low + 1, Math.ceil(g2T) + 1); g2++) {
          const total = 3 * w + 2 * g + g2 + wc;
          // It could not sit centred on whole pixels.
          if ((size - total) % 2 !== 0) continue;
          // The bar width against its target, the gap and the cursor's width as shares of the bar
          // width (what the eye compares), the cursor's distance, and the whole span.
          const cost =
            3 * ((w - wT) / wT) ** 2 +
            2 * ((g / w - BAR_GAP / BAR_WIDTH) / (BAR_GAP / BAR_WIDTH)) ** 2 +
            ((wc / w - CURSOR_WIDTH / BAR_WIDTH) / (CURSOR_WIDTH / BAR_WIDTH)) ** 2 +
            ((g2 - g2T) / g2T) ** 2 +
            8 * ((total - span) / span) ** 2;
          if (!best || cost < best.cost) best = { cost, w, g, wc, g2, total };
        }
      }
    }
  }
  // Some cursor gap of the two or more tried always has the size's parity.
  const { w, g, wc, g2, total } = best ?? { w: 1, g: 1, wc: 1, g2: 2, total: 8 };
  // Heights take the size's parity, so each part is centred on whole pixels, and keep their order:
  // short bar < middle bar < tall bar < cursor.
  const order = HEIGHTS_IN_ORDER;
  const floors = [w, w, w, wc];
  const options = order.map((height, i) => {
    const t = Math.floor(height * px);
    const values: number[] = [];
    for (let v = Math.max(floors[i] ?? 1, t - 4); v <= t + 5; v++)
      if ((v - size) % 2 === 0) values.push(v);
    return values;
  });
  let bestHeights: { cost: number; heights: number[] } | undefined;
  const none: number[] = [];
  const [shortest = none, middle = none, tallest = none, cursorHeights = none] = options;
  for (const a of shortest) {
    for (const b of middle.filter((v: number) => v > a)) {
      for (const c of tallest.filter((v: number) => v > b)) {
        for (const d of cursorHeights.filter((v: number) => v > c)) {
          const heights = [a, b, c, d];
          const cost = heights.reduce(
            (sum, h, i) => sum + ((h - (order[i] ?? 1) * px) / ((order[i] ?? 1) * px)) ** 2,
            0,
          );
          if (!bestHeights || cost < bestHeights.cost) bestHeights = { cost, heights };
        }
      }
    }
  }
  const heights = bestHeights?.heights ?? [w, w + 2, w + 4, w + 6];
  const heightOf = (design: number) => heights[order.indexOf(design)] ?? w;
  const left = (size - total) / 2;
  const part = (x: number, width: number, height: number) => ({
    x,
    y: (size - height) / 2,
    width,
    height,
  });
  return {
    bars: LOGO_BARS.map(([, height], i) => part(left + i * (w + g), w, heightOf(height))),
    cursor: part(left + 3 * w + 2 * g + g2, wc, heightOf(CURSOR_HEIGHT)),
  };
}
