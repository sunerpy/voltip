import { render, screen } from "@testing-library/react";
import { LOGO_NAVY, LOGO_ORANGE, LOGO_PALE, Logo } from "./Logo";

describe("Logo", () => {
  it("draws the app mark: navy rounded square, pale left arm, orange right arm", () => {
    render(<Logo size={32} />);
    const svg = screen.getByTestId("app-logo");
    expect(svg).toHaveAttribute("width", "32");
    expect(svg).toHaveAttribute("height", "32");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    const [square] = svg.querySelectorAll("rect");
    expect(square).toHaveAttribute("fill", LOGO_NAVY);
    expect(square).toHaveAttribute("rx", "22");
    const arms = svg.querySelectorAll("polygon");
    expect(arms).toHaveLength(2);
    expect(arms[0]).toHaveAttribute("fill", LOGO_PALE);
    expect(arms[1]).toHaveAttribute("fill", LOGO_ORANGE);
  });

  it("is an accessible image when labelled", () => {
    render(<Logo label="Voltip" />);
    expect(screen.getByRole("img", { name: "Voltip" })).toBeInTheDocument();
  });
});
