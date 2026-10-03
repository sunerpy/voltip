import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { PresentationProvider, usePresentation } from "../presentation/PresentationProvider";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { Input, Textarea } from "./Input";
import { Menu } from "./Menu";
import { Segmented } from "./Segmented";
import { Toggle } from "./Toggle";

const TAP = "[-webkit-tap-highlight-color:transparent]";
const TARGET = ["relative", "after:h-[max(100%,44px)]", "after:w-[max(100%,44px)]"];

/** Every control the touch presentation changes, once. */
function Controls() {
  return (
    <>
      <Button>保存</Button>
      <Button variant="primary" size="sm">
        新建
      </Button>
      <Button variant="text">编辑</Button>
      <Button disabled>不可用</Button>
      <Button loading>处理中</Button>
      <IconButton icon="copy" label="复制" />
      <IconButton icon="trash" label="删除" tone="danger" size={28} />
      <Toggle checked label="开" onChange={vi.fn()} />
      <Toggle checked={false} label="关" onChange={vi.fn()} />
      <Toggle checked disabled label="锁定" onChange={vi.fn()} />
      <Segmented
        label="来源"
        value="local"
        onChange={vi.fn()}
        options={[
          { value: "local", label: "本地" },
          { value: "cloud", label: "云端" },
          { value: "off", label: "关", disabled: true },
        ]}
      />
      <Input label="名称" icon="search" keys="Ctrl F" />
      <Textarea label="正文" />
      <Menu
        trigger="校对"
        label="AI 预设"
        triggerClassName="h-8 px-2"
        onSelect={vi.fn()}
        sections={[
          {
            items: [
              { kind: "radio", id: "a", label: "校对", checked: true },
              { kind: "radio", id: "b", label: "翻译", checked: false, disabled: true },
              { kind: "action", id: "c", label: "管理预设…" },
            ],
          },
        ]}
      />
    </>
  );
}

const TOUCH_ONLY =
  /touch-manipulation|tap-highlight|(^|\s)(active|enabled|after|before):|min-h-\[44px\]|min-w-\[44px\]/;

describe("controls under the touch presentation", () => {
  it("are unchanged without a provider and under the native presentation", async () => {
    const user = userEvent.setup();
    const plain = render(<Controls />);
    await user.click(screen.getByRole("button", { name: "AI 预设" }));
    const html = plain.container.innerHTML;
    const touchOnly = [...plain.container.querySelectorAll("[class]")]
      .map((el) => el.getAttribute("class") ?? "")
      .filter((cls) => TOUCH_ONLY.test(cls));
    expect(touchOnly).toEqual([]);
    // The menu's rows keep their `h-8`.
    const rows = screen.getByRole("menu").querySelectorAll("button");
    expect(rows).toHaveLength(3);
    for (const row of rows) expect(row).toHaveClass("h-8");
    // A click beside the input does not move the focus into it.
    await user.click(screen.getByLabelText("名称").parentElement as HTMLElement);
    expect(screen.getByLabelText("名称")).not.toHaveFocus();
    plain.unmount();
    const native = render(
      <PresentationProvider value="native">
        <Controls />
      </PresentationProvider>,
    );
    await user.click(screen.getByRole("button", { name: "AI 预设" }));
    // React's generated ids differ between the two renders; nothing else does.
    const ids = /_r_[0-9a-z]+_/g;
    expect(native.container.innerHTML.replaceAll(ids, "")).toBe(html.replaceAll(ids, ""));
  });

  it("buttons keep their size, take a 44 px target, drop the tap flash and show a pressed state", () => {
    render(
      <PresentationProvider value="touch">
        <Controls />
      </PresentationProvider>,
    );
    const save = screen.getByRole("button", { name: "保存" });
    expect(save).toHaveClass("h-8", "touch-manipulation", TAP, ...TARGET, "active:bg-inset2");
    const create = screen.getByRole("button", { name: "新建" });
    expect(create).toHaveClass("h-7", ...TARGET, "active:opacity-80");
    expect(screen.getByRole("button", { name: "编辑" })).toHaveClass("active:opacity-60");
    // Nothing to press on a button that does nothing.
    for (const name of ["不可用", "处理中"]) {
      const button = screen.getByRole("button", { name });
      expect(button).toHaveClass(...TARGET);
      expect(button.className).not.toMatch(/active:/);
    }
    const copy = screen.getByRole("button", { name: "复制" });
    expect(copy).toHaveStyle({ width: "24px", height: "24px" });
    expect(copy).toHaveClass(...TARGET, TAP, "enabled:active:bg-inset2", "enabled:active:text-fg");
    expect(screen.getByRole("button", { name: "删除" })).toHaveClass("enabled:active:text-danger");
  });

  it("switches and segments take 44 px targets and pressed states, labels no tap flash", () => {
    render(
      <PresentationProvider value="touch">
        <Controls />
      </PresentationProvider>,
    );
    const on = screen.getByRole("switch", { name: "开" });
    expect(on).toHaveClass("h-[19px]", "w-8", ...TARGET, "active:opacity-80");
    expect(on.closest("label")).toHaveClass("touch-manipulation", TAP, "select-none");
    expect(screen.getByRole("switch", { name: "关" })).toHaveClass("active:bg-fg/20");
    expect(screen.getByRole("switch", { name: "锁定" }).className).not.toMatch(/active:/);
    const group = screen.getByRole("radiogroup", { name: "来源" });
    expect(group).toHaveClass("touch-manipulation", TAP, "select-none");
    for (const segment of within(group).getAllByRole("radio"))
      expect(segment).toHaveClass(
        "min-w-[44px]",
        "relative",
        "after:inset-x-0",
        "after:h-[max(100%,44px)]",
      );
    // Only a segment that changes the choice has a pressed state.
    expect(screen.getByRole("radio", { name: "云端" })).toHaveClass("active:bg-inset2");
    expect(screen.getByRole("radio", { name: "本地" }).className).not.toMatch(/active:/);
    expect(screen.getByRole("radio", { name: "关" }).className).not.toMatch(/active:/);
  });

  it("menu rows are 44 px tall, and its trigger takes a 44 px target", async () => {
    const user = userEvent.setup();
    render(
      <PresentationProvider value="touch">
        <Controls />
      </PresentationProvider>,
    );
    const trigger = screen.getByRole("button", { name: "AI 预设" });
    expect(trigger).toHaveClass("h-8", "px-2", ...TARGET, "enabled:active:opacity-70");
    expect(trigger.parentElement).toHaveClass("touch-manipulation", TAP, "select-none");
    await user.click(trigger);
    const menu = screen.getByRole("menu");
    const rows = menu.querySelectorAll("button");
    expect(rows).toHaveLength(3);
    for (const row of rows) {
      expect(row).toHaveClass("min-h-[44px]");
      expect(row).not.toHaveClass("h-8");
    }
    expect(within(menu).getByRole("menuitemradio", { name: "校对" })).toHaveClass(
      "active:bg-inset2",
    );
    expect(within(menu).getByRole("menuitem", { name: "管理预设…" })).toHaveClass(
      "active:bg-inset2",
    );
    expect(within(menu).getByRole("menuitemradio", { name: "翻译" }).className).not.toMatch(
      /active:/,
    );
  });

  it("a text field takes taps 44 px tall: a tap beside the text focuses it, a tap on it places the caret", async () => {
    const user = userEvent.setup();
    render(
      <PresentationProvider value="touch">
        <Controls />
      </PresentationProvider>,
    );
    const input = screen.getByLabelText("名称");
    const field = input.parentElement as HTMLElement;
    expect(field).toHaveClass("h-8", "touch-manipulation", "relative", "before:h-[max(100%,44px)]");
    // Drawn over the field's target, so the input itself gets the taps on it.
    expect(input).toHaveClass("relative");
    expect(input.closest(".flex-col")).toHaveClass(TAP);
    // A tap on the icon (or the padding, or the keycaps) lands on the field: the input takes focus.
    await user.click(field.querySelector('[data-icon="search"]') as Element);
    expect(input).toHaveFocus();
    input.blur();
    await user.click(input);
    expect(input).toHaveFocus();
    const textarea = screen.getByLabelText("正文");
    expect(textarea).toHaveClass("touch-manipulation");
    expect(textarea.parentElement).toHaveClass(TAP);
  });
});

describe("usePresentation", () => {
  function Shows() {
    return <span>{usePresentation()}</span>;
  }

  it("is native without a provider, and the nearest provider's value inside one", () => {
    const wrap = (children: ReactNode) => <div data-testid="box">{children}</div>;
    render(wrap(<Shows />));
    expect(screen.getByTestId("box")).toHaveTextContent("native");
    render(
      <PresentationProvider value="touch">
        <span data-testid="touch">
          <Shows />
        </span>
        <PresentationProvider value="native">
          <span data-testid="inner">
            <Shows />
          </span>
        </PresentationProvider>
      </PresentationProvider>,
    );
    expect(screen.getByTestId("touch")).toHaveTextContent("touch");
    expect(screen.getByTestId("inner")).toHaveTextContent("native");
  });
});
