import { rowsThatFit } from "./recent-rows";

const ROW = 26;

describe("rowsThatFit", () => {
  it("regression: a tall window gets the rows its blank space holds, a short one gives them back down to six (user request 2026-10-01)", () => {
    // 1920 × 1080: 262 px under six rows hold ten more.
    expect(rowsThatFit({ shown: 6, slack: 262, row: ROW, min: 6, max: 30 })).toBe(16);
    // Once they are there, less than a row is left: it stays.
    expect(rowsThatFit({ shown: 16, slack: 2, row: ROW, min: 6, max: 30 })).toBe(16);
    expect(rowsThatFit({ shown: 16, slack: ROW - 1, row: ROW, min: 6, max: 30 })).toBe(16);
    // The window got shorter and the page runs 10 px past the bottom: one row goes.
    expect(rowsThatFit({ shown: 16, slack: -10, row: ROW, min: 6, max: 30 })).toBe(15);
    // Much shorter: never under six, the page scrolls as it did before.
    expect(rowsThatFit({ shown: 16, slack: -900, row: ROW, min: 6, max: 30 })).toBe(6);
  });

  it("shows no more rows than there are entries, and all of them when there are fewer than six", () => {
    expect(rowsThatFit({ shown: 16, slack: 2000, row: ROW, min: 6, max: 30 })).toBe(30);
    expect(rowsThatFit({ shown: 3, slack: 500, row: ROW, min: 6, max: 3 })).toBe(3);
    expect(rowsThatFit({ shown: 0, slack: 500, row: 0, min: 6, max: 0 })).toBe(0);
  });

  it("keeps six rows while nothing is laid out to measure", () => {
    expect(rowsThatFit({ shown: 0, slack: 0, row: 0, min: 6, max: 30 })).toBe(6);
    expect(rowsThatFit({ shown: 6, slack: 400, row: Number.NaN, min: 6, max: 30 })).toBe(6);
  });
});
