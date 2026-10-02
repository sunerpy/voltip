import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog, dismissTopDialog } from "./Dialog";

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

  // Found 2026-09-29 (built-in scenes, 恢复默认): a confirmation over the scene editor was named
  // after the editor (both used one fixed title id), and Esc closed the editor under it.
  it("regression: a dialog over another one has its own name, and Esc closes only the one on top", () => {
    const closeOuter = vi.fn();
    const closeInner = vi.fn();
    const { rerender } = render(
      <>
        <Dialog
          open
          title="编辑场景"
          onClose={closeOuter}
          actions={<button type="button">a</button>}
        />
        <Dialog
          open
          title="恢复默认？"
          onClose={closeInner}
          actions={<button type="button">b</button>}
        />
      </>,
    );
    expect(screen.getByRole("dialog", { name: "编辑场景" })).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "恢复默认？" })).toBeInTheDocument();
    // A new close handler for the one underneath (a re-render) does not put it on top.
    rerender(
      <>
        <Dialog
          open
          title="编辑场景"
          onClose={() => closeOuter()}
          actions={<button type="button">a</button>}
        />
        <Dialog
          open
          title="恢复默认？"
          onClose={closeInner}
          actions={<button type="button">b</button>}
        />
      </>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect([closeInner.mock.calls.length, closeOuter.mock.calls.length]).toEqual([1, 0]);
    rerender(
      <>
        <Dialog
          open
          title="编辑场景"
          onClose={closeOuter}
          actions={<button type="button">a</button>}
        />
        <Dialog
          open={false}
          title="恢复默认？"
          onClose={closeInner}
          actions={<button type="button">b</button>}
        />
      </>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect([closeInner.mock.calls.length, closeOuter.mock.calls.length]).toEqual([1, 1]);
  });

  it("the phone's system back closes the dialog on top, as Esc does, and reports when none is open", () => {
    const closeOuter = vi.fn();
    const closeInner = vi.fn();
    const both = (inner: boolean) => (
      <>
        <Dialog
          open
          title="编辑词条"
          onClose={closeOuter}
          actions={<button type="button">a</button>}
        />
        <Dialog
          open={inner}
          title="删除词条？"
          onClose={closeInner}
          actions={<button type="button">b</button>}
        />
      </>
    );
    const { rerender, unmount } = render(both(true));
    expect(dismissTopDialog()).toBe(true);
    expect([closeInner.mock.calls.length, closeOuter.mock.calls.length]).toEqual([1, 0]);
    rerender(both(false));
    expect(dismissTopDialog()).toBe(true);
    expect([closeInner.mock.calls.length, closeOuter.mock.calls.length]).toEqual([1, 1]);
    unmount();
    expect(dismissTopDialog()).toBe(false);
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
