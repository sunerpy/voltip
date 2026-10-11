import { rowsThatFit } from "./recent-rows";

const ROW = 60;
const HEADING = 28;

/** Entries, newest first, of which those at `starts` open a new day. */
function days(count: number, ...starts: number[]): boolean[] {
  return Array.from({ length: count }, (_, i) => i === 0 || starts.includes(i));
}

describe("rowsThatFit", () => {
  it("regression: a tall window gets the rows its share of the window holds, a short one gives them back down to four (user request 2026-10-01)", () => {
    const today = days(30);
    // One day: a heading, then rows.
    expect(
      rowsThatFit({
        budget: HEADING + 16 * ROW,
        row: ROW,
        heading: HEADING,
        starts: today,
        min: 4,
      }),
    ).toBe(16);
    // A pixel short of the sixteenth row: fifteen.
    expect(
      rowsThatFit({
        budget: HEADING + 16 * ROW - 1,
        row: ROW,
        heading: HEADING,
        starts: today,
        min: 4,
      }),
    ).toBe(15);
    // Much shorter: never under four, the page scrolls as it did before.
    expect(rowsThatFit({ budget: 100, row: ROW, heading: HEADING, starts: today, min: 4 })).toBe(4);
    expect(rowsThatFit({ budget: -500, row: ROW, heading: HEADING, starts: today, min: 4 })).toBe(
      4,
    );
  });

  // User report 2026-10-11: a maximized window flickered for ever between 16 and 17 rows. Counting
  // on from the rows on screen, room for one more row added the 17th, which opened 昨天: its
  // heading took the page past the bottom, the row went, the heading with it, and there was room
  // again. The heading a row brings is part of what it needs.
  it("regression: a row that opens a new day needs room for its heading too, so the answer never flips at a day boundary", () => {
    const twoDays = days(30, 16);
    // Room for sixteen rows and 70 px more: a 17th row alone would fit, with 昨天 above it not.
    const budget = HEADING + 16 * ROW + 70;
    expect(rowsThatFit({ budget, row: ROW, heading: HEADING, starts: twoDays, min: 4 })).toBe(16);
    // With 88 px to spare the row and its heading fit.
    expect(
      rowsThatFit({
        budget: HEADING + 16 * ROW + ROW + HEADING,
        row: ROW,
        heading: HEADING,
        starts: twoDays,
        min: 4,
      }),
    ).toBe(17);
    // The answer is a function of the window and the entries alone: the same at every budget
    // whatever the list showed before, so showing it cannot change it.
    for (let b = 0; b < 2400; b += 7) {
      const first = rowsThatFit({ budget: b, row: ROW, heading: HEADING, starts: twoDays, min: 4 });
      expect(rowsThatFit({ budget: b, row: ROW, heading: HEADING, starts: twoDays, min: 4 })).toBe(
        first,
      );
    }
  });

  it("shows no more rows than there are entries, and all of them when there are fewer than four", () => {
    expect(
      rowsThatFit({ budget: 5000, row: ROW, heading: HEADING, starts: days(30), min: 4 }),
    ).toBe(30);
    expect(rowsThatFit({ budget: 5000, row: ROW, heading: HEADING, starts: days(3), min: 4 })).toBe(
      3,
    );
    expect(rowsThatFit({ budget: 5000, row: ROW, heading: HEADING, starts: [], min: 4 })).toBe(0);
  });

  it("keeps four rows while nothing is laid out to measure", () => {
    expect(rowsThatFit({ budget: 900, row: 0, heading: 0, starts: days(30), min: 4 })).toBe(4);
    expect(
      rowsThatFit({ budget: 900, row: Number.NaN, heading: HEADING, starts: days(30), min: 4 }),
    ).toBe(4);
    expect(
      rowsThatFit({ budget: Number.NaN, row: ROW, heading: HEADING, starts: days(30), min: 4 }),
    ).toBe(4);
  });
});
