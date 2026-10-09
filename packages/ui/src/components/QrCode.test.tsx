import { render, screen } from "@testing-library/react";
import { QrCode, encodeQr, qrPath } from "./QrCode";

describe("QrCode", () => {
  it("encodes a ticket URI into a module matrix and an SVG path", () => {
    const matrix = encodeQr("voltip://pair?v=1&s=abcd&t=0123");
    expect(matrix.size).toBeGreaterThanOrEqual(21);
    expect(matrix.dark(0, 0)).toBe(true);
    const path = qrPath(matrix);
    expect(path.startsWith("M0 0h7")).toBe(true);
    expect(qrPath({ size: 2, dark: (x, y) => x === 1 && y === 1 })).toBe("M1 1h1v1h-1z");
  });

  it("renders dimmed with an overlay when expired", () => {
    render(<QrCode value="voltip://pair?v=1" dimmed overlay={<span>已过期</span>} size={120} />);
    expect(screen.getByRole("img", { name: "配对二维码" })).toHaveClass("opacity-40");
    expect(screen.getByText("已过期")).toBeInTheDocument();
    render(<QrCode value="x" />);
    expect(screen.getAllByRole("img")).toHaveLength(2);
  });
});
