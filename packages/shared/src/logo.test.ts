import { describe, expect, it } from "vitest";
import { LOGO_BARS, fitLogo } from "./logo";

/** x, y, width, height of each bar and then the cursor. */
function parts(size: number): number[][] {
  const { bars, cursor } = fitLogo(size);
  return [...bars, cursor].map(({ x, y, width, height }) => [x, y, width, height]);
}

describe("fitLogo", () => {
  it("is the design at 1024 px", () => {
    expect(parts(1024)).toEqual([
      ...LOGO_BARS.map(([x, height]) => [x, 512 - height / 2, 84, height]),
      [708, 232, 72, 560],
    ]);
  });

  it("regression: at the sidebar's 24 px the bars and the cursor sit on whole pixels", () => {
    // User 2026-10-08 (清晰度要提高下): drawn as designed, a 24 px bar is 1.97 px wide with its
    // edges between pixels, and the browser smears it into grey.
    expect(parts(24)).toEqual([
      [6, 9, 2, 6],
      [9, 7, 2, 10],
      [12, 8, 2, 8],
      [16, 5, 2, 14],
    ]);
  });

  it("fits every size from 16 px to whole pixels, centred and in the mark's order", () => {
    // The fits scripts/render-icons.py prints (`--fit 16 32 48`); the tray (Mark::fitted) agrees.
    expect(parts(16)).toEqual([
      [4, 6, 1, 4],
      [6, 4, 1, 8],
      [8, 5, 1, 6],
      [11, 3, 1, 10],
    ]);
    expect(parts(32)).toEqual([
      [7, 13, 3, 6],
      [12, 9, 3, 14],
      [17, 11, 3, 10],
      [23, 7, 2, 18],
    ]);
    expect(parts(48)).toEqual([
      [12, 19, 4, 10],
      [18, 14, 4, 20],
      [24, 17, 4, 14],
      [33, 11, 3, 26],
    ]);
    for (let size = 16; size <= 256; size++) {
      const all = parts(size);
      for (const [x = 0, y = 0, width = 0, height = 0] of all) {
        expect([x, y, width, height].every(Number.isInteger)).toBe(true);
        expect(y + height / 2).toBe(size / 2);
        expect(width).toBeGreaterThanOrEqual(1);
      }
      const [first = [], second = [], third = [], cursor = []] = all;
      // Left to right with at least a pixel between, centred as a whole.
      expect((second[0] ?? 0) - (first[0] ?? 0) - (first[2] ?? 0)).toBeGreaterThanOrEqual(1);
      expect((cursor[0] ?? 0) - (third[0] ?? 0) - (third[2] ?? 0)).toBeGreaterThan(1);
      expect((first[0] ?? 0) + (cursor[0] ?? 0) + (cursor[2] ?? 0)).toBe(size);
      // Short, tall, middle; the cursor tallest and no wider than a bar.
      const [h1 = 0, h2 = 0, h3 = 0, hc = 0] = all.map((part) => part[3] ?? 0);
      expect(h1 < h3 && h3 < h2 && h2 < hc).toBe(true);
      expect(cursor[2] ?? 0).toBeLessThanOrEqual(first[2] ?? 0);
    }
  });

  it("draws the design scaled below 16 px and at fractional sizes", () => {
    for (const size of [12, 20.5]) {
      const { bars, cursor } = fitLogo(size);
      const k = size / 1024;
      expect(bars.map(({ x, width, height }) => [x / k, width / k, height / k])).toEqual(
        LOGO_BARS.map(([x, height]) => [x, 84, height]),
      );
      expect([cursor.x / k, cursor.y / k, cursor.width / k, cursor.height / k]).toEqual([
        708, 232, 72, 560,
      ]);
    }
  });
});
