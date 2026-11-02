import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Checkbox } from "./Checkbox";

function Toggled({ initial = false, disabled = false }: { initial?: boolean; disabled?: boolean }) {
  const [checked, setChecked] = useState(initial);
  return <Checkbox checked={checked} onChange={setChecked} label="选择" disabled={disabled} />;
}

describe("Checkbox", () => {
  it("is a native check box named by its label: a click and Space toggle it", async () => {
    const user = userEvent.setup();
    render(<Toggled />);
    const box = screen.getByRole("checkbox", { name: "选择" });
    expect(box).not.toBeChecked();
    await user.click(box);
    expect(box).toBeChecked();
    box.focus();
    await user.keyboard(" ");
    expect(box).not.toBeChecked();
  });

  it("shows a partial selection as mixed and drops it once the box is all or nothing", () => {
    const { rerender } = render(
      <Checkbox checked={false} indeterminate onChange={() => undefined} label="全选" />,
    );
    const box = screen.getByRole("checkbox", { name: "全选" });
    expect(box).toBePartiallyChecked();
    rerender(<Checkbox checked onChange={() => undefined} label="全选" />);
    expect(box).not.toBePartiallyChecked();
    expect(box).toBeChecked();
  });

  it("a disabled box does not change, and a click on it does not reach the row under it", async () => {
    const user = userEvent.setup();
    const row = vi.fn();
    render(
      <div onClick={row}>
        <Toggled disabled />
        <Checkbox checked={false} onChange={() => undefined} label="另一个" />
      </div>,
    );
    const off = screen.getByRole("checkbox", { name: "选择" });
    expect(off).toBeDisabled();
    await user.click(off);
    expect(off).not.toBeChecked();
    await user.click(screen.getByRole("checkbox", { name: "另一个" }));
    expect(row).not.toHaveBeenCalled();
  });
});
