import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog } from "./Dialog";

describe("Dialog", () => {
  it("renders nothing when closed and closes on Esc / scrim click", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const { rerender } = render(
      <Dialog open={false} title="t" actions={<button>x</button>} onClose={onClose} />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    rerender(
      <Dialog
        open
        title="删除全部历史记录？"
        facts="history.db · 1.8 MB"
        actions={<button data-autofocus>取消</button>}
        onClose={onClose}>
        正文
      </Dialog>,
    );
    expect(screen.getByRole("dialog", { name: "删除全部历史记录？" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "取消" })).toHaveFocus();
    expect(screen.getByText("history.db · 1.8 MB")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByTestId("dialog-scrim"));
    expect(onClose).toHaveBeenCalledTimes(2);
    await user.click(screen.getByRole("dialog"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("focuses the dialog itself without an autofocus target and shows a custom hint", () => {
    render(
      <Dialog
        open
        title="t"
        actions={<span>a</span>}
        onClose={vi.fn()}
        hint="自定义"
        width={300}
      />,
    );
    expect(screen.getByRole("dialog")).toHaveFocus();
    expect(screen.getByText("自定义")).toBeInTheDocument();
  });

  it("regression: Escape is marked consumed so an enclosing document listener does not also close", () => {
    const onClose = vi.fn();
    let outerWouldClose = 0;
    const outer = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) outerWouldClose += 1;
    };
    document.addEventListener("keydown", outer);
    try {
      render(
        <Dialog open title="Inner" onClose={onClose} actions={<button type="button">ok</button>}>
          <p>body</p>
        </Dialog>,
      );
      fireEvent.keyDown(document, { key: "Escape" });
      expect(onClose).toHaveBeenCalledTimes(1);
      expect(outerWouldClose).toBe(0);
    } finally {
      document.removeEventListener("keydown", outer);
    }
  });
});
