import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog } from "./Dialog";
import { Popover } from "./Popover";

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
});
