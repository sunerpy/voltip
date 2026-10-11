import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Menu, type MenuSection } from "./Menu";

/** A box as `getBoundingClientRect()` reports it. */
function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    top,
    left,
    width,
    height,
    bottom: top + height,
    right: left + width,
    x: left,
    y: top,
    toJSON: () => ({}),
  };
}

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

  it("skips rows that cannot be chosen, names why, and picks nothing when one is clicked", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <Menu
        trigger="内置服务 · Qwen3-ASR-1.7B"
        label="语音模型"
        sections={[
          {
            label: "云端服务",
            items: [
              {
                kind: "radio",
                id: "openai",
                label: "OpenAI",
                checked: false,
                disabled: true,
                detail: "缺少密钥",
              },
              { kind: "radio", id: "builtin", label: "内置服务", checked: true },
              {
                kind: "radio",
                id: "groq",
                label: "Groq",
                checked: false,
                disabled: true,
                detail: "缺少密钥",
              },
            ],
          },
          {
            label: "本地模型",
            items: [{ kind: "radio", id: "local", label: "均衡", checked: false, detail: "本机" }],
          },
        ]}
        onSelect={onSelect}
        data-testid="speech"
      />,
    );
    await user.click(screen.getByRole("button", { name: "语音模型" }));
    const menu = screen.getByTestId("speech-menu");
    const openai = within(menu).getByRole("menuitemradio", { name: "OpenAI · 缺少密钥" });
    expect(openai).toBeDisabled();
    expect(openai).toHaveAttribute("aria-disabled", "true");
    expect(within(openai).getByText("缺少密钥")).toBeInTheDocument();
    expect(within(menu).getByRole("menuitemradio", { name: "内置服务" })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(within(menu).getByRole("menuitemradio", { name: "均衡 · 本机" })).toHaveFocus();
    // Wraps past the disabled first row back to the first one that can be chosen.
    await user.keyboard("{ArrowDown}");
    expect(within(menu).getByRole("menuitemradio", { name: "内置服务" })).toHaveFocus();
    await user.keyboard("{End}");
    expect(within(menu).getByRole("menuitemradio", { name: "均衡 · 本机" })).toHaveFocus();
    await user.keyboard("{Home}");
    expect(within(menu).getByRole("menuitemradio", { name: "内置服务" })).toHaveFocus();
    await user.click(openai);
    expect(onSelect).not.toHaveBeenCalled();
    expect(screen.getByTestId("speech-menu")).toBeInTheDocument();
  });

  it("tells its owner each time the user opens it, not when it closes", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <Menu
        trigger="Fifine K669"
        label="麦克风"
        sections={SECTIONS}
        onSelect={vi.fn()}
        onOpen={onOpen}
      />,
    );
    const trigger = screen.getByRole("button", { name: "麦克风" });
    await user.click(trigger);
    expect(onOpen).toHaveBeenCalledTimes(1);
    await user.click(trigger);
    expect(onOpen).toHaveBeenCalledTimes(1);
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it("picks with a click, keeps rows on one line and marks the user's own names", async () => {
    const user = userEvent.setup();
    const { onSelect, trigger } = renderMenu();
    await user.click(trigger);
    const menu = screen.getByTestId("presets-menu");
    expect(menu).toHaveClass("whitespace-nowrap", "w-max", "font-ui");
    expect(menu).toHaveAttribute("data-tauri-drag-region", "false");
    expect(within(menu).getByText("周报")).toHaveAttribute("data-user-text");
    expect(within(menu).getByText("校对")).not.toHaveAttribute("data-user-text");
    await user.click(within(menu).getByRole("menuitemradio", { name: "校对" }));
    expect(onSelect).toHaveBeenCalledWith("proofread");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  // User report 2026-10-11: in a small window the home page's preset and microphone menus opened
  // under their triggers and ran past the window's bottom, their last rows out of sight.
  it("regression: a menu low in a short window opens above its trigger and scrolls inside the room the page's scroll area leaves it", async () => {
    const user = userEvent.setup();
    render(
      // The page's scroll area: under the title bar and right of the sidebar of a 960 × 600 window.
      <div data-testid="page" style={{ overflowY: "auto" }}>
        <Menu
          trigger="校对"
          label="AI 预设"
          sections={SECTIONS}
          onSelect={vi.fn()}
          data-testid="presets"
        />
      </div>,
    );
    const trigger = screen.getByRole("button", { name: "AI 预设" });
    const page = screen.getByTestId("page");
    const content = { width: 157, height: 290 };
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(960);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(600);
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (
      this: Element,
    ) {
      if (this === page) return rect(246, 95, 714, 505);
      return this === trigger.parentElement ? rect(378, 433, 90, 20) : rect(0, 0, 0, 0);
    });
    vi.spyOn(Element.prototype, "clientWidth", "get").mockImplementation(function (this: Element) {
      return this === page ? 714 : 0;
    });
    vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
      return this === page ? 505 : 0;
    });
    const isMenu = (el: Element) => el.getAttribute("role") === "menu";
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (
      this: HTMLElement,
    ) {
      return isMenu(this) ? content.width : 0;
    });
    vi.spyOn(Element.prototype, "scrollHeight", "get").mockImplementation(function (this: Element) {
      return isMenu(this) ? content.height : 0;
    });

    // 135 px under the trigger, 326 above it: above, where all 290 px fit.
    await user.click(trigger);
    const menu = screen.getByTestId("presets-menu");
    expect(menu).toHaveClass("bottom-full", "mb-1", "overflow-y-auto", "overflow-x-hidden");
    expect(menu).not.toHaveClass("top-full");
    expect(menu.style.maxHeight).toBe(`${433 - 4 - (95 + 8)}px`);
    expect(menu.style.transform).toBe("");
    await user.keyboard("{Escape}");

    // Taller than either side: above, as tall as the room there, and it scrolls.
    content.height = 480;
    await user.click(trigger);
    expect(screen.getByTestId("presets-menu")).toHaveClass("bottom-full");
    expect(screen.getByTestId("presets-menu").style.maxHeight).toBe("326px");
    await user.keyboard("{Escape}");

    // Short enough for the room under the trigger: under it, as before.
    content.height = 120;
    await user.click(trigger);
    expect(screen.getByTestId("presets-menu")).toHaveClass("top-full", "mt-1");
  });
});
