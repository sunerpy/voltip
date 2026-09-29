import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OptionCard } from "./OptionCard";

describe("OptionCard", () => {
  it("renders title, subtitle, badge, body and footer as a plain article without onSelect", () => {
    render(
      <OptionCard
        icon="cpu"
        title="精确 · SenseVoice"
        subtitle="sensevoice-small-int8"
        badge={<span>已安装</span>}
        footer={<button type="button">配置</button>}
        aria-label="sensevoice"
        selected>
        <span>RTF 0.063</span>
      </OptionCard>,
    );
    const card = screen.getByRole("article", { name: "sensevoice" });
    expect(card).toHaveTextContent("精确 · SenseVoice");
    expect(card).toHaveTextContent("sensevoice-small-int8");
    expect(card).toHaveTextContent("已安装");
    expect(card).toHaveTextContent("RTF 0.063");
    expect(screen.getByRole("button", { name: "配置" })).toBeInTheDocument();
    expect(card.querySelector('[data-icon="cpu"]')).not.toBeNull();
    expect(card).toHaveAttribute("data-selected", "true");
    expect(card).not.toHaveAttribute("role");
    expect(card).not.toHaveAttribute("aria-selected");
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("regression: a title cut by a narrow card shows whole on hover", () => {
    // Browser check 2026-09-29: 「按一下开始，再按一下结束」 was cut with no way to read it.
    render(<OptionCard icon="keyboard" title="按一下开始，再按一下结束" aria-label="toggle" />);
    const title = screen.getByText("按一下开始，再按一下结束");
    expect(title).toHaveClass("truncate");
    expect(title).toHaveAttribute("title", "按一下开始，再按一下结束");
  });

  it("an unselected article has no data-selected and a neutral icon tile", () => {
    render(<OptionCard icon="cloud" title="OpenAI" aria-label="openai" />);
    const card = screen.getByRole("article", { name: "openai" });
    expect(card).not.toHaveAttribute("data-selected");
    expect(card.querySelector('[data-icon="cloud"]')?.parentElement).toHaveClass("bg-inset");
    expect(card.className).not.toContain("ring-accent");
  });

  it("is an option with aria-selected when onSelect is given and selects on click, Enter and Space", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <div role="listbox" aria-label="models">
        <OptionCard title="A" onSelect={onSelect} aria-label="a" />
        <OptionCard title="B" onSelect={vi.fn()} aria-label="b" selected />
      </div>,
    );
    const a = screen.getByRole("option", { name: "a" });
    expect(a).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("option", { name: "b" })).toHaveAttribute("aria-selected", "true");
    expect(a).toHaveAttribute("tabindex", "0");
    await user.click(a);
    expect(onSelect).toHaveBeenCalledTimes(1);
    a.focus();
    await user.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledTimes(2);
    await user.keyboard(" ");
    expect(onSelect).toHaveBeenCalledTimes(3);
    await user.keyboard("{ArrowDown}");
    expect(onSelect).toHaveBeenCalledTimes(3);
  });

  it("does not select from footer clicks or keys, and disabled prevents selection", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onAction = vi.fn();
    render(
      <div role="listbox" aria-label="models">
        <OptionCard
          title="A"
          onSelect={onSelect}
          aria-label="a"
          footer={
            <button type="button" onClick={onAction}>
              更多
            </button>
          }>
          <input aria-label="别名" />
        </OptionCard>
        <OptionCard title="D" onSelect={onSelect} aria-label="d" disabled />
      </div>,
    );
    await user.click(screen.getByRole("button", { name: "更多" }));
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
    // Keys typed into a control inside the body are not card selection either.
    screen.getByRole("textbox", { name: "别名" }).focus();
    await user.keyboard("x{Enter} ");
    expect(onSelect).not.toHaveBeenCalled();
    screen.getByRole("button", { name: "更多" }).focus();
    await user.keyboard("{Enter}");
    expect(onAction).toHaveBeenCalledTimes(2);
    expect(onSelect).not.toHaveBeenCalled();
    const d = screen.getByRole("option", { name: "d" });
    expect(d).toHaveAttribute("aria-disabled", "true");
    expect(d).toHaveAttribute("tabindex", "-1");
    await user.click(d);
    expect(onSelect).not.toHaveBeenCalled();
    d.focus();
    await user.keyboard("{Enter}");
    expect(onSelect).not.toHaveBeenCalled();
  });
});
