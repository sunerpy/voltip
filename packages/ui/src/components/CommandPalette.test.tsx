import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type CommandItem, CommandPalette, filterCommands } from "./CommandPalette";

function items(run = vi.fn()): CommandItem[] {
  return [
    { id: "theme-light", group: "主题", label: "主题 › 明亮", hint: "当前", run },
    { id: "theme-dark", group: "主题", label: "主题 › 暗黑", run, icon: "settings" },
    { id: "dictate", group: "动作", label: "开始听写", keys: "Ctrl Alt Space", run },
    {
      id: "clear",
      group: "动作",
      label: "删除全部历史…",
      disabled: true,
      disabledHint: "历史记录已关闭",
      run,
    },
    { id: "settings", group: "导航", label: "打开设置 › 外观", keys: "Ctrl ,", run },
  ];
}

describe("CommandPalette", () => {
  it("filters by query and groups", () => {
    expect(filterCommands(items(), "暗").map((i) => i.id)).toEqual(["theme-dark"]);
    expect(filterCommands(items(), "  ")).toHaveLength(5);
    expect(filterCommands(items(), "当前").map((i) => i.id)).toEqual(["theme-light"]);
  });

  it("navigates with arrows, tab, enter; runs and closes; previews highlight", async () => {
    const user = userEvent.setup();
    const run = vi.fn();
    const onClose = vi.fn();
    const onHighlight = vi.fn();
    render(<CommandPalette open items={items(run)} onClose={onClose} onHighlight={onHighlight} />);
    const input = screen.getByRole("combobox");
    expect(input).toHaveFocus();
    expect(onHighlight).toHaveBeenLastCalledWith(expect.objectContaining({ id: "theme-light" }));
    expect(screen.getByText("5 条结果")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("option", { name: /暗黑/ })).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(screen.getByRole("option", { name: /打开设置/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{Tab}");
    expect(screen.getByRole("option", { name: /明亮/ })).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{Tab}");
    expect(screen.getByRole("option", { name: /开始听写/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{Enter}");
    expect(run).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("highlights matches, refuses disabled items, closes on Esc and scrim", async () => {
    const user = userEvent.setup();
    const run = vi.fn();
    const onClose = vi.fn();
    render(<CommandPalette open items={items(run)} onClose={onClose} />);
    await user.type(screen.getByRole("combobox"), "历史");
    expect(screen.getByText("1 条结果")).toBeInTheDocument();
    expect(screen.getByText("历史记录已关闭")).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(run).not.toHaveBeenCalled();
    await user.click(screen.getByRole("option", { name: /删除全部历史/ }));
    expect(run).not.toHaveBeenCalled();
    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "zzz");
    expect(screen.getByText("没有匹配的命令")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Enter}{Tab}");
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByTestId("palette-scrim"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("mouse hover moves the cursor and click runs", async () => {
    const user = userEvent.setup();
    const run = vi.fn();
    render(<CommandPalette open items={items(run)} onClose={vi.fn()} />);
    await user.hover(screen.getByRole("option", { name: /开始听写/ }));
    expect(screen.getByRole("option", { name: /开始听写/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("option", { name: /开始听写/ }));
    expect(run).toHaveBeenCalled();
    const { container } = render(<CommandPalette open={false} items={[]} onClose={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });
});
