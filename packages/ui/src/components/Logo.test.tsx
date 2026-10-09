import { fitLogo } from "@voltip/shared";
import { render, screen } from "@testing-library/react";
import {
  LOGO_BARS,
  LOGO_CURSOR,
  LOGO_CURSOR_TOP,
  LOGO_INK,
  LOGO_INK_END,
  LOGO_WAVE,
  Logo,
} from "./Logo";

/** x, y, width, height of each bar and then the cursor. */
function geometry(svg: Element): number[][] {
  return [...svg.querySelectorAll("rect")]
    .slice(1)
    .map((rect) => ["x", "y", "width", "height"].map((name) => Number(rect.getAttribute(name))));
}

/** Lays every element out at (left, top), plus the relative offset the mark writes on itself, as
 *  a browser's getBoundingClientRect does. */
function placeAt(left: number, top: number) {
  return vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (
    this: Element,
  ) {
    const style = (this as HTMLElement).style;
    const x = left + (Number.parseFloat(style?.left ?? "") || 0);
    const y = top + (Number.parseFloat(style?.top ?? "") || 0);
    return {
      x,
      y,
      left: x,
      top: y,
      right: x + 24,
      bottom: y + 24,
      width: 24,
      height: 24,
      toJSON: () => ({}),
    };
  });
}

describe("Logo", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("draws the app mark: an ink tile, three white sound bars and a cyan cursor", () => {
    render(<Logo size={1024} />);
    const svg = screen.getByTestId("app-logo");
    expect(svg).toHaveAttribute("width", "1024");
    expect(svg).toHaveAttribute("height", "1024");
    expect(svg).toHaveAttribute("viewBox", "0 0 1024 1024");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    const [tile, ...rest] = svg.querySelectorAll("rect");
    const bars = rest.slice(0, 3);
    const cursor = rest[3];
    expect(rest).toHaveLength(4);
    if (!tile || !cursor) throw new Error("the mark has no tile or no cursor");
    expect(tile).toHaveAttribute("rx", "232");
    // At its full size the fit is the design.
    expect(geometry(svg)).toEqual([
      ...LOGO_BARS.map(([x, height]) => [x, 512 - height / 2, 84, height]),
      [708, 232, 72, 560],
    ]);
    for (const bar of bars) {
      expect(bar).toHaveAttribute("fill", LOGO_WAVE);
      // Both ends fully round.
      expect(bar).toHaveAttribute("rx", "42");
    }
    expect(cursor).toHaveAttribute("rx", "36");
    // The tile and the cursor take their gradients from the mark's own defs.
    const gradient = (rect: Element) => {
      const ref = /^url\(#(.+)\)$/.exec(rect.getAttribute("fill") ?? "")?.[1];
      const stops = svg.querySelectorAll(`[id="${ref}"] stop`);
      return [...stops].map((stop) => stop.getAttribute("stop-color"));
    };
    expect(gradient(tile)).toEqual([LOGO_INK, LOGO_INK_END]);
    expect(gradient(cursor)).toEqual([LOGO_CURSOR_TOP, LOGO_CURSOR]);
  });

  it("regression: at the sidebar's 24 px the bars and the cursor sit on whole pixels", () => {
    // User 2026-10-08 (清晰度要提高下): drawn as designed, a 24 px bar is 1.97 px wide with its
    // edges between pixels, and the browser smears it into grey.
    render(<Logo size={24} />);
    const svg = screen.getByTestId("app-logo");
    expect(svg).toHaveAttribute("viewBox", "0 0 24 24");
    expect(geometry(svg)).toEqual([
      [6, 9, 2, 6],
      [9, 7, 2, 10],
      [12, 8, 2, 8],
      [16, 5, 2, 14],
    ]);
  });

  it("fits the device pixels it covers on a scaled screen", () => {
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1.5);
    render(<Logo size={24} />);
    const svg = screen.getByTestId("app-logo");
    // 24 CSS px are 36 device pixels at 150 %: the mark is fitted at 36 and drawn 24 px wide.
    expect(svg).toHaveAttribute("width", "24");
    expect(svg).toHaveAttribute("viewBox", "0 0 36 36");
    const { bars, cursor } = fitLogo(36);
    expect(geometry(svg)).toEqual(
      [...bars, cursor].map(({ x, y, width, height }) => [x, y, width, height]),
    );
  });

  it("regression: it moves onto whole device pixels where the rem layout puts it between two", () => {
    // At the default 14 px text size the sidebar lays the mark out at 17.5, 5.5.
    placeAt(17.5, 5.5);
    const { unmount } = render(<Logo size={24} />);
    const svg = screen.getByTestId("app-logo");
    const rect = svg.getBoundingClientRect();
    expect([rect.left, rect.top]).toEqual([18, 6]);
    unmount();
    vi.restoreAllMocks();
    // On a 150 % screen a device pixel is 2/3 px: 17.5 px is 26.25 device pixels, moved to 26.
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1.5);
    placeAt(17.5, 5.5);
    render(<Logo size={24} />);
    const scaled = screen.getByTestId("app-logo").getBoundingClientRect();
    expect(scaled.left * 1.5).toBeCloseTo(26, 6);
    expect(scaled.top * 1.5).toBeCloseTo(8, 6);
  });

  it("leaves a mark already on whole pixels where it is", () => {
    placeAt(16, 8);
    render(<Logo size={24} />);
    const svg = screen.getByTestId("app-logo");
    expect([svg.style.position, svg.style.left, svg.style.top]).toEqual(["", "", ""]);
  });

  it("gives every mark on the page gradients of its own", () => {
    render(
      <>
        <Logo />
        <Logo size={56} />
      </>,
    );
    const ids = screen
      .getAllByTestId("app-logo")
      .map((svg) => [...svg.querySelectorAll("linearGradient")].map((g) => g.id));
    expect(ids[0]).toHaveLength(2);
    expect(new Set(ids.flat()).size).toBe(4);
  });

  it("is an accessible image when labelled", () => {
    render(<Logo label="Voltip" />);
    expect(screen.getByRole("img", { name: "Voltip" })).toBeInTheDocument();
  });
});
