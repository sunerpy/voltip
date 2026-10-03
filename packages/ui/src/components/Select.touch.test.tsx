import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { PresentationProvider } from "../presentation/PresentationProvider";
import { Dialog, dismissTopDialog } from "./Dialog";
import { Select, type SelectOption } from "./Select";

const LANGUAGES: SelectOption<string>[] = [
  { value: "auto", label: "自动检测" },
  { value: "zh", label: "中文 · zh" },
  { value: "yue", label: "粤语 · yue", disabled: true },
  { value: "en", label: "English" },
];

function touch(ui: ReactNode) {
  return render(<PresentationProvider value="touch">{ui}</PresentationProvider>);
}

function renderLanguages(props: { value?: string; onChange?: (value: string) => void } = {}) {
  const onChange = props.onChange ?? vi.fn();
  const view = touch(
    <>
      <button type="button">before</button>
      <Select
        label="识别语言"
        options={LANGUAGES}
        value={props.value ?? "zh"}
        onChange={onChange}
      />
      <button type="button">after</button>
    </>,
  );
  return { ...view, onChange, trigger: screen.getByRole("button", { name: "识别语言" }) };
}

/** A box as `getBoundingClientRect()` reports it. */
function rect(top: number, left: number, width: number, height: number): DOMRect {
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

/** jsdom lays nothing out: give the trigger a box, the list a natural size and its rows 44 px. */
function fakeLayout(trigger: { current: DOMRect }, list = { width: 160, height: 0, view: 0 }) {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return this.getAttribute("aria-haspopup") === "listbox" ? trigger.current : rect(0, 0, 0, 0);
  });
  const isList = (el: Element) => el.getAttribute("role") === "listbox";
  const rowIndex = (el: Element) =>
    el.getAttribute("role") === "option" && el.parentElement
      ? [...el.parentElement.children].indexOf(el)
      : -1;
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return isList(this) ? list.width : 0;
  });
  vi.spyOn(Element.prototype, "scrollHeight", "get").mockImplementation(function (this: Element) {
    return isList(this) ? list.height : 0;
  });
  vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
    return isList(this) ? list.view : 0;
  });
  // 4 px of padding, then the rows.
  vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    const at = rowIndex(this);
    return at < 0 ? 0 : 4 + at * 44;
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return rowIndex(this) < 0 ? 0 : 44;
  });
}

describe("Select under the touch presentation", () => {
  it("regression: on the phone a select opens its own list, not the platform's radio-button picker", async () => {
    // User report 2026-10-03: every dropdown on the phone opened Android's old dialog of radio
    // buttons (a native <select> in the WebView). The touch presentation draws its own list.
    const user = userEvent.setup();
    const { trigger, container } = renderLanguages();
    expect(container.querySelector("select")).toBeNull();
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).not.toHaveAttribute("aria-controls");
    // The label names it; the chosen option, which the name would hide, is its description.
    expect(trigger).toHaveAccessibleDescription("中文 · zh");
    expect(trigger).toHaveTextContent("中文 · zh");
    await user.click(trigger);
    const list = screen.getByRole("listbox", { name: "识别语言" });
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(trigger).toHaveAttribute("aria-controls", list.id);
    const options = within(list).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(LANGUAGES.map((o) => o.label));
    expect(within(list).getByRole("option", { selected: true })).toHaveTextContent("中文 · zh");
    // The chosen row carries the check mark and the focus.
    expect(list.querySelectorAll('[data-icon="check"]')).toHaveLength(1);
    expect(within(list).getByRole("option", { name: "中文 · zh" })).toHaveFocus();
    expect(within(list).getByRole("option", { name: "中文 · zh" })).toContainElement(
      list.querySelector('[data-icon="check"]') as HTMLElement,
    );
  });

  it("draws the trigger as the native select and the list as Menu does, rows 44 px tall", async () => {
    const user = userEvent.setup();
    const { container } = touch(
      <Select label="模型" size="sm" mono options={LANGUAGES} value="en" onChange={vi.fn()} />,
    );
    const trigger = screen.getByRole("button", { name: "模型" });
    expect(trigger).toHaveClass(
      "hairline",
      "rounded-6",
      "bg-surface",
      "h-7",
      "text-[12px]",
      "mono",
    );
    expect(trigger).toHaveClass("pr-7", "pl-2.5", "w-full", "col-start-1", "row-start-1");
    // The same hidden copy of every label sets the width, sharing the trigger's grid cell.
    const sizer = container.querySelector("[data-select-sizer]");
    expect(sizer?.parentElement).toBe(trigger.parentElement);
    expect([...(sizer?.children ?? [])].map((line) => line.textContent)).toEqual(
      LANGUAGES.map((o) => o.label),
    );
    expect(sizer).toHaveClass("text-[12px]", "mono", "min-w-max");
    expect(container.querySelector('[data-icon="chevronDown"]')).not.toBeNull();
    await user.click(trigger);
    const list = screen.getByRole("listbox");
    expect(list).toHaveClass("fixed", "hairline", "shadow-pop", "rounded-10", "overflow-y-auto");
    for (const option of within(list).getAllByRole("option")) {
      expect(option).toHaveClass("min-h-[44px]");
      expect(option.lastElementChild).toHaveClass("mono");
    }
    const md = touch(
      <Select aria-label="大小" options={LANGUAGES} value="en" onChange={vi.fn()} />,
    );
    expect(within(md.container).getByRole("button", { name: "大小" })).toHaveClass(
      "h-8",
      "text-[13px]",
    );
  });

  it("chooses with a tap, closes, gives the focus back, and calls onChange only for a new choice", async () => {
    const user = userEvent.setup();
    const { trigger, onChange } = renderLanguages();
    await user.click(trigger);
    await user.click(screen.getByRole("option", { name: "English" }));
    expect(onChange).toHaveBeenCalledWith("en");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    // The option already chosen: the list closes, nothing changes.
    await user.click(trigger);
    await user.click(screen.getByRole("option", { name: "中文 · zh" }));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("listbox")).toBeNull();
    // A second tap on the trigger closes an open list.
    await user.click(trigger);
    await user.click(trigger);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("works from the keyboard: ↓ ↑ Home End skip what cannot be chosen, Enter and Space choose, Esc keeps the value", async () => {
    const user = userEvent.setup();
    const { trigger, onChange } = renderLanguages();
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    const list = screen.getByRole("listbox");
    const row = (name: string) => within(list).getByRole("option", { name });
    expect(row("中文 · zh")).toHaveFocus();
    // Past the disabled 粤语, and no further than the last row.
    await user.keyboard("{ArrowDown}");
    expect(row("English")).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(row("English")).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(row("中文 · zh")).toHaveFocus();
    await user.keyboard("{Home}");
    expect(row("自动检测")).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(row("自动检测")).toHaveFocus();
    await user.keyboard("{End}");
    expect(row("English")).toHaveFocus();
    // Keys the list does not use change nothing.
    await user.keyboard("x");
    expect(row("English")).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onChange).toHaveBeenLastCalledWith("en");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveFocus();
    // ↑ opens it too; Space chooses.
    await user.keyboard("{ArrowUp}");
    await user.keyboard("{Home}");
    await user.keyboard(" ");
    expect(onChange).toHaveBeenLastCalledWith("auto");
    expect(trigger).toHaveFocus();
    // Esc closes without choosing, and the focus is back on the trigger.
    await user.keyboard("{Enter}");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveFocus();
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("closes on Tab, and the focus moves on from the trigger", async () => {
    const user = userEvent.setup();
    const { trigger, onChange } = renderLanguages();
    await user.click(trigger);
    await user.tab();
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("button", { name: "after" })).toHaveFocus();
    await user.click(trigger);
    await user.tab({ shift: true });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("button", { name: "before" })).toHaveFocus();
    // Tab with the list closed is an ordinary Tab.
    trigger.focus();
    await user.tab();
    expect(screen.getByRole("button", { name: "after" })).toHaveFocus();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows rows that cannot be chosen but never chooses them", async () => {
    const user = userEvent.setup();
    const { trigger, onChange } = renderLanguages();
    await user.click(trigger);
    const disabled = screen.getByRole("option", { name: "粤语 · yue" });
    expect(disabled).toBeDisabled();
    expect(disabled).toHaveAttribute("aria-disabled", "true");
    expect(disabled).toHaveClass("text-fg-subtle");
    expect(disabled).not.toHaveClass("active:bg-inset2");
    await user.click(disabled);
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("listbox")).toBeInTheDocument();
  });

  it("opens on the first row that can be chosen when the chosen one cannot be, and on the list when none can", async () => {
    const user = userEvent.setup();
    touch(
      <>
        <Select aria-label="一" options={LANGUAGES} value="yue" onChange={vi.fn()} />
        <Select
          aria-label="二"
          options={[{ value: "a", label: "甲", disabled: true }]}
          value="a"
          onChange={vi.fn()}
        />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "一" }));
    expect(screen.getByRole("option", { name: "自动检测" })).toHaveFocus();
    // The disabled choice is still the one marked chosen.
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("粤语 · yue");
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "二" }));
    const list = screen.getByRole("listbox", { name: "二" });
    expect(list).toHaveFocus();
    // Nothing to move to.
    await user.keyboard("{ArrowDown}{ArrowUp}{End}");
    expect(list).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("shows the first option that can be chosen when the value matches none, and choosing it replaces the value", async () => {
    // A value no option has (the phone's pinned scene after the scene was deleted): the trigger
    // shows what a native select would, and a tap on that option is a choice like any other, so
    // the stale value goes (the phone's talk card clears its scene that way).
    const user = userEvent.setup();
    const onChange = vi.fn();
    touch(
      <Select
        aria-label="语言"
        options={[{ value: "x", label: "已停用", disabled: true }, ...LANGUAGES]}
        value="gone"
        onChange={onChange}
      />,
    );
    const trigger = screen.getByRole("button", { name: "语言" });
    expect(trigger).toHaveTextContent("自动检测");
    await user.click(trigger);
    expect(screen.getByRole("option", { selected: true })).toHaveTextContent("自动检测");
    await user.click(screen.getByRole("option", { name: "自动检测" }));
    expect(onChange).toHaveBeenCalledWith("auto");
    await user.click(trigger);
    await user.click(screen.getByRole("option", { name: "English" }));
    expect(onChange).toHaveBeenLastCalledWith("en");
    // No options at all: an empty trigger, an empty list.
    touch(<Select aria-label="空" options={[]} value="" onChange={vi.fn()} />);
    const empty = screen.getByRole("button", { name: "空" });
    expect(empty).toHaveTextContent("");
    await user.click(empty);
    expect(screen.getByRole("listbox", { name: "空" })).toHaveFocus();
  });

  it("closes on a tap outside: the tap lands on a backdrop, so nothing under it is pressed", async () => {
    const user = userEvent.setup();
    const { trigger, onChange, container } = renderLanguages();
    await user.click(trigger);
    const backdrop = container.querySelector("[data-select-backdrop]");
    if (!(backdrop instanceof HTMLElement)) throw new Error("no backdrop");
    expect(backdrop).toHaveClass("fixed", "inset-0");
    expect(backdrop).toHaveAttribute("aria-hidden", "true");
    // It comes before the list, which is drawn over it.
    expect(backdrop.nextElementSibling).toBe(screen.getByRole("listbox"));
    await user.click(backdrop);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(container.querySelector("[data-select-backdrop]")).toBeNull();
    expect(trigger).toHaveFocus();
    // A press on something drawn over the backdrop (a toast) closes it too, and the focus goes
    // where the press went.
    await user.click(trigger);
    fireEvent.pointerDown(screen.getByRole("button", { name: "after" }));
    expect(screen.queryByRole("listbox")).toBeNull();
    // A press inside the list keeps it open.
    await user.click(trigger);
    fireEvent.pointerDown(screen.getByRole("listbox"));
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("closes on the phone's system back before the dialog it opened in, which Esc does too", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    touch(
      <Dialog open title="编辑场景" onClose={onClose} actions={<button type="button">完成</button>}>
        <Select aria-label="AI 润色" options={LANGUAGES} value="zh" onChange={vi.fn()} />
      </Dialog>,
    );
    const trigger = screen.getByRole("button", { name: "AI 润色" });
    await user.click(trigger);
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    let dismissed = false;
    act(() => {
      dismissed = dismissTopDialog();
    });
    expect(dismissed).toBe(true);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveFocus();
    expect(onClose).not.toHaveBeenCalled();
    // The next back closes the dialog.
    act(() => {
      dismissed = dismissTopDialog();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    // Esc in the list closes the list only; the next Esc closes the dialog.
    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("dialog", { name: "编辑场景" })).toBeInTheDocument();
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("reports when back finds no list open, and leaves a dialog opened over the list to Esc", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const page = (confirming: boolean) => (
      <PresentationProvider value="touch">
        <Select label="识别语言" options={LANGUAGES} value="zh" onChange={vi.fn()} />
        <Dialog
          open={confirming}
          title="确认"
          onClose={onClose}
          actions={<button type="button">好</button>}
        />
      </PresentationProvider>
    );
    const { rerender } = render(page(false));
    const trigger = screen.getByRole("button", { name: "识别语言" });
    await user.click(trigger);
    act(() => {
      expect(dismissTopDialog()).toBe(true);
    });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(dismissTopDialog()).toBe(false);
    // A dialog opened while the list is open is on top: Esc closes the dialog, the list stays.
    await user.click(trigger);
    rerender(page(true));
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    // Gone, the dialog leaves the list on top again.
    rerender(page(false));
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("opens under the trigger, or flips above it near the bottom of the screen, and follows it", async () => {
    const user = userEvent.setup();
    // jsdom's window is 1024 × 768.
    const box = { current: rect(100, 16, 200, 28) };
    fakeLayout(box, { width: 160, height: 228, view: 0 });
    const { trigger } = renderLanguages();
    await user.click(trigger);
    const list = screen.getByRole("listbox");
    expect(list.style.top).toBe("132px");
    expect(list.style.bottom).toBe("");
    expect(list.style.left).toBe("16px");
    expect(list.style.minWidth).toBe("200px");
    expect(list.style.maxWidth).toBe("1000px");
    expect(list.style.maxHeight).toBe("320px");
    expect(list.style.visibility).toBe("");
    // The page scrolls the trigger down: the list follows it.
    box.current = rect(300, 16, 200, 28);
    fireEvent.scroll(window);
    expect(list.style.top).toBe("332px");
    // Its own rows scrolling moves nothing.
    box.current = rect(700, 16, 200, 28);
    fireEvent.scroll(list);
    expect(list.style.top).toBe("332px");
    // The window resizes with the trigger near the bottom: no room below, so above it.
    fireEvent(window, new Event("resize"));
    expect(list.style.top).toBe("");
    expect(list.style.bottom).toBe("72px");
    await user.keyboard("{Escape}");
    // Reopened near the bottom: above from the start.
    await user.click(trigger);
    expect(screen.getByRole("listbox").style.bottom).toBe("72px");
  });

  it("scrolls the chosen row into the middle of a long list, and the rows the keys reach into view", async () => {
    const user = userEvent.setup();
    const many = Array.from({ length: 8 }, (_, i) => ({ value: `m${i}`, label: `${i + 1} 分钟` }));
    // Eight 44 px rows and 8 px of padding; three rows show.
    fakeLayout({ current: rect(100, 16, 200, 28) }, { width: 160, height: 360, view: 132 });
    touch(<Select aria-label="时长" options={many} value="m6" onChange={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "时长" }));
    const list = screen.getByRole("listbox");
    expect(screen.getByRole("option", { name: "7 分钟" })).toHaveFocus();
    // Row 7 starts at 268: centred in a 132 px view.
    expect(list.scrollTop).toBe(224);
    await user.keyboard("{ArrowDown}");
    expect(list.scrollTop).toBe(224);
    await user.keyboard("{Home}");
    expect(list.scrollTop).toBe(4);
  });

  it("puts the text markers on the chosen label and on every option's label, not on the trigger", async () => {
    const user = userEvent.setup();
    const { container } = touch(
      <>
        <Select
          label="设备"
          options={[
            { value: "a", label: "Fifine K669" },
            { value: "b", label: "我的耳机" },
          ]}
          value="a"
          onChange={vi.fn()}
          data-user-text=""
          data-testid="device"
        />
        <Select
          aria-label="界面语言"
          options={[
            { value: "zh-CN", label: "简体中文" },
            { value: "en", label: "English" },
          ]}
          value="en"
          onChange={vi.fn()}
          data-endonyms=""
        />
      </>,
    );
    const device = screen.getByTestId("device");
    expect(device).toBe(screen.getByRole("button", { name: "设备" }));
    expect(device).not.toHaveAttribute("data-user-text");
    expect(within(device).getByText("Fifine K669")).toHaveAttribute("data-user-text");
    expect(container.querySelector("[data-select-sizer][data-user-text]")).not.toBeNull();
    await user.click(device);
    const list = screen.getByRole("listbox", { name: "设备" });
    expect(list).not.toHaveAttribute("data-user-text");
    for (const name of ["Fifine K669", "我的耳机"])
      expect(within(list).getByText(name)).toHaveAttribute("data-user-text");
    await user.keyboard("{Escape}");
    const language = screen.getByRole("button", { name: "界面语言" });
    expect(within(language).getByText("English")).toHaveAttribute("data-endonyms");
    await user.click(language);
    const languages = screen.getByRole("listbox", { name: "界面语言" });
    expect(within(languages).getByText("简体中文")).toHaveAttribute("data-endonyms");
    expect(within(languages).getByText("简体中文")).not.toHaveAttribute("data-user-text");
  });

  it("names the trigger and the list from aria-label or aria-labelledby, and keeps a caller's description", async () => {
    const user = userEvent.setup();
    touch(
      <>
        <span id="heading">保留最近</span>
        <span id="note">超出后删除最旧的记录</span>
        <Select
          aria-labelledby="heading"
          aria-describedby="note"
          options={LANGUAGES}
          value="en"
          onChange={vi.fn()}
        />
        <Select options={LANGUAGES} value="auto" onChange={vi.fn()} data-testid="unnamed" />
      </>,
    );
    const trigger = screen.getByRole("button", { name: "保留最近" });
    expect(trigger).toHaveAccessibleDescription("English 超出后删除最旧的记录");
    await user.click(trigger);
    expect(screen.getByRole("listbox", { name: "保留最近" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    // Without a name its text is its name, and nothing describes it twice.
    const unnamed = screen.getByTestId("unnamed");
    expect(unnamed).toHaveAccessibleName("自动检测");
    expect(unnamed).not.toHaveAttribute("aria-describedby");
  });

  it("does not open while disabled, and closes when disabled while open", async () => {
    const user = userEvent.setup();
    const props = { label: "场景", options: LANGUAGES, value: "zh", onChange: vi.fn() };
    const { rerender } = touch(<Select {...props} disabled />);
    const trigger = screen.getByRole("button", { name: "场景" });
    expect(trigger).toBeDisabled();
    expect(trigger).not.toHaveClass("active:bg-inset");
    await user.click(trigger);
    expect(screen.queryByRole("listbox")).toBeNull();
    rerender(
      <PresentationProvider value="touch">
        <Select {...props} />
      </PresentationProvider>,
    );
    expect(trigger).toHaveClass("active:bg-inset");
    await user.click(trigger);
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    rerender(
      <PresentationProvider value="touch">
        <Select {...props} disabled />
      </PresentationProvider>,
    );
    expect(screen.queryByRole("listbox")).toBeNull();
    rerender(
      <PresentationProvider value="touch">
        <Select {...props} />
      </PresentationProvider>,
    );
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("lets a test choose with user.selectOptions on the open list", async () => {
    const user = userEvent.setup();
    const { trigger, onChange } = renderLanguages();
    await user.click(trigger);
    await user.selectOptions(screen.getByRole("listbox"), "en");
    expect(onChange).toHaveBeenCalledWith("en");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("drops the touch states of the trigger and the tap flash of the whole field", () => {
    const { container, trigger } = renderLanguages();
    expect(trigger).toHaveClass("touch-manipulation", "relative", "after:h-[max(100%,44px)]");
    expect(container.querySelector(".flex.flex-col")).toHaveClass(
      "[-webkit-tap-highlight-color:transparent]",
    );
  });
});

describe("Select under the native presentation", () => {
  it("is the platform's <select> without a provider and with value native", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const { unmount } = render(
      <Select label="识别语言" options={LANGUAGES} value="zh" onChange={onChange} />,
    );
    const select = screen.getByRole("combobox", { name: "识别语言" });
    expect(select.tagName).toBe("SELECT");
    expect(select).not.toHaveAttribute("aria-haspopup");
    expect(select.className).not.toContain("touch-manipulation");
    unmount();
    render(
      <PresentationProvider value="native">
        <Select label="识别语言" options={LANGUAGES} value="zh" onChange={onChange} />
      </PresentationProvider>,
    );
    const native = screen.getByRole("combobox", { name: "识别语言" });
    expect(native.tagName).toBe("SELECT");
    expect(screen.getAllByRole("option")).toHaveLength(LANGUAGES.length);
    expect(screen.queryByRole("button")).toBeNull();
    await user.selectOptions(native, "en");
    expect(onChange).toHaveBeenCalledWith("en");
  });
});
