import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sidebar } from "./Sidebar";
import { Toolbar } from "./Toolbar";

describe("Sidebar", () => {
  it("renders groups, counts, active and disabled items, footer buttons", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const onFooter = vi.fn();
    render(
      <Sidebar
        activeId="home"
        onNavigate={onNavigate}
        disabledIds={["history"]}
        groups={[
          {
            title: "工作台",
            items: [
              { id: "home", label: "首页", icon: "home" },
              { id: "history", label: "历史记录", icon: "history", count: 128 },
            ],
          },
          { title: "配置", items: [{ id: "settings", label: "设置", icon: "settings" }] },
        ]}
        footer={[{ id: "about", icon: "info", label: "关于", onClick: onFooter }]}
      />,
    );
    expect(screen.getByRole("button", { name: /首页/ })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: /历史记录/ })).toBeDisabled();
    expect(screen.getByText("128")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /设置/ }));
    expect(onNavigate).toHaveBeenCalledWith("settings");
    await user.click(screen.getByRole("button", { name: "关于" }));
    expect(onFooter).toHaveBeenCalled();
    expect(screen.getByText("Voltip")).toBeInTheDocument();
  });

  it("regression: the brand row is a 40 px deep drag region flush with the top edge, with a macOS traffic-light inset", () => {
    const props = { activeId: "home", onNavigate: vi.fn(), groups: [] };
    const { rerender } = render(<Sidebar {...props} className="extra" />);
    const nav = screen.getByRole("navigation", { name: "主导航" });
    expect(nav).toHaveClass("pb-4", "extra");
    expect(nav).not.toHaveClass("py-4");
    const brand = screen.getByTestId("sidebar-brand");
    expect(brand).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(brand).toHaveClass("h-10", "px-2");
    expect(brand).not.toHaveClass("pl-15");
    expect(brand.querySelector("button")).toBeNull();
    rerender(<Sidebar {...props} trafficLights />);
    expect(screen.getByTestId("sidebar-brand")).toHaveClass("h-10", "pl-15", "pr-2");
    expect(screen.getByTestId("sidebar-brand")).not.toHaveClass("px-2");
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
