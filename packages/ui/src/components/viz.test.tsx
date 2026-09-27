import { render, screen } from "@testing-library/react";
import { Heatmap, levelClass } from "./Heatmap";
import { LedMeter, clamp01 } from "./LedMeter";
import { Progress } from "./Progress";
import { Sparkline, sparklinePoints } from "./Sparkline";
import { Waveform } from "./Waveform";

describe("LedMeter", () => {
  it("lights segments by level, marks clip zone danger and peak hold", () => {
    const { container } = render(<LedMeter level={0.5} peak={0.9} segments={10} clipFrom={0.8} />);
    const lit = container.querySelectorAll('[data-lit="true"]');
    expect(lit).toHaveLength(5);
    expect(screen.getByRole("meter")).toHaveAttribute("aria-valuenow", "50");
    expect(container.querySelector('[data-peak="true"]')).toHaveClass("bg-led-on");
    const { container: full } = render(
      <LedMeter level={1} segments={10} clipFrom={0.8} size="xs" />,
    );
    expect(full.querySelectorAll(".bg-danger")).toHaveLength(2);
    render(<LedMeter level={Number.NaN} disabled size="sm" />);
    expect(screen.getAllByRole("meter")[2]).toHaveClass("opacity-40");
    expect(clamp01(2)).toBe(1);
    expect(clamp01(-1)).toBe(0);
  });
});

describe("Waveform", () => {
  it("pads to the bar count, mirrors heights and colours the recent 45 %", () => {
    const { container } = render(<Waveform levels={[0.2, 1]} bars={10} height={20} />);
    const bars = container.querySelectorAll("span");
    expect(bars).toHaveLength(10);
    expect(bars[9]).toHaveStyle({ height: "20px" });
    expect(bars[9]).toHaveClass("bg-accent");
    expect(bars[0]).toHaveClass("bg-wave");
    const { container: frozen } = render(<Waveform levels={[1, 1]} state="frozen" bars={2} />);
    expect(frozen.querySelectorAll(".bg-led-off")).toHaveLength(2);
    const { container: collapsed } = render(<Waveform levels={[1]} state="collapsed" bars={3} />);
    expect(collapsed.querySelector("span")).toHaveStyle({ height: "2px" });
    const { container: danger } = render(
      <Waveform levels={Array.from({ length: 50 }, () => 0.5)} tone="danger" bars={4} />,
    );
    expect(danger.querySelectorAll(".bg-danger").length).toBeGreaterThan(0);
    render(<Waveform levels={[0.4]} state="idle" bars={1} />);
    expect(screen.getAllByRole("img")).toHaveLength(5);
  });
});

describe("Heatmap / Sparkline / Progress", () => {
  it("heatmap maps levels to four accent classes with legend and headers", () => {
    const { container } = render(
      <Heatmap
        values={[
          [0, 1],
          [2, 3],
        ]}
        columnLabels={["W-1", "W0"]}
      />,
    );
    expect(container.querySelectorAll('[data-level="3"]')).toHaveLength(1);
    expect(container.querySelector('[data-level="3"]')).toHaveClass("bg-accent");
    expect(screen.getByText("W0")).toBeInTheDocument();
    expect(screen.getByText("少")).toBeInTheDocument();
    expect(levelClass(9)).toBe("bg-accent");
    expect(levelClass(-2)).toBe("bg-inset2");
    render(<Heatmap values={[[1]]} legend={false} />);
    expect(screen.getAllByText("少")).toHaveLength(1);
  });

  it("sparkline builds points and progress reports value / segments / indeterminate", () => {
    expect(sparklinePoints([], 10, 10)).toBe("");
    expect(sparklinePoints([1], 10, 10)).toBe("0.0,1.0");
    expect(sparklinePoints([0, 1], 10, 10).split(" ")).toHaveLength(2);
    render(<Sparkline values={[1, 3, 2]} />);
    expect(screen.getByRole("img", { name: "趋势" })).toBeInTheDocument();
    const { container } = render(
      <>
        <Progress value={0.62} />
        <Progress indeterminate size={2} tone="ink" />
        <Progress value={0.5} segments={4} tone="ok" size={8} />
      </>,
    );
    const bars = screen.getAllByRole("progressbar");
    expect(bars[0]).toHaveAttribute("aria-valuenow", "62");
    expect(bars[1]).not.toHaveAttribute("aria-valuenow");
    expect(container.querySelectorAll(".bg-ok")).toHaveLength(2);
  });
});
