import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  SIDEBAR_GLYPH_SLOT_CLASS,
  SIDEBAR_RAIL_WIDTH,
  SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS,
  SIDEBAR_ROW_CLASS,
  SIDEBAR_ROW_EXPANDED_CLASS,
  Sidebar,
  SidebarEntry,
  TRAFFIC_LIGHTS_BRAND_INSET,
  TRAFFIC_LIGHTS_CLEARANCE,
} from "./Sidebar";
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
    expect(brand).not.toHaveClass(TRAFFIC_LIGHTS_BRAND_INSET);
    // The lamp is a status, not a control.
    expect(brand.querySelector("button")).toBeNull();
    rerender(<Sidebar {...props} trafficLights />);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass(
      "h-10",
      TRAFFIC_LIGHTS_BRAND_INSET,
      "pr-2",
    );
    expect(screen.getByTestId("sidebar-brand")).not.toHaveClass("px-2");
    // Floating (the hover preview of a hidden sidebar): a shadow, no border.
    rerender(<Sidebar {...props} floating />);
    expect(screen.getByRole("navigation", { name: "主导航" })).toHaveClass("shadow-win");
    expect(screen.getByRole("navigation", { name: "主导航" })).not.toHaveClass("border-r");
  });
});

describe("Sidebar on macOS", () => {
  const props = { activeId: "home", onNavigate: vi.fn(), groups: [] };

  it("regression: the traffic lights get a slot of their own, clear of the brand", () => {
    // User feedback 2026-09-29: on macOS the red, yellow and green buttons crowded the app mark
    // at the left of the title bar. The brand row starts 80 px from the window's left edge (the
    // lights end at 64 px; 16 px of air), in px so the 字号 setting cannot shrink it, and draws the
    // wordmark without the mark (the Dock, the menu bar and the tray already show it).
    const { rerender } = render(<Sidebar {...props} trafficLights />);
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveClass(TRAFFIC_LIGHTS_BRAND_INSET);
    expect(TRAFFIC_LIGHTS_BRAND_INSET).toBe(`pl-[calc(${TRAFFIC_LIGHTS_CLEARANCE}px_-_0.75rem)]`);
    expect(within(brand).queryByTestId("app-logo")).toBeNull();
    expect(within(brand).getByText("Voltip")).toBeInTheDocument();
    // Elsewhere the mark stays.
    rerender(<Sidebar {...props} />);
    expect(within(screen.getByTestId("sidebar-brand")).getByTestId("app-logo")).toBeInTheDocument();
  });

  it("regression: the collapsed rail holds the traffic lights instead of cutting through them", () => {
    // 56 px was narrower than the lights (they end at 64 px), so the rail's border crossed the
    // green button; on macOS the rail is 76 px: the lights with 12 px on either side.
    const { rerender } = render(<Sidebar {...props} collapsed trafficLights />);
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav.style.width).toBe(`${SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS}px`);
    expect(SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS).toBe(76);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass("invisible");
    rerender(<Sidebar {...props} collapsed />);
    expect(screen.getByRole("navigation", { name: "主导航" }).style.width).toBe(
      `${SIDEBAR_RAIL_WIDTH}px`,
    );
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
    expect(screen.getByTestId("theme-switch")).toHaveClass("justify-center");
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.getByRole("button", { name: "切换到跟随系统" })).toHaveAttribute(
      "title",
      "切换到跟随系统",
    );
  });
});

describe("the sidebar's rows line up", () => {
  it("regression: the theme row has the box of the other entries, its glyph fills the glyph slot and its menu text starts where a label does, with one hover surface", () => {
    render(
      <div>
        <SidebarEntry icon="chat" label="反馈" onClick={() => undefined} />
        <ThemeSwitch value="light" onChange={() => undefined} />
        <SidebarEntry icon="settings" label="设置" onClick={() => undefined} />
      </div>,
    );
    const entry = screen.getByRole("button", { name: "设置" });
    const row = screen.getByTestId("theme-switch");
    // The same 36 px row box as every entry.
    for (const cls of SIDEBAR_ROW_CLASS.split(" ")) {
      expect(entry).toHaveClass(cls);
      expect(row).toHaveClass(cls);
    }
    // An entry puts its glyph 0.5 rem in and its label 0.625 rem after the 16 px glyph; the theme
    // row's glyph button is exactly that slot, and the menu text starts with no padding of its own.
    for (const cls of SIDEBAR_ROW_EXPANDED_CLASS.split(" ")) expect(entry).toHaveClass(cls);
    const step = screen.getByTestId("theme-switch-step");
    for (const cls of SIDEBAR_GLYPH_SLOT_CLASS.split(" ")) expect(step).toHaveClass(cls);
    // 0.5 rem + 0.625 rem (the row's inset and gap) + the 16 px glyph.
    expect(SIDEBAR_GLYPH_SLOT_CLASS).toBe("w-[calc(1.125rem_+_16px)] pl-2");
    const menu = screen.getByRole("combobox", { name: "主题" });
    expect(menu).toHaveClass("pl-0");
    // One hover surface for the whole row, like an entry; neither half paints its own.
    expect(row).toHaveClass("hover:bg-nav-active");
    expect(entry.className).toMatch(/hover:bg-nav-active/);
    expect(step.className).not.toMatch(/hover:bg-/);
    expect(menu.className).not.toMatch(/hover:bg-/);
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
