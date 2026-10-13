import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { I18nProvider } from "../i18n/I18nProvider";
import { Pagination, pageItems } from "./Pagination";

/** The accessible names of the buttons in `nav`, in order. */
function names(nav: HTMLElement): (string | null)[] {
  return within(nav)
    .getAllByRole("button")
    .map((b) => b.getAttribute("aria-label"));
}

describe("pageItems", () => {
  it("lists every page up to seven, then the first, the current one with a page on each side and the last, with gaps between", () => {
    expect(pageItems(1, 1)).toEqual([1]);
    expect(pageItems(3, 7)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(pageItems(1, 20)).toEqual([1, 2, 3, 4, 5, "gap", 20]);
    expect(pageItems(4, 20)).toEqual([1, 2, 3, 4, 5, "gap", 20]);
    expect(pageItems(5, 20)).toEqual([1, "gap", 4, 5, 6, "gap", 20]);
    expect(pageItems(16, 20)).toEqual([1, "gap", 15, 16, 17, "gap", 20]);
    expect(pageItems(17, 20)).toEqual([1, "gap", 16, 17, 18, 19, 20]);
    expect(pageItems(20, 20)).toEqual([1, "gap", 16, 17, 18, 19, 20]);
    // Seven places on every page, so the buttons do not move under the pointer, and a gap always
    // stands for two pages or more (one page is shown instead).
    for (let page = 1; page <= 20; page++) {
      const items = pageItems(page, 20);
      expect(items).toHaveLength(7);
      items.forEach((item, i) => {
        if (item !== "gap") return;
        const before = items[i - 1];
        const after = items[i + 1];
        if (typeof before !== "number" || typeof after !== "number") throw new Error("gap");
        expect(after - before).toBeGreaterThan(2);
      });
    }
    // A page outside the range reads as the nearest end; no pages reads as one.
    expect(pageItems(0, 3)).toEqual([1, 2, 3]);
    expect(pageItems(9, 9)).toEqual(pageItems(40, 9));
    expect(pageItems(1, 0)).toEqual([1]);
  });
});

describe("Pagination", () => {
  it("names every page, marks the current one, and goes to the first, the previous, a numbered, the next and the last page", async () => {
    const user = userEvent.setup();
    const onPage = vi.fn();
    render(<Pagination page={5} pages={20} onPage={onPage} />);
    const nav = screen.getByRole("navigation", { name: "分页" });
    expect(names(nav)).toEqual([
      "第一页",
      "上一页",
      "第 1 页",
      "第 4 页",
      "第 5 页",
      "第 6 页",
      "第 20 页",
      "下一页",
      "最后一页",
    ]);
    const current = within(nav).getByRole("button", { name: "第 5 页" });
    expect(current).toHaveAttribute("aria-current", "page");
    expect(current).toHaveClass("bg-accent-soft", "text-accent-text");
    expect(within(nav).getByRole("button", { name: "第 4 页" })).not.toHaveAttribute(
      "aria-current",
    );
    // The gaps are drawn for the eye only.
    const gaps = nav.querySelectorAll("[data-page-gap]");
    expect(gaps).toHaveLength(2);
    for (const gap of gaps) {
      expect(gap).toHaveAttribute("aria-hidden", "true");
      expect(gap).toHaveTextContent("…");
    }
    for (const [name, page] of [
      ["第一页", 1],
      ["上一页", 4],
      ["第 6 页", 6],
      ["第 20 页", 20],
      ["下一页", 6],
      ["最后一页", 20],
    ] as const) {
      await user.click(within(nav).getByRole("button", { name }));
      expect(onPage).toHaveBeenLastCalledWith(page);
    }
    // The page shown asks for nothing.
    onPage.mockClear();
    await user.click(current);
    expect(onPage).not.toHaveBeenCalled();
  });

  it("keeps the first, previous, next and last buttons focusable at the ends, so the keyboard does not lose its place, but they do nothing there", async () => {
    const user = userEvent.setup();
    const onPage = vi.fn();
    const { rerender } = render(<Pagination page={1} pages={3} onPage={onPage} />);
    for (const name of ["第一页", "上一页"]) {
      const button = screen.getByRole("button", { name });
      expect(button).toHaveAttribute("aria-disabled", "true");
      expect(button).toBeEnabled();
      await user.click(button);
    }
    expect(onPage).not.toHaveBeenCalled();
    // Tab reaches the pages; Enter and Space turn them.
    const next = screen.getByRole("button", { name: "下一页" });
    expect(next).not.toHaveAttribute("aria-disabled");
    next.focus();
    await user.keyboard("{Enter}");
    expect(onPage).toHaveBeenLastCalledWith(2);
    await user.keyboard(" ");
    expect(onPage).toHaveBeenCalledTimes(2);
    rerender(<Pagination page={3} pages={3} onPage={onPage} />);
    // The same button, now at the end: still focused, and it asks for nothing.
    expect(next).toHaveFocus();
    expect(next).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("button", { name: "最后一页" }));
    expect(onPage).toHaveBeenCalledTimes(2);
    await user.click(screen.getByRole("button", { name: "第 1 页" }));
    expect(onPage).toHaveBeenLastCalledWith(1);
  });

  it("hides the first and last buttons in a narrow container, where the first and last page numbers still are", () => {
    render(<Pagination page={2} pages={9} onPage={() => undefined} className="extra" />);
    const nav = screen.getByRole("navigation", { name: "分页" });
    expect(nav).toHaveClass("@container", "extra");
    for (const name of ["第一页", "最后一页"]) {
      expect(screen.getByRole("button", { name })).toHaveClass("@max-[19rem]:hidden");
    }
    for (const name of ["上一页", "下一页", "第 1 页", "第 9 页"]) {
      expect(screen.getByRole("button", { name }).className).not.toContain("hidden");
    }
  });

  it("speaks the interface language and groups long page numbers", () => {
    render(
      <I18nProvider locale="en">
        <Pagination page={1200} pages={1250} onPage={() => undefined} />
      </I18nProvider>,
    );
    const nav = screen.getByRole("navigation", { name: "Pages" });
    expect(names(nav)).toEqual([
      "First page",
      "Previous page",
      "Page 1",
      "Page 1,199",
      "Page 1,200",
      "Page 1,201",
      "Page 1,250",
      "Next page",
      "Last page",
    ]);
    expect(within(nav).getByRole("button", { name: "Page 1,200" })).toHaveTextContent("1,200");
  });
});
