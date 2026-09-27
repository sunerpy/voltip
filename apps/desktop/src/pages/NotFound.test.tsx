import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

/** Fixed pixel panel sizes and two-fixed-column grids broke the 1440 / 1920 px windows (Windows
 *  test 2026-09-24); `max-w-[…]` / `min-w-[…]` caps stay allowed (the root is `max-w-[1440px]`). */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

describe("NotFound page", () => {
  it("names the missing path and 回到首页 navigates home", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/nope/really" });
    expect(await screen.findByText("没有这个页面")).toBeInTheDocument();
    expect(screen.getByText("/nope/really")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "回到首页" }));
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
  });

  it("regression: notfound is fluid (no fixed-width panels)", async () => {
    renderApp({ path: "/nope" });
    const page = await screen.findByTestId("page-notfound");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6");
    // Allow-list: none (EmptyState caps its own copy with `max-w-[480px]`, which is a cap).
    expect(fixedSizeOffenders(page)).toEqual([]);
  });
});
