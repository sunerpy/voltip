import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Mock } from "vitest";
import { I18nProvider } from "../i18n/I18nProvider";
import { TRAFFIC_LIGHTS_CLEARANCE } from "./Sidebar";
import {
  TITLE_BAR_HEIGHT,
  TITLE_BAR_READOUT_MAX,
  TITLE_BAR_SEARCH_LABEL,
  TitleBar,
  type TitleBarControls,
} from "./TitleBar";

function controls(): TitleBarControls & {
  [K in keyof TitleBarControls]-?: Mock<() => void>;
} {
  return {
    minimize: vi.fn<() => void>(),
    toggleMaximize: vi.fn<() => void>(),
    close: vi.fn<() => void>(),
    startDragging: vi.fn<() => void>(),
  };
}

const READOUTS = [
  { label: "引擎", value: "精确 · SenseVoice", lamp: "ok" as const },
  { label: "麦克风", value: "Fifine K669" },
  {
    label: "Bridge",
    value: "运行中 · 2 客户端",
    lamp: "ok" as const,
    mono: true,
    title: "127.0.0.1:47823",
  },
];

describe("TitleBar", () => {
  it("regression: the title bar carries only the title, search icon, polish icon and window controls", async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn();
    render(
      <TitleBar
        title="首页"
        onSearch={onSearch}
        right={<button type="button" aria-label="润色 · 开/关" />}
        platform="windows"
        controls={controls()}
        className="extra"
      />,
    );
    expect(TITLE_BAR_HEIGHT).toBe(40);
    const bar = screen.getByTestId("title-bar");
    expect(bar).toHaveClass("h-10", "extra");
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    // User feedback 2026-09-25: no readouts, no 220 px search field, no sample-data chip here.
    expect(bar.querySelectorAll("[data-tone]")).toHaveLength(0);
    expect(screen.queryByText("Ctrl K")).toBeNull();
    expect(screen.queryByText(/示例/)).toBeNull();
    const search = screen.getByTestId("title-bar-search");
    expect(search).toHaveAttribute("aria-label", TITLE_BAR_SEARCH_LABEL);
    expect(search).toHaveAttribute("title", "搜索或输入命令 · Ctrl K");
    expect(search.querySelector("svg[data-icon='search']")).toBeInTheDocument();
    expect(search).toHaveStyle({ width: "28px", height: "28px" });
    await user.click(screen.getByRole("button", { name: /搜索或输入命令/ }));
    expect(onSearch).toHaveBeenCalledOnce();
    // Exactly: search · polish · minimize · maximize · close.
    expect([...bar.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"))).toEqual([
      TITLE_BAR_SEARCH_LABEL,
      "润色 · 开/关",
      "最小化",
      "最大化",
      "关闭",
    ]);
  });

  it("marks the whole bar as a deep Tauri drag region and tags the platform", () => {
    render(<TitleBar title="首页" platform="windows" />);
    const bar = screen.getByTestId("title-bar");
    // `deep` (not the bare attribute): the bare form only drags on the container's own pixels,
    // so clicking the title text would not move the window.
    expect(bar).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(bar).toHaveAttribute("data-platform", "windows");
    expect(bar.querySelectorAll("[data-tauri-drag-region]")).toHaveLength(0);
    // Tauri's injected script toggles maximize on double-click; a second handler would undo it.
    expect(bar).not.toHaveAttribute("ondblclick");
    // No search, no right slot, no controls: nothing clickable.
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });

  it("hides the window controls without a Tauri window (plain browser) and on macOS", () => {
    const { rerender } = render(<TitleBar title="首页" platform="windows" />);
    expect(screen.queryByTestId("window-controls")).toBeNull();
    rerender(<TitleBar title="首页" platform="windows" controls={null} />);
    expect(screen.queryByTestId("window-controls")).toBeNull();
    rerender(<TitleBar title="首页" platform="macos" controls={controls()} />);
    expect(screen.queryByTestId("window-controls")).toBeNull();
    expect(screen.getByRole("heading", { name: "首页" })).toBeInTheDocument();
    expect(screen.getByTestId("title-bar")).toHaveAttribute("data-platform", "macos");
  });

  it("regression: at the window's left edge on macOS the title starts clear of the traffic lights", () => {
    // User feedback 2026-09-29 (the lights crowded what sat next to them): the same 80 px, in px,
    // as the sidebar brand row, whatever the 字号 setting makes a rem.
    const { rerender } = render(<TitleBar title="首页" platform="macos" trafficLights />);
    const row = screen.getByRole("heading", { name: "首页" }).parentElement;
    expect(row).toHaveClass(`pl-[${TRAFFIC_LIGHTS_CLEARANCE}px]`);
    rerender(<TitleBar title="首页" platform="macos" />);
    expect(screen.getByRole("heading", { name: "首页" }).parentElement).toHaveClass("pl-6");
  });

  it.each(["windows", "linux", "unknown"] as const)(
    "draws minimize · maximize · close on %s and forwards each click once",
    async (platform) => {
      const user = userEvent.setup();
      const c = controls();
      render(<TitleBar title="首页" platform={platform} controls={c} />);
      const cluster = screen.getByTestId("window-controls");
      const buttons = cluster.querySelectorAll("button");
      expect([...buttons].map((b) => b.getAttribute("aria-label"))).toEqual([
        "最小化",
        "最大化",
        "关闭",
      ]);
      for (const b of buttons) expect(b).toHaveClass("w-11.5", "h-full");
      expect(cluster.querySelector("svg[data-icon='minimize']")).toBeInTheDocument();
      expect(cluster.querySelector("svg[data-icon='maximize']")).toBeInTheDocument();
      expect(cluster.querySelector("svg[data-icon='close']")).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "最小化" }));
      await user.click(screen.getByRole("button", { name: "最大化" }));
      await user.click(screen.getByRole("button", { name: "关闭" }));
      expect(c.minimize).toHaveBeenCalledOnce();
      expect(c.toggleMaximize).toHaveBeenCalledOnce();
      expect(c.close).toHaveBeenCalledOnce();
      expect(screen.getByRole("button", { name: "关闭" })).toHaveClass("hover:bg-danger");
    },
  );

  it("switches the maximize button to 还原 with the restore icon while maximized", () => {
    const { rerender } = render(<TitleBar title="首页" platform="windows" controls={controls()} />);
    const normal = screen.getByRole("button", { name: "最大化" });
    expect(normal).toHaveAttribute("data-state", "normal");
    expect(normal.querySelector("svg")).toHaveAttribute("data-icon", "maximize");
    rerender(<TitleBar title="首页" platform="windows" controls={controls()} maximized />);
    const restore = screen.getByRole("button", { name: "还原" });
    expect(restore).toHaveAttribute("data-state", "maximized");
    expect(restore).toHaveAttribute("title", "还原");
    expect(restore.querySelector("svg")).toHaveAttribute("data-icon", "restore");
    expect(screen.queryByRole("button", { name: "最大化" })).toBeNull();
  });

  it("starts a drag for touch and pen pointers only, never for the mouse or on a control", () => {
    const c = controls();
    render(<TitleBar title="首页" platform="windows" controls={c} onSearch={vi.fn()} />);
    const bar = screen.getByTestId("title-bar");
    fireEvent.pointerDown(bar, { pointerType: "mouse" });
    expect(c.startDragging).not.toHaveBeenCalled();
    fireEvent.pointerDown(screen.getByRole("heading", { name: "首页" }), { pointerType: "touch" });
    fireEvent.pointerDown(bar, { pointerType: "pen" });
    expect(c.startDragging).toHaveBeenCalledTimes(2);
    // A finger on the close button or the search field is a tap, not a drag.
    fireEvent.pointerDown(screen.getByRole("button", { name: "关闭" }), { pointerType: "touch" });
    fireEvent.pointerDown(screen.getByRole("button", { name: /搜索/ }), { pointerType: "touch" });
    expect(c.startDragging).toHaveBeenCalledTimes(2);
  });

  it("regression: the title bar shows the compact engine and microphone readout inline and no second header row", () => {
    const { rerender } = render(<TitleBar title="首页" readouts={READOUTS} platform="windows" />);
    expect(TITLE_BAR_READOUT_MAX).toBe(2);
    const bar = screen.getByTestId("title-bar");
    // Inline, inside the drag strip, mono and subtle, hidden below `md`, at most two items.
    const readout = screen.getByTestId("title-bar-readout");
    expect(bar).toContainElement(readout);
    expect(readout).toHaveClass(
      "mono",
      "text-[11px]",
      "text-fg-subtle",
      "hidden",
      "md:flex",
      "truncate",
    );
    expect(readout).toHaveAttribute("aria-label", "语音模型与麦克风");
    expect(readout).toHaveTextContent("精确 · SenseVoice·Fifine K669");
    expect(screen.queryByText(/运行中/)).toBeNull();
    expect(readout.querySelectorAll("[data-tone='ok']")).toHaveLength(1);
    expect(screen.getByTitle("引擎 · 精确 · SenseVoice")).toBeInTheDocument();
    // Labels stay in the tooltip: the bar itself never prints them.
    expect(screen.queryByText("引擎")).toBeNull();
    expect(screen.queryByTestId("page-header")).toBeNull();
    expect(bar.nextElementSibling).toBeNull();
    // No readout: nothing but the title (and the flexible spacer).
    rerender(<TitleBar title="首页" readouts={[]} platform="windows" />);
    expect(screen.queryByTestId("title-bar-readout")).toBeNull();
    // The window control labels follow the locale.
    rerender(
      <I18nProvider locale="en">
        <TitleBar title="Home" onSearch={vi.fn()} platform="windows" controls={controls()} />
      </I18nProvider>,
    );
    expect([...screen.getAllByRole("button")].map((b) => b.getAttribute("aria-label"))).toEqual([
      "Search or type a command · Ctrl K",
      "Minimize",
      "Maximize",
      "Close",
    ]);
  });

  it("tolerates controls without a drag fallback", () => {
    const { startDragging: _omit, ...rest } = controls();
    render(<TitleBar title="首页" platform="linux" controls={rest} />);
    expect(() => {
      fireEvent.pointerDown(screen.getByTestId("title-bar"), { pointerType: "touch" });
    }).not.toThrow();
    expect(screen.getByTestId("window-controls")).toBeInTheDocument();
  });
});
