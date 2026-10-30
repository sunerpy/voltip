import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Tabs } from "./Tabs";

type View = "one" | "two" | "three";

const ITEMS = [
  { value: "one" as const, label: "One", id: "tab-one", panelId: "panel-one" },
  { value: "two" as const, label: "Two", id: "tab-two", panelId: "panel-two" },
  { value: "three" as const, label: "Three", id: "tab-three", panelId: "panel-three" },
];

function Views({ onChange }: { onChange: (view: View) => void }) {
  const [view, setView] = useState<View>("one");
  return (
    <>
      <Tabs
        items={ITEMS}
        value={view}
        label="Views"
        onChange={(next) => {
          onChange(next);
          setView(next);
        }}
      />
      <div role="tabpanel" id={`panel-${view}`} aria-labelledby={`tab-${view}`}>
        {view}
      </div>
    </>
  );
}

describe("Tabs", () => {
  it("is a tab list: the selected tab alone is in the Tab order and names its panel", () => {
    render(<Views onChange={() => undefined} />);
    expect(screen.getByRole("tablist", { name: "Views" })).toBeInTheDocument();
    const [one, two] = screen.getAllByRole("tab");
    expect(one).toHaveAttribute("aria-selected", "true");
    expect(one).toHaveAttribute("tabindex", "0");
    expect(one).toHaveAttribute("aria-controls", "panel-one");
    expect(two).toHaveAttribute("aria-selected", "false");
    expect(two).toHaveAttribute("tabindex", "-1");
    expect(two).not.toHaveAttribute("aria-controls");
    expect(screen.getByRole("tabpanel", { name: "One" })).toHaveTextContent("one");
  });

  it("a click shows a view, and the arrow keys, Home and End move to one and show it", async () => {
    const user = userEvent.setup();
    const changes: View[] = [];
    render(<Views onChange={(view) => changes.push(view)} />);
    await user.click(screen.getByRole("tab", { name: "Two" }));
    expect(screen.getByRole("tabpanel", { name: "Two" })).toBeInTheDocument();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Three" })).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "One" })).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("tab", { name: "Three" })).toHaveFocus();
    await user.keyboard("{Home}");
    expect(screen.getByRole("tab", { name: "One" })).toHaveFocus();
    await user.keyboard("{End}");
    expect(screen.getByRole("tab", { name: "Three" })).toHaveAttribute("aria-selected", "true");
    // Other keys are left to the page.
    await user.keyboard("{ArrowDown}x");
    expect(changes).toEqual(["two", "three", "one", "three", "one", "three"]);
  });

  it("comes in the small size too", () => {
    render(<Tabs items={ITEMS} value="two" label="Views" size="sm" onChange={() => undefined} />);
    expect(screen.getByRole("tablist")).toHaveClass("h-7");
    expect(screen.getByRole("tab", { name: "Two" })).toHaveClass("text-[11px]");
  });
});
