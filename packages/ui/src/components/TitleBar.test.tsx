import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Mock } from "vitest";
import { I18nProvider } from "../i18n/I18nProvider";
import { TRAFFIC_LIGHTS_CLEARANCE } from "./Sidebar";
import { TITLE_BAR_SEARCH_LABEL, TitleBar, type TitleBarControls } from "./TitleBar";

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

describe("TitleBar", () => {
  it("is the page header of board 01: the title over its subtitle, the command field, the right slot and the window controls", async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn();
    render(
      <TitleBar
        title="首页"
        subtitle="今天听写 6 次 · 节省 3 分 16 秒 · 历史记录 1,286 条"
        onSearch={onSearch}
        right={<button type="button" aria-label="AI 润色 · 开/关" />}
        platform="windows"
        controls={controls()}
        className="extra"
      />,
    );
    const bar = screen.getByTestId("title-bar");
    expect(bar).toHaveClass("extra");
    expect(bar.tagName).toBe("HEADER");
    const heading = screen.getByRole("heading", { name: "首页", level: 1 });
    expect(heading).toHaveClass("text-[26px]", "font-bold");
    expect(screen.getByTestId("title-bar-subtitle")).toHaveTextContent(
      "今天听写 6 次 · 节省 3 分 16 秒 · 历史记录 1,286 条",
    );
    // The subtitle is text here, so it is no group.
    expect(screen.getByTestId("title-bar-subtitle")).not.toHaveAttribute("role");
    // The command field is a pill naming what it does, with the shortcut in keycaps.
    const search = screen.getByTestId("title-bar-search");
    expect(search).toHaveAttribute("aria-label", TITLE_BAR_SEARCH_LABEL);
    expect(search).toHaveAttribute("title", "搜索或输入命令 · Ctrl K");
    expect(search).toHaveClass("rounded-pill", "bg-field", "h-12");
    expect(search).toHaveTextContent("输入命令或页面…");
    expect(within(search).getByLabelText("Ctrl K")).toBeInTheDocument();
    expect(search.querySelector("svg[data-icon='search']")).toBeInTheDocument();
    await user.click(search);
    expect(onSearch).toHaveBeenCalledOnce();
    // Exactly: search · polish · minimize · maximize · close, all inside the one header.
    expect([...bar.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"))).toEqual([
      TITLE_BAR_SEARCH_LABEL,
      "AI 润色 · 开/关",
      "最小化",
      "最大化",
      "关闭",
    ]);
    expect(bar.nextElementSibling).toBeNull();
  });

  it("marks the whole header as a deep Tauri drag region and tags the platform", () => {
    render(<TitleBar title="首页" platform="windows" />);
    const bar = screen.getByTestId("title-bar");
    // `deep` (not the bare attribute): the bare form only drags on the container's own pixels,
    // so clicking the title text would not move the window.
    expect(bar).toHaveAttribute("data-tauri-drag-region", "deep");
    expect(bar).toHaveAttribute("data-platform", "windows");
    expect(bar.querySelectorAll("[data-tauri-drag-region]")).toHaveLength(0);
    // Tauri's injected script toggles maximize on double-click; a second handler would undo it.
    expect(bar).not.toHaveAttribute("ondblclick");
    // No search, no right slot, no controls: nothing clickable, and no subtitle line.
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(screen.queryByTestId("title-bar-subtitle")).toBeNull();
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
    // User feedback 2026-09-29 (the lights crowded what sat next to them): the same clearance, in
    // px, as the sidebar brand row, whatever the 字号 setting makes a rem.
    const { rerender } = render(<TitleBar title="首页" platform="macos" trafficLights />);
    expect(screen.getByTestId("title-bar")).toHaveClass(`pl-[${TRAFFIC_LIGHTS_CLEARANCE}px]`);
    rerender(<TitleBar title="首页" platform="macos" />);
    expect(screen.getByTestId("title-bar")).toHaveClass("pl-6");
    rerender(
      <TitleBar title="首页" platform="macos" left={<button type="button">显示侧栏</button>} />,
    );
    expect(screen.getByTestId("title-bar")).toHaveClass("pl-3");
  });

  it.each(["windows", "linux", "unknown"] as const)(
    "draws minimize · maximize · close in the top-right corner on %s and forwards each click once",
    async (platform) => {
      const user = userEvent.setup();
      const c = controls();
      render(<TitleBar title="首页" platform={platform} controls={c} />);
      const cluster = screen.getByTestId("window-controls");
      expect(cluster).toHaveClass("absolute", "top-0", "right-0");
      const buttons = cluster.querySelectorAll("button");
      expect([...buttons].map((b) => b.getAttribute("aria-label"))).toEqual([
        "最小化",
        "最大化",
        "关闭",
      ]);
      for (const b of buttons) expect(b).toHaveClass("w-10", "h-full");
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

  it("regression: the speech model and microphone menus sit on the subtitle line, a named group that does not clip the menus they open", () => {
    // User request 2026-09-30 (switch them in place, on the title bar); board 01 puts them under
    // the title, in the same header.
    render(
      <TitleBar
        title="历史记录"
        subtitle={
          <>
            <button type="button">Qwen3-ASR-1.7B</button>
            <button type="button">Fifine K669</button>
          </>
        }
        subtitleLabel="语音模型与麦克风"
        platform="windows"
      />,
    );
    const bar = screen.getByTestId("title-bar");
    const line = screen.getByTestId("title-bar-subtitle");
    expect(bar).toContainElement(line);
    expect(line).toHaveAttribute("role", "group");
    expect(line).toHaveAttribute("aria-label", "语音模型与麦克风");
    // A menu opens below its button: the box must not hide what overflows it.
    expect(line).not.toHaveClass("truncate");
    expect(line).not.toHaveClass("overflow-hidden");
    expect(
      within(line)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Qwen3-ASR-1.7B", "Fifine K669"]);
    expect(bar.nextElementSibling).toBeNull();
  });

  it("follows the locale", () => {
    render(
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
