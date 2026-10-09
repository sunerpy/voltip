import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import {
  CardGrid,
  DisclosureCard,
  SettingsPane,
  SettingsRows,
  SettingsSection,
} from "./SettingsLayout";
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

  it("a disclosure card toggles its region from the header button and keeps actions outside it", async () => {
    const user = userEvent.setup();
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <DisclosureCard
          open={open}
          onToggle={setOpen}
          icon="cloud"
          title="Groq"
          subtitle="whisper-large-v3-turbo"
          badge={<span>使用中</span>}
          actions={<button type="button">使用</button>}
          selected
          aria-label="Groq">
          <p>body</p>
        </DisclosureCard>
      );
    }
    render(<Harness />);
    const card = screen.getByRole("article", { name: "Groq" });
    expect(card).toHaveAttribute("data-selected", "true");
    const toggle = screen.getByRole("button", { name: /Groq/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("body")).toBeNull();
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("region", { name: /Groq/ })).toHaveTextContent("body");
    expect(card).toHaveAttribute("data-open", "true");
    // The action is not part of the toggle.
    const use = screen.getByRole("button", { name: "使用" });
    expect(toggle.contains(use)).toBe(false);
    await user.click(toggle);
    expect(screen.queryByText("body")).toBeNull();
  });
});
