import { zhT } from "@voltip/shared";
import { render, renderHook } from "@testing-library/react";
import {
  CLIPBOARD_UNAVAILABLE_MESSAGE,
  ShellProvider,
  copyText,
  copyWithToast,
  useShell,
} from "./shell-context";

function Bad() {
  useShell();
  return null;
}

describe("shell-context", () => {
  it("copyText reports success/failure and handles a missing clipboard", async () => {
    const original = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    expect(await copyText("x")).toBe(false);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: () => Promise.resolve() },
      configurable: true,
    });
    expect(await copyText("x")).toBe(true);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: () => Promise.reject(new Error("denied")) },
      configurable: true,
    });
    expect(await copyText("x")).toBe(false);
    Object.defineProperty(navigator, "clipboard", { value: original, configurable: true });
  });

  it("regression: copyWithToast only claims success after the clipboard accepted the text", async () => {
    const original = navigator.clipboard;
    const toast = vi.fn(() => "id");
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    expect(await copyWithToast({ toast, t: zhT.t }, "x", "已复制")).toBe(false);
    expect(toast).toHaveBeenLastCalledWith({
      message: CLIPBOARD_UNAVAILABLE_MESSAGE,
      duration: 3000,
      tone: "danger",
    });
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: () => Promise.resolve() },
      configurable: true,
    });
    expect(await copyWithToast({ toast, t: zhT.t }, "x", "已复制")).toBe(true);
    expect(toast).toHaveBeenLastCalledWith({ message: "已复制", duration: 2000 });
    expect(toast).toHaveBeenCalledTimes(2);
    Object.defineProperty(navigator, "clipboard", { value: original, configurable: true });
  });

  it("provides toasts, confirm and palette state; guards usage", () => {
    const { result } = renderHook(() => useShell(), { wrapper: ShellProvider });
    expect(result.current.paletteOpen).toBe(false);
    expect(result.current.pending).toBeUndefined();
    expect(() => render(<Bad />)).toThrow("useShell must be used inside <ShellProvider>");
  });
});
