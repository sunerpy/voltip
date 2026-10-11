import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  SIDEBAR_FOOTER_BUTTON_CLASS,
  SIDEBAR_RAIL_WIDTH,
  SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS,
  SIDEBAR_ROW_CLASS,
  SIDEBAR_ROW_EXPANDED_CLASS,
  SIDEBAR_WIDTH,
  Sidebar,
  SidebarButton,
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

  it("renders groups, counts, active and disabled items, the status card and footer buttons", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const onFooter = vi.fn();
    render(
      <Sidebar
        activeId="home"
        onNavigate={onNavigate}
        disabledIds={["history"]}
        groups={groups}
        status={<span>中继 · 已连接</span>}
        footer={<SidebarButton icon="settings" label="设置" opensDialog onClick={onFooter} />}
      />,
    );
    const home = screen.getByRole("button", { name: /首页/ });
    expect(home).toHaveAttribute("aria-current", "page");
    // Board 01: the current row is the raised one, its glyph in the primary colour.
    expect(home).toHaveClass("bg-nav-active", "shadow-row");
    expect(home.querySelector("svg")).toHaveClass("text-primary");
    expect(screen.getByRole("button", { name: /历史记录/ })).toBeDisabled();
    expect(screen.getByText("128")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /语音模型/ }));
    expect(onNavigate).toHaveBeenCalledWith("speech");
    expect(screen.getByTestId("sidebar-status")).toHaveTextContent("中继 · 已连接");
    const settings = screen.getByRole("button", { name: "设置" });
    expect(settings).toHaveAttribute("aria-haspopup", "dialog");
    expect(settings).toHaveAttribute("title", "设置");
    expect(settings).not.toHaveAttribute("aria-current");
    for (const cls of SIDEBAR_FOOTER_BUTTON_CLASS.split(" ")) expect(settings).toHaveClass(cls);
    expect(screen.getByTestId("sidebar-footer")).toHaveClass("grid-cols-3");
    await user.click(settings);
    expect(onFooter).toHaveBeenCalled();
    expect(screen.getByText("Voltip")).toBeInTheDocument();
    expect(SIDEBAR_WIDTH).toBe(232);
    expect(screen.getByRole("navigation", { name: "主导航" })).toHaveStyle({ width: "232px" });
  });

  it("regression: the collapsed rail is 72 px of glyphs whose labels stay as names and tooltips", () => {
    render(
      <Sidebar
        activeId="home"
        onNavigate={vi.fn()}
        groups={groups}
        collapsed
        controls={<button type="button">展开侧栏</button>}
        status={<span>中继 · 已连接</span>}
        footer={<SidebarButton icon="chat" label="反馈" onClick={vi.fn()} />}
      />,
    );
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav).toHaveAttribute("data-collapsed", "true");
    expect(SIDEBAR_RAIL_WIDTH).toBe(72);
    expect(nav).toHaveStyle({ width: "72px", minWidth: "72px", maxWidth: "72px" });
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
    // The status card needs the room of the expanded panel; the footer stacks.
    expect(screen.queryByTestId("sidebar-status")).toBeNull();
    expect(screen.getByTestId("sidebar-footer")).toHaveClass("grid-cols-1");
    // The layout controls move out of the brand row, under it.
    expect(screen.getByTestId("sidebar-brand")).not.toContainElement(
      screen.getByRole("button", { name: "展开侧栏" }),
    );
    expect(screen.getByTestId("sidebar-controls")).toContainElement(
      screen.getByRole("button", { name: "展开侧栏" }),
    );
  });

  it("regression: the panel floats with the glass surface, and its brand row drags the window", () => {
    const props = { activeId: "home", onNavigate: vi.fn(), groups: [] };
    const { rerender } = render(<Sidebar {...props} className="extra" />);
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav).toHaveClass(
      "extra",
      "rounded-panel",
      "border",
      "border-sidebar-edge",
      "bg-sidebar",
      "shadow-sidebar",
    );
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(brand).toHaveClass("h-16", "px-2");
    expect(brand).not.toHaveClass(TRAFFIC_LIGHTS_BRAND_INSET);
    // The lamp is a status, not a control.
    expect(brand.querySelector("button")).toBeNull();
    rerender(<Sidebar {...props} trafficLights />);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass(
      "h-16",
      TRAFFIC_LIGHTS_BRAND_INSET,
      "pr-1",
    );
    expect(screen.getByTestId("sidebar-brand")).not.toHaveClass("px-2");
    // Floating over the page (the hover preview of a hidden sidebar): the window's shadow.
    rerender(<Sidebar {...props} floating />);
    expect(screen.getByRole("navigation", { name: "主导航" })).toHaveClass("shadow-win");
    expect(screen.getByRole("navigation", { name: "主导航" })).not.toHaveClass("shadow-sidebar");
  });
});

describe("Sidebar on macOS", () => {
  const props = { activeId: "home", onNavigate: vi.fn(), groups: [] };

  it("regression: the traffic lights get a slot of their own, clear of the brand", () => {
    // User feedback 2026-09-29: on macOS the red, yellow and green buttons crowded the app mark.
    // From 0.1.1 the lights sit in the floating panel's brand row (trafficLightPosition x 32): they
    // end 84 px from the window's edge and the wordmark starts 16 px later, 100 px in, in px so the
    // 字号 setting cannot shrink it, without the mark (the Dock, the menu bar and the tray show it).
    const { rerender } = render(<Sidebar {...props} trafficLights />);
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveClass(TRAFFIC_LIGHTS_BRAND_INSET);
    expect(TRAFFIC_LIGHTS_CLEARANCE).toBe(100);
    // 100 px less the panel's 16 px margin and its 12 px padding.
    expect(TRAFFIC_LIGHTS_BRAND_INSET).toBe("pl-[72px]");
    expect(within(brand).queryByTestId("app-logo")).toBeNull();
    expect(within(brand).getByText("Voltip")).toBeInTheDocument();
    // Elsewhere the mark stays.
    rerender(<Sidebar {...props} />);
    expect(within(screen.getByTestId("sidebar-brand")).getByTestId("app-logo")).toBeInTheDocument();
  });

  it("regression: the collapsed rail holds the traffic lights instead of cutting through them", () => {
    // The lights are 52 px wide from x 32: on macOS the rail is 84 px, the lights with 16 px on
    // either side, and the brand row stays invisible under them.
    const { rerender } = render(<Sidebar {...props} collapsed trafficLights />);
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav.style.width).toBe(`${SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS}px`);
    expect(SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS).toBe(84);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass("invisible");
    rerender(<Sidebar {...props} collapsed />);
    expect(screen.getByRole("navigation", { name: "主导航" }).style.width).toBe(
      `${SIDEBAR_RAIL_WIDTH}px`,
    );
  });
});

describe("ThemeSwitch", () => {
  it("is a square footer button whose menu, above it, lists every choice and picks one", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { rerender } = render(<ThemeSwitch value="light" onChange={onChange} />);
    expect(nextThemeChoice("light")).toBe("dark");
    expect(nextThemeChoice("graphite")).toBe("system");
    // 澄空 and its dark twin 夜空 come first (the default since 0.1.1).
    expect(nextThemeChoice("system")).toBe("glass");
    expect(nextThemeChoice("glass")).toBe("glass_dark");
    expect(nextThemeChoice("glass_dark")).toBe("light");
    // The button says what is chosen; the glyph is the theme's.
    const button = screen.getByRole("button", { name: "主题 · 明亮" });
    expect(button).toHaveAttribute("title", "主题 · 明亮");
    expect(button).toHaveAttribute("aria-haspopup", "menu");
    expect(button.querySelector("svg")).toHaveAttribute("data-icon", "sun");
    for (const cls of SIDEBAR_FOOTER_BUTTON_CLASS.split(" ")) expect(button).toHaveClass(cls);
    await user.click(button);
    const menu = screen.getByRole("menu", { name: "主题" });
    // It opens above the button: the button sits at the window's bottom edge.
    expect(menu).toHaveClass("bottom-full");
    expect(
      within(menu)
        .getAllByRole("menuitemradio")
        .map((o) => o.textContent),
    ).toEqual(["跟随系统", "澄空", "夜空", "明亮", "暗黑", "暖纸", "石墨"]);
    expect(within(menu).getByRole("menuitemradio", { name: "明亮" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(menu).getByRole("menuitemradio", { name: "暖纸" }));
    expect(onChange).toHaveBeenLastCalledWith("warm");
    rerender(<ThemeSwitch value="glass_dark" onChange={onChange} />);
    expect(
      screen.getByRole("button", { name: "主题 · 夜空" }).querySelector("svg"),
    ).toHaveAttribute("data-icon", "nightSky");
    rerender(<ThemeSwitch value="system" onChange={onChange} />);
    await user.click(screen.getByRole("button", { name: "主题 · 跟随系统" }));
    await user.click(screen.getByRole("menuitemradio", { name: "澄空" }));
    expect(onChange).toHaveBeenLastCalledWith("glass");
  });
});

describe("the sidebar's rows line up", () => {
  it("regression: every nav row has the same 44 px box, glyph inset and label gap", () => {
    render(
      <Sidebar
        activeId="home"
        onNavigate={vi.fn()}
        groups={[
          {
            title: "工作台",
            items: [
              { id: "home", label: "首页", icon: "home" },
              { id: "rules", label: "规则", icon: "sparkles", count: 6 },
            ],
          },
        ]}
      />,
    );
    expect(SIDEBAR_ROW_CLASS).toContain("h-11");
    for (const name of [/首页/, /规则/]) {
      const row = screen.getByRole("button", { name });
      for (const cls of SIDEBAR_ROW_CLASS.split(" ")) expect(row).toHaveClass(cls);
      for (const cls of SIDEBAR_ROW_EXPANDED_CLASS.split(" ")) expect(row).toHaveClass(cls);
      expect(row.querySelector("svg")).toHaveAttribute("width", "18");
    }
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
