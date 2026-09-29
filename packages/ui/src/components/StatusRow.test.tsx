import { render, screen } from "@testing-library/react";
import { StatusRow } from "./StatusRow";

describe("StatusRow", () => {
  it("regression: help text is not capped at a fixed width", () => {
    // User feedback 2026-09-29: help that fits the row went to a second line at 280 px.
    render(
      <StatusRow label="插入方式" help="粘贴会临时占用剪贴板，完成后恢复原内容。" data-testid="row">
        <button type="button">切换</button>
      </StatusRow>,
    );
    const help = screen.getByText("粘贴会临时占用剪贴板，完成后恢复原内容。");
    const left = help.parentElement;
    if (left === null) throw new Error("no label column");
    expect(left.className).not.toMatch(/max-w-\[/);
    expect(left.className).toContain("flex-1");
    expect(left.className).toContain("min-w-0");
    // The control column keeps its size up to 60 % of the row and may shrink below its content,
    // so a long readout truncates instead of squeezing the label to nothing.
    const right = screen.getByRole("button", { name: "切换" }).parentElement;
    expect(right?.className).toContain("max-w-[60%]");
    expect(right?.className).toContain("min-w-0");
    expect(right?.className).not.toContain("shrink-0");
  });
});
