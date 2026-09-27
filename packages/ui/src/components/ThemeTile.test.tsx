import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ThemeTile } from "./ThemeTile";

describe("ThemeTile", () => {
  it("paints the preview with its own data-theme and reports selection", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const { container } = render(
      <>
        <ThemeTile theme="light" selected onSelect={onSelect} caption="light · 默认" />
        <ThemeTile theme="dark" selected={false} onSelect={onSelect} />
        <ThemeTile theme="warm" selected={false} onSelect={onSelect} disabled />
      </>,
    );
    expect(container.querySelector('[data-theme="dark"]')).not.toBeNull();
    expect(screen.getByRole("radio", { name: "明亮" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText("light · 默认")).toBeInTheDocument();
    expect(screen.getByText("dark")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "暗黑" }));
    expect(onSelect).toHaveBeenCalledWith("dark");
    expect(screen.getByRole("radio", { name: "暖纸" })).toBeDisabled();
  });
});
