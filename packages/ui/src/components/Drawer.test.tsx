import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Dialog, dismissTopDialog } from "./Dialog";
import { Drawer, DrawerBody, DrawerFooter } from "./Drawer";

/** A button that opens a drawer with two fields and an action, as a provider card does. */
function Opener({ onClose = () => undefined }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => {
          setOpen(true);
        }}>
        打开 Groq
      </button>
      <Drawer
        open={open}
        onClose={() => {
          onClose();
          setOpen(false);
        }}
        title="Groq"
        icon="cloud"
        badges={<span>缺少密钥</span>}
        description="Groq 托管的 Whisper 与开源大模型。"
        data-testid="drawer">
        <DrawerBody>
          <label>
            接口地址
            <input />
          </label>
          <label>
            API 密钥
            <input />
          </label>
        </DrawerBody>
        <DrawerFooter>
          <button type="button">保存</button>
        </DrawerFooter>
      </Drawer>
    </>
  );
}

describe("Drawer", () => {
  it("opens over a scrim, named by its title and described by its sentence", async () => {
    const user = userEvent.setup();
    render(<Opener />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(screen.getByRole("button", { name: "打开 Groq" }));
    const drawer = screen.getByRole("dialog", { name: "Groq" });
    expect(drawer).toHaveAttribute("aria-modal", "true");
    expect(drawer).toHaveAccessibleDescription("Groq 托管的 Whisper 与开源大模型。");
    expect(drawer).toHaveTextContent("缺少密钥");
    expect(drawer).toHaveAttribute("data-testid", "drawer");
    // Never wider than the window: a fixed width would not fit a narrow one.
    expect(drawer.style.width).toMatch(/^min\(480px, .*100vw/);
    expect(drawer).toHaveFocus();
  });

  it("closes on the close button, Esc and the scrim, never on a click inside, and gives the focus back", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Opener onClose={onClose} />);
    const opener = screen.getByRole("button", { name: "打开 Groq" });
    await user.click(opener);
    await user.click(screen.getByLabelText("接口地址"));
    expect(onClose).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "关闭" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
    await user.click(opener);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(opener).toHaveFocus();
    await user.click(opener);
    await user.click(screen.getByTestId("drawer-scrim"));
    expect(onClose).toHaveBeenCalledTimes(3);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps Tab inside the panel, both ways", async () => {
    const user = userEvent.setup();
    render(<Opener />);
    await user.click(screen.getByRole("button", { name: "打开 Groq" }));
    const close = screen.getByRole("button", { name: "关闭" });
    const save = screen.getByRole("button", { name: "保存" });
    await user.tab();
    expect(close).toHaveFocus();
    await user.tab();
    expect(screen.getByLabelText("接口地址")).toHaveFocus();
    await user.tab();
    await user.tab();
    expect(save).toHaveFocus();
    await user.tab();
    expect(close).toHaveFocus();
    await user.tab({ shift: true });
    expect(save).toHaveFocus();
  });

  it("leaves Esc to a dialog opened over it, and the phone's back closes the one on top", async () => {
    const user = userEvent.setup();
    const closeDrawer = vi.fn();
    const closeDialog = vi.fn();
    const { rerender } = render(
      <Drawer open onClose={closeDrawer} title="自定义接口">
        <DrawerBody>
          <p>正文</p>
        </DrawerBody>
      </Drawer>,
    );
    rerender(
      <>
        <Drawer open onClose={closeDrawer} title="自定义接口">
          <DrawerBody>
            <p>正文</p>
          </DrawerBody>
        </Drawer>
        <Dialog open title="删除密钥？" onClose={closeDialog} actions={<button>删除</button>} />
      </>,
    );
    await user.keyboard("{Escape}");
    expect(closeDialog).toHaveBeenCalledTimes(1);
    expect(closeDrawer).not.toHaveBeenCalled();
    rerender(
      <Drawer open onClose={closeDrawer} title="自定义接口">
        <DrawerBody>
          <p>正文</p>
        </DrawerBody>
      </Drawer>,
    );
    expect(dismissTopDialog()).toBe(true);
    expect(closeDrawer).toHaveBeenCalledTimes(1);
  });

  it("draws the item in use on a soft accent block, and takes a width of its own", () => {
    render(
      <Drawer
        open
        onClose={() => undefined}
        title="内置服务"
        icon="sparkles"
        iconTone="accent"
        width={720}>
        <DrawerBody>
          <p>正文</p>
        </DrawerBody>
      </Drawer>,
    );
    const drawer = screen.getByRole("dialog", { name: "内置服务" });
    expect(drawer.style.width).toMatch(/^min\(720px, .*100vw/);
    const block = drawer.querySelector("header svg")?.parentElement;
    expect(block?.className).toContain("bg-accent-soft");
    expect(block?.className).toContain("text-accent-text");
    expect(drawer).not.toHaveAttribute("aria-describedby");
  });
});
