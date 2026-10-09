import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Select } from "./Select";

const OPTIONS = [
  { value: "auto", label: "自动检测" },
  { value: "zh", label: "中文 · zh" },
  { value: "mic", label: "跟随系统默认（Fifine K669 USB Microphone）" },
] as const;

describe("Select", () => {
  it("regression: the chosen label is not clipped by a fixed caller width", () => {
    // User feedback 2026-09-29: callers' fixed widths (w-44) cut the chosen label. The select now
    // shares a grid cell with an invisible copy of every label, one per line: the column, and the
    // select that fills it, are at least as wide as the longest option. No CSS field-sizing.
    const { container } = render(
      <Select label="设备" options={OPTIONS} value="mic" onChange={() => undefined} />,
    );
    const select = screen.getByLabelText("设备");
    const sizer = container.querySelector("[data-select-sizer]");
    if (!(sizer instanceof HTMLElement)) throw new Error("no sizer");
    expect([...sizer.children].map((line) => line.textContent)).toEqual(
      OPTIONS.map((o) => o.label),
    );
    // `min-w-max`: the copy is a scroll container (overflow hidden), which would otherwise give it
    // no automatic minimum width, and the column would not grow (browser check 2026-09-29).
    for (const cls of [
      "whitespace-nowrap",
      "col-start-1",
      "row-start-1",
      "invisible",
      "h-0",
      "min-w-max",
    ]) {
      expect(sizer.className).toContain(cls);
    }
    expect(sizer).toHaveAttribute("aria-hidden", "true");
    for (const cls of ["col-start-1", "row-start-1", "w-full"]) {
      expect(select.className).toContain(cls);
    }
    expect(sizer.parentElement).toBe(select.parentElement);
    expect(sizer.parentElement?.className).toContain("grid");
    // The copy is not read out: the select still has exactly its own options.
    expect(screen.getAllByRole("option")).toHaveLength(OPTIONS.length);
  });

  it("the sizer follows the select's size and mono face", () => {
    const { container } = render(
      <Select options={OPTIONS} value="zh" onChange={() => undefined} size="sm" mono />,
    );
    const sizer = container.querySelector("[data-select-sizer]");
    expect(sizer?.className).toContain("text-[12px]");
    expect(sizer?.className).toContain("mono");
  });

  it("calls onChange with the chosen option's value", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Select label="语言" options={OPTIONS} value="auto" onChange={onChange} />);
    await user.selectOptions(screen.getByLabelText("语言"), "zh");
    expect(onChange).toHaveBeenCalledWith("zh");
  });
});
