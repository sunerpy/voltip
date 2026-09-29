import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Menu, type MenuSection } from "./Menu";

const SECTIONS: MenuSection[] = [
  {
    label: "内置预设",
    items: [
      { kind: "radio", id: "proofread", label: "校对", checked: false },
      { kind: "radio", id: "prompt", label: "提示词优化", checked: true },
    ],
  },
  {
    label: "自定义预设",
    items: [{ kind: "radio", id: "u1", label: "周报", checked: false, userText: true }],
  },
  { items: [{ kind: "action", id: "manage", label: "管理预设…" }] },
];

function renderMenu(onSelect = vi.fn(), onOuterKey = vi.fn()) {
  render(
    // A dialog under the menu that closes on Esc unless the menu handled it first.
    <div
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) onOuterKey();
      }}>
      <Menu
        trigger="提示词优化"
        label="AI 预设"
        triggerLabel="AI 预设：提示词优化"
        sections={SECTIONS}
        onSelect={onSelect}
        data-testid="presets"
      />
      <button type="button">elsewhere</button>
    </div>,
  );
  return {
    onSelect,
    onOuterKey,
    trigger: screen.getByRole("button", { name: "AI 预设：提示词优化" }),
  };
}

describe("Menu", () => {
  it("opens on the checked choice, moves with the arrow keys and picks with Enter", async () => {
    const user = userEvent.setup();
    const { onSelect, trigger } = renderMenu();
    expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await user.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    const menu = screen.getByRole("menu", { name: "AI 预设" });
    expect(trigger).toHaveAttribute("aria-controls", menu.id);
    expect(
      within(menu)
        .getAllByRole("group")
        .map((g) => g.getAttribute("aria-label")),
    ).toEqual(["内置预设", "自定义预设", null]);
    expect(within(menu).getByRole("menuitemradio", { name: "提示词优化" })).toHaveFocus();
    expect(within(menu).getByRole("menuitemradio", { name: "提示词优化" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.keyboard("{ArrowDown}");
    expect(within(menu).getByRole("menuitemradio", { name: "周报" })).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    // Wraps from the last row to the first.
    expect(within(menu).getByRole("menuitemradio", { name: "校对" })).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(within(menu).getByRole("menuitem", { name: "管理预设…" })).toHaveFocus();
    await user.keyboard("{Home}");
    expect(within(menu).getByRole("menuitemradio", { name: "校对" })).toHaveFocus();
    await user.keyboard("{End}{Enter}");
    expect(onSelect).toHaveBeenCalledWith("manage");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("closes on Esc without letting the dialog beneath close, and on a press elsewhere", async () => {
    const user = userEvent.setup();
    const { onSelect, onOuterKey, trigger } = renderMenu();
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(onOuterKey).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();
    await user.click(trigger);
    fireEvent.pointerDown(screen.getByRole("button", { name: "elsewhere" }));
    expect(screen.queryByRole("menu")).toBeNull();
    // A second click on the trigger closes an open menu.
    await user.click(trigger);
    await user.click(trigger);
    expect(screen.queryByRole("menu")).toBeNull();
    // Tab leaves it closed; nothing was picked on the way.
    await user.click(trigger);
    await user.tab();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("picks with a click, keeps rows on one line and marks the user's own names", async () => {
    const user = userEvent.setup();
    const { onSelect, trigger } = renderMenu();
    await user.click(trigger);
    const menu = screen.getByTestId("presets-menu");
    expect(menu).toHaveClass("whitespace-nowrap", "w-max");
    expect(menu).toHaveAttribute("data-tauri-drag-region", "false");
    expect(within(menu).getByText("周报")).toHaveAttribute("data-user-text");
    expect(within(menu).getByText("校对")).not.toHaveAttribute("data-user-text");
    await user.click(within(menu).getByRole("menuitemradio", { name: "校对" }));
    expect(onSelect).toHaveBeenCalledWith("proofread");
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
