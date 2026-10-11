import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog } from "./Dialog";
import { Popover } from "./Popover";

/** A box as `getBoundingClientRect()` reports it. */
function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    top,
    left,
    width,
    height,
    bottom: top + height,
    right: left + width,
    x: left,
    y: top,
    toJSON: () => ({}),
  };
}

/** jsdom lays nothing out: a window of `size`, a box for the wrapper around the trigger (what the
 *  panel is placed against) and a size for the panel. */
function fakeLayout(
  size: { width: number; height: number },
  anchor: () => Element | null,
  box: DOMRect,
  panel: { width: number; height: number },
) {
  vi.spyOn(window, "innerWidth", "get").mockImplementation(() => size.width);
  vi.spyOn(window, "innerHeight", "get").mockImplementation(() => size.height);
  const isPanel = (el: Element) => el.getAttribute("role") === "dialog";
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return this === anchor() ? box : rect(0, 0, 0, 0);
  });
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return isPanel(this) ? panel.width : 0;
  });
  vi.spyOn(Element.prototype, "scrollHeight", "get").mockImplementation(function (this: Element) {
    return isPanel(this) ? panel.height : 0;
  });
}

describe("Popover", () => {
  it("opens and closes from its button, and closes on Esc (focus back) and on a click elsewhere", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Popover trigger="依据" label="节省时间的依据" data-testid="basis">
          节省时间 = 说话时长 × 1.9。
        </Popover>
        <button type="button">elsewhere</button>
      </>,
    );
    const button = screen.getByRole("button", { name: "节省时间的依据" });
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
    const panel = screen.getByRole("dialog", { name: "节省时间的依据" });
    expect(panel).toHaveTextContent("说话时长 × 1.9");
    expect(button).toHaveAttribute("aria-controls", panel.id);
    await user.click(button);
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(button);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(button).toHaveFocus();
    await user.click(button);
    await user.click(screen.getByRole("button", { name: "elsewhere" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Esc inside a dialog closes the panel only", async () => {
    const user = userEvent.setup();
    const closeDialog = vi.fn();
    render(
      <Dialog open title="统计" onClose={closeDialog} actions={<button type="button">ok</button>}>
        <Popover trigger="依据" label="依据">
          正文
        </Popover>
      </Dialog>,
    );
    await user.click(screen.getByRole("button", { name: "依据" }));
    expect(screen.getByRole("dialog", { name: "依据" })).toBeInTheDocument();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "依据" })).toBeNull();
    expect(closeDialog).not.toHaveBeenCalled();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(closeDialog).toHaveBeenCalledTimes(1);
  });

  // User report 2026-10-11: in a small window the 依据 explanation ran on one line past the window
  // and the screen. The panel took `white-space: nowrap` from the title bar's subtitle it opens
  // from, and nothing kept it inside the window.
  it("regression: the explanation wraps inside its panel and the panel stays in the window, whatever the trigger sits in", async () => {
    const user = userEvent.setup();
    render(
      <div className="whitespace-nowrap">
        <Popover trigger="依据" label="节省时间的依据" data-testid="basis">
          节省时间 = 说话时长 × 1.9。依据：Ruan 等
          2016（arXiv:1608.07323）的实验里，说话输入比手机打字快约 2.9 倍。
        </Popover>
      </div>,
    );
    const button = screen.getByRole("button", { name: "节省时间的依据" });
    // The smallest main window; the trigger 800 px in, so the 280 px panel lined up with it would
    // end at 1080.
    const size = { width: 960, height: 600 };
    fakeLayout(size, () => button.parentElement, rect(800, 77, 30, 20), { width: 280, height: 93 });
    await user.click(button);
    const panel = screen.getByRole("dialog", { name: "节省时间的依据" });
    expect(panel).toHaveClass("whitespace-normal", "break-words", "text-left");
    // Back by 128 px: it ends 8 px inside the window's right edge, under the trigger.
    expect(panel).toHaveClass("top-full");
    expect(panel.style.transform).toBe("translateX(-128px)");
    expect(panel.style.maxHeight).toBe(`${600 - 8 - (97 + 4)}px`);
    // The window changes size while it is open: the panel follows.
    size.width = 1000;
    fireEvent(window, new Event("resize"));
    expect(panel.style.transform).toBe("translateX(-88px)");
    // Room enough: no shift.
    size.width = 1200;
    fireEvent(window, new Event("resize"));
    expect(panel.style.transform).toBe("");
  });
});
