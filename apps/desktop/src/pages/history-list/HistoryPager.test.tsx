import { I18nProvider } from "@voltip/ui";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HistoryPager, type HistoryPagerProps } from "./HistoryPager";

function pager(props: Partial<HistoryPagerProps> = {}, locale: "zh-CN" | "en" = "zh-CN") {
  const onPage = vi.fn();
  const onSize = vi.fn();
  const all: HistoryPagerProps = {
    page: 1,
    pages: 1,
    matching: 1,
    searching: false,
    size: 50,
    onPage,
    onSize,
    ...props,
  };
  const view = render(
    <I18nProvider locale={locale}>
      <HistoryPager {...all} />
    </I18nProvider>,
  );
  return { ...view, onPage, onSize };
}

describe("the foot of the history log (user request 2026-10-13)", () => {
  it("says where the page is, counts entries or matches, and groups large numbers", () => {
    const { rerender } = pager({ page: 2, pages: 1200, matching: 23_981 });
    expect(screen.getByTestId("history-page-count")).toHaveTextContent(
      "第 2 / 1,200 页 · 共 23,981 条",
    );
    rerender(
      <I18nProvider locale="zh-CN">
        <HistoryPager
          page={1}
          pages={1}
          matching={3}
          searching
          size={50}
          onPage={() => undefined}
          onSize={() => undefined}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("history-page-count")).toHaveTextContent("第 1 / 1 页 · 找到 3 条");
    // One page: no page buttons, but the size can still be chosen.
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(screen.getByRole("combobox", { name: "每页显示条数" })).toBeInTheDocument();
  });

  it("reads in English, with one entry or match told apart from several", () => {
    const one = pager({ matching: 1 }, "en");
    expect(screen.getByTestId("history-page-count")).toHaveTextContent("Page 1 of 1 · 1 entry");
    one.unmount();
    const many = pager({ page: 3, pages: 7, matching: 320 }, "en");
    expect(screen.getByTestId("history-page-count")).toHaveTextContent("Page 3 of 7 · 320 entries");
    expect(screen.getByRole("navigation", { name: "Pages" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Entries per page" })).toHaveValue("50");
    expect(screen.getByText("Per page")).toBeInTheDocument();
    many.unmount();
    pager({ matching: 1, searching: true }, "en");
    expect(screen.getByTestId("history-page-count")).toHaveTextContent("Page 1 of 1 · 1 match");
  });

  it("hands on the page and the size chosen", async () => {
    const user = userEvent.setup();
    const { onPage, onSize } = pager({ page: 2, pages: 3, matching: 120 });
    await user.click(screen.getByRole("button", { name: "第 3 页" }));
    expect(onPage).toHaveBeenCalledWith(3);
    await user.selectOptions(screen.getByRole("combobox", { name: "每页显示条数" }), "200");
    expect(onSize).toHaveBeenCalledWith(200);
  });
});
