import { LIST_MAX_HEIGHT, placeList, revealScrollTop } from "./placement";

// A 375 × 667 phone screen and a trigger 28 px tall, 200 px wide.
const VIEWPORT = { width: 375, height: 667 };
const trigger = (top: number, left = 16, width = 200) => ({
  top,
  bottom: top + 28,
  left,
  right: left + width,
});
// Five 44 px rows and 4 px of padding above and below.
const FIVE_ROWS = { width: 160, height: 228 };

describe("placeList", () => {
  it("opens under the trigger when the list fits there, as wide as the trigger at least", () => {
    expect(placeList({ trigger: trigger(100), viewport: VIEWPORT, content: FIVE_ROWS })).toEqual({
      side: "below",
      top: 132,
      left: 16,
      minWidth: 200,
      maxWidth: 351,
      maxHeight: LIST_MAX_HEIGHT,
    });
  });

  it("flips above the trigger when there is no room below, and grows upwards from it", () => {
    const placed = placeList({ trigger: trigger(560), viewport: VIEWPORT, content: FIVE_ROWS });
    // 667 - 560 + 4: the list's bottom edge sits 4 px above the trigger.
    expect(placed).toMatchObject({ side: "above", bottom: 111, maxHeight: LIST_MAX_HEIGHT });
    expect(placed).not.toHaveProperty("top");
  });

  it("takes the side with more room when the list fits on neither, and scrolls inside it", () => {
    const tall = { width: 160, height: 900 };
    // 296 px of room above the trigger, 319 below (under the 320 px cap): below, that tall.
    expect(placeList({ trigger: trigger(308), viewport: VIEWPORT, content: tall })).toMatchObject({
      side: "below",
      top: 340,
      maxHeight: 319,
    });
    // With room enough the cap is the limit.
    expect(placeList({ trigger: trigger(100), viewport: VIEWPORT, content: tall })).toMatchObject({
      side: "below",
      maxHeight: LIST_MAX_HEIGHT,
    });
    // Without the cap the room itself is the limit: never past the screen's edge.
    expect(
      placeList({ trigger: trigger(308), viewport: VIEWPORT, content: tall, maxHeight: 1000 }),
    ).toMatchObject({ side: "below", top: 340, maxHeight: 667 - 8 - 340 });
    // Lower on the screen there is more room above.
    expect(
      placeList({ trigger: trigger(420), viewport: VIEWPORT, content: tall, maxHeight: 1000 }),
    ).toMatchObject({ side: "above", bottom: 251, maxHeight: 667 - 8 - 251 });
  });

  it("keeps the list on a short screen and a trigger partly off it on the screen", () => {
    // A landscape phone with the keyboard up: no room above, 156 px below.
    const short = { width: 640, height: 200 };
    expect(placeList({ trigger: trigger(4), viewport: short, content: FIVE_ROWS })).toMatchObject({
      side: "below",
      top: 36,
      maxHeight: 156,
    });
    // Scrolled half out at the top: the list starts at the margin, not off the screen.
    expect(
      placeList({ trigger: trigger(-20), viewport: VIEWPORT, content: FIVE_ROWS }),
    ).toMatchObject({ side: "below", top: 12 });
    expect(
      placeList({ trigger: trigger(-60), viewport: VIEWPORT, content: FIVE_ROWS }),
    ).toMatchObject({ side: "below", top: 8 });
    // Below the screen: above it, its bottom edge at the margin.
    expect(
      placeList({ trigger: trigger(700), viewport: VIEWPORT, content: FIVE_ROWS }),
    ).toMatchObject({ side: "above", bottom: 8, maxHeight: LIST_MAX_HEIGHT });
  });

  it("ends at the trigger's right edge when it would not fit from its left edge, and never leaves the screen sideways", () => {
    // A narrow trigger at the right of a row (保留最近 · 2,000 条): the list lines up with its end.
    const right = trigger(100, 291, 68);
    expect(
      placeList({ trigger: right, viewport: VIEWPORT, content: { width: 180, height: 120 } }),
    ).toMatchObject({ left: 179, minWidth: 68, maxWidth: 188 });
    // Wider than the screen: from margin to margin, the rows wrap.
    expect(
      placeList({ trigger: right, viewport: VIEWPORT, content: { width: 600, height: 120 } }),
    ).toMatchObject({ left: 8, minWidth: 68, maxWidth: 359 });
    // A trigger hanging off the left edge.
    expect(
      placeList({
        trigger: trigger(100, -30, 120),
        viewport: VIEWPORT,
        content: { width: 100, height: 120 },
      }),
    ).toMatchObject({ left: 8, minWidth: 120, maxWidth: 359 });
  });

  it("honours a custom gap and margin", () => {
    expect(
      placeList({
        trigger: trigger(100),
        viewport: VIEWPORT,
        content: FIVE_ROWS,
        gap: 8,
        margin: 16,
      }),
    ).toMatchObject({ side: "below", top: 136, left: 16, maxWidth: 343 });
  });
});

describe("revealScrollTop", () => {
  const view = { scrollTop: 88, height: 132, scrollHeight: 400 };

  it("moves a scrolled list as little as it can to show a row", () => {
    // Already in view: stays.
    expect(revealScrollTop({ top: 132, height: 44 }, view, "nearest")).toBe(88);
    // Above the view: its top at the top.
    expect(revealScrollTop({ top: 44, height: 44 }, view, "nearest")).toBe(44);
    // Below the view: its bottom at the bottom.
    expect(revealScrollTop({ top: 264, height: 44 }, view, "nearest")).toBe(176);
  });

  it("centres a row, but never scrolls past either end of the list", () => {
    expect(revealScrollTop({ top: 176, height: 44 }, view, "center")).toBe(132);
    expect(revealScrollTop({ top: 4, height: 44 }, view, "center")).toBe(0);
    expect(revealScrollTop({ top: 352, height: 44 }, view, "center")).toBe(268);
    // A list that does not scroll stays at 0.
    expect(
      revealScrollTop(
        { top: 184, height: 44 },
        { scrollTop: 0, height: 228, scrollHeight: 228 },
        "center",
      ),
    ).toBe(0);
  });
});
