import { render, screen } from "@testing-library/react";
import { CardGrid, SettingsPane, SettingsRows, SettingsSection } from "./SettingsLayout";
import { StatusRow } from "./StatusRow";

describe("settings layout", () => {
  it("a pane has one h2 title, the lede and its sections as labelled regions", () => {
    render(
      <SettingsPane
        title="通用"
        lede="语言与更新"
        data-testid="pane"
        actions={<button type="button">重置</button>}>
        <SettingsSection
          title="语言"
          description="界面语言"
          aside={<span>中文</span>}
          data={{ "data-state": "on" }}>
          <SettingsRows>
            <StatusRow label="界面语言">
              <span>x</span>
            </StatusRow>
          </SettingsRows>
        </SettingsSection>
      </SettingsPane>,
    );
    expect(screen.getByRole("heading", { level: 2, name: "通用" })).toBeInTheDocument();
    expect(screen.getByText("语言与更新")).toBeInTheDocument();
    const section = screen.getByRole("region", { name: "语言" });
    expect(section).toHaveAttribute("data-state", "on");
    expect(section).toHaveTextContent("界面语言");
    expect(screen.getByRole("heading", { level: 3, name: "语言" })).toHaveClass("eyebrow");
    expect(screen.getByRole("button", { name: "重置" })).toBeInTheDocument();
    expect(screen.getByTestId("pane")).toBeInTheDocument();
  });

  it("a card grid lays columns out by width", () => {
    render(
      <CardGrid min={240} role="list" aria-label="模型">
        <div role="listitem">a</div>
      </CardGrid>,
    );
    const grid = screen.getByRole("list", { name: "模型" });
    expect(grid.style.gridTemplateColumns).toBe("repeat(auto-fill, minmax(240px, 1fr))");
  });
});
