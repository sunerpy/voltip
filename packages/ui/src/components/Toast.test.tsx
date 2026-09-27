import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Toast, ToastViewport, useToasts } from "./Toast";

describe("Toast", () => {
  it("renders message, action with keycap and dismisses after the action", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    const onAction = vi.fn();
    render(
      <Toast
        toast={{
          id: "1",
          message: "已取消 · 录音已丢弃",
          duration: 5000,
          action: { label: "撤销", keys: "Z", onClick: onAction },
        }}
        onDismiss={onDismiss}
      />,
    );
    await user.click(screen.getByRole("button", { name: /撤销/ }));
    expect(onAction).toHaveBeenCalled();
    expect(onDismiss).toHaveBeenCalledWith("1");
    render(
      <Toast
        toast={{ id: "2", message: "失败", duration: 3000, tone: "danger" }}
        onDismiss={onDismiss}
      />,
    );
    expect(screen.getByRole("alert")).toHaveAttribute("data-tone", "danger");
  });

  it("viewport shows the newest three and nothing when empty", () => {
    const { container, rerender } = render(<ToastViewport toasts={[]} onDismiss={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
    rerender(
      <ToastViewport
        toasts={[1, 2, 3, 4].map((n) => ({ id: String(n), message: `t${n}`, duration: 1000 }))}
        onDismiss={vi.fn()}
      />,
    );
    expect(screen.queryByText("t1")).toBeNull();
    expect(screen.getByText("t4")).toBeInTheDocument();
  });

  it("useToasts queues, auto-dismisses and clears timers on unmount", () => {
    vi.useFakeTimers();
    const { result, unmount } = renderHook(() => useToasts());
    let id = "";
    act(() => {
      id = result.current.push({ message: "已复制到剪贴板 · 42 字", duration: 2000 });
      result.current.push({ message: "默认时长" });
    });
    expect(result.current.toasts).toHaveLength(2);
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(result.current.toasts.map((t) => t.id)).not.toContain(id);
    act(() => {
      result.current.dismiss(result.current.toasts[0]?.id ?? "");
    });
    expect(result.current.toasts).toHaveLength(0);
    act(() => {
      result.current.push({ message: "pending" });
    });
    unmount();
    vi.useRealTimers();
  });
});
