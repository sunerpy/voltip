import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sidebar, SidebarEntry } from "./Sidebar";
import { ThemeSwitch, nextThemeChoice } from "./ThemeSwitch";
import { Toolbar } from "./Toolbar";

describe("Sidebar", () => {
  const groups = [
    {
      title: "工作台",
      items: [
        { id: "home", label: "首页", icon: "home" as const },
        { id: "history", label: "历史记录", icon: "history" as const, count: 128 },
      ],
    },
    { title: "语音输入", items: [{ id: "speech", label: "语音模型", icon: "wave" as const }] },
  ];

  it("renders groups, counts, active and disabled items, footer entries", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const onFooter = vi.fn();
    render(
      <Sidebar
        activeId="home"
        onNavigate={onNavigate}
        disabledIds={["history"]}
        groups={groups}
        footer={<SidebarEntry icon="settings" label="设置" opensDialog onClick={onFooter} />}
      />,
    );
    expect(screen.getByRole("button", { name: /首页/ })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: /历史记录/ })).toBeDisabled();
    expect(screen.getByText("128")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /语音模型/ }));
    expect(onNavigate).toHaveBeenCalledWith("speech");
    const settings = screen.getByRole("button", { name: "设置" });
    expect(settings).toHaveAttribute("aria-haspopup", "dialog");
    expect(settings).not.toHaveAttribute("aria-current");
    await user.click(settings);
    expect(onFooter).toHaveBeenCalled();
    expect(screen.getByText("Voltip")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "主导航" })).toHaveStyle({ width: "224px" });
  });

  it("regression: the collapsed rail is 56 px of glyphs whose labels stay as names and tooltips", () => {
    render(
      <Sidebar
        activeId="home"
        onNavigate={vi.fn()}
        groups={groups}
        collapsed
        controls={<button type="button">展开侧栏</button>}
        footer={<SidebarEntry icon="chat" label="反馈" collapsed onClick={vi.fn()} />}
      />,
    );
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav).toHaveAttribute("data-collapsed", "true");
    expect(nav).toHaveStyle({ width: "56px", minWidth: "56px", maxWidth: "56px" });
    expect(screen.queryByText("Voltip")).toBeNull();
    expect(screen.queryByText("首页")).toBeNull();
    const home = screen.getByRole("button", { name: "首页" });
    expect(home).toHaveAttribute("title", "首页");
    expect(home).toHaveAttribute("aria-current", "page");
    // The count travels with the name, since there is no room to draw it.
    expect(screen.getByRole("button", { name: "历史记录 · 128" })).toHaveAttribute(
      "title",
      "历史记录 · 128",
    );
    expect(screen.getByRole("button", { name: "反馈" })).toHaveAttribute("title", "反馈");
    // The layout controls move out of the 40 px brand row, under it.
    expect(screen.getByTestId("sidebar-brand")).not.toContainElement(
      screen.getByRole("button", { name: "展开侧栏" }),
    );
    expect(screen.getByTestId("sidebar-controls")).toContainElement(
      screen.getByRole("button", { name: "展开侧栏" }),
    );
  });

  it("regression: the brand row is a 40 px deep drag region flush with the top edge, with a macOS traffic-light inset", () => {
    const props = { activeId: "home", onNavigate: vi.fn(), groups: [] };
    const { rerender } = render(<Sidebar {...props} className="extra" />);
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav).toHaveClass("pb-4", "extra", "border-r");
    expect(nav).not.toHaveClass("py-4");
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(brand).toHaveClass("h-10", "px-2");
    expect(brand).not.toHaveClass("pl-15");
    // The lamp is a status, not a control.
    expect(brand.querySelector("button")).toBeNull();
    rerender(<Sidebar {...props} trafficLights />);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass("h-10", "pl-15", "pr-2");
    expect(screen.getByTestId("sidebar-brand")).not.toHaveClass("px-2");
    // Floating (the hover preview of a hidden sidebar): a shadow, no border.
    rerender(<Sidebar {...props} floating />);
    expect(screen.getByRole("navigation", { name: "主导航" })).toHaveClass("shadow-win");
    expect(screen.getByRole("navigation", { name: "主导航" })).not.toHaveClass("border-r");
  });
});

describe("ThemeSwitch", () => {
  it("steps through the themes at the glyph and picks one from the menu", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<ThemeSwitch value="light" onChange={onChange} />);
    expect(nextThemeChoice("light")).toBe("dark");
    expect(nextThemeChoice("graphite")).toBe("system");
    expect(nextThemeChoice("system")).toBe("light");
    await user.click(screen.getByRole("button", { name: "切换到暗黑" }));
    expect(onChange).toHaveBeenLastCalledWith("dark");
    const menu = screen.getByRole("combobox", { name: "主题" });
    expect(menu).toHaveValue("light");
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "跟随系统",
      "明亮",
      "暗黑",
      "暖纸",
      "石墨",
    ]);
    await user.selectOptions(menu, "warm");
    expect(onChange).toHaveBeenLastCalledWith("warm");
    rerender(<ThemeSwitch value="graphite" onChange={onChange} collapsed />);
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByRole("button", { name: "切换到跟随系统" })).toHaveAttribute(
      "title",
      "切换到跟随系统",
    );
  });
});

describe("Toolbar", () => {
  it("renders human-readable readouts with lamps, search button and right slot", async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn();
    render(
      <Toolbar
        title="首页"
        readouts={[
          { label: "引擎", value: "精确 · SenseVoice", lamp: "ok" },
          { label: "麦克风", value: "Fifine K669" },
          {
            label: "Bridge",
            value: "运行中 · 2 客户端",
            lamp: "ok",
            mono: true,
            title: "127.0.0.1:47823",
          },
        ]}
        onSearch={onSearch}
        right={<span>润色 · 开</span>}
      />,
    );
    expect(screen.getByRole("heading", { name: "首页" })).toBeInTheDocument();
    expect(screen.getByText("精确 · SenseVoice")).toBeInTheDocument();
    expect(screen.getByTitle("127.0.0.1:47823")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /搜索或输入命令/ }));
    expect(onSearch).toHaveBeenCalled();
    expect(screen.getByText("润色 · 开")).toBeInTheDocument();
    render(<Toolbar title="无搜索" />);
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });
});
