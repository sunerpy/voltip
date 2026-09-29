import { idleSnapshot } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, fireEvent, screen } from "@testing-library/react";
import { renderApp } from "../test/render";

const CODE = {
  words: ["alpha", "bravo", "charlie", "delta"],
  fingerprint: "A7:C4:19:8E · 3D:F2:61:09",
} as const;

async function verifyScreen(backend: MockBackend) {
  renderApp({ backend, initialScreen: "verify" });
  await screen.findByRole("heading", { level: 1 });
  // The frame registers its event listener in an effect: let it run first.
  await act(async () => {
    await Promise.resolve();
  });
}

describe("Verify screen (docs/pairing.md)", () => {
  it("every finished state says why and offers 重新配对, which resets and goes back to pairing", async () => {
    const backend = new MockBackend({ role: "phone" });
    const invoke = vi.spyOn(backend, "invoke");
    await verifyScreen(backend);
    expect(screen.getByText("没有进行中的配对。")).toBeInTheDocument();
    act(() => {
      backend.publish({ type: "pairing", ...idleSnapshot(), state: { state: "rejected" } });
    });
    expect(screen.getByText("配对已被拒绝，本次配对已取消。")).toBeInTheDocument();
    act(() => {
      backend.publish({ type: "pairing", ...idleSnapshot(), state: { state: "expired" } });
    });
    expect(screen.getByText("配对失败，本次配对已取消。")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重新配对" }));
    expect(invoke).toHaveBeenCalledWith("pairing_reset");
    expect(await screen.findByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
  });

  it("an anonymous peer shows as 电脑; the lamps follow who confirmed", async () => {
    const backend = new MockBackend({ role: "phone" });
    await verifyScreen(backend);
    act(() => {
      backend.publish({
        type: "pairing",
        ...idleSnapshot(),
        state: { state: "awaiting_verification" },
        safety_code: { ...CODE, words: [...CODE.words] },
      });
    });
    expect(screen.getByText("电脑")).toBeInTheDocument();
    expect(screen.getByText("— · 已建立加密连接")).toBeInTheDocument();
    act(() => {
      backend.publish({
        type: "pairing",
        ...idleSnapshot(),
        state: { state: "awaiting_verification" },
        safety_code: { ...CODE, words: [...CODE.words] },
        local_confirmed: true,
      });
    });
    expect(screen.getByText("已确认 · 等待电脑确认…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "等待电脑确认…" })).toBeDisabled();
    act(() => {
      backend.publish({
        type: "pairing",
        ...idleSnapshot(),
        state: { state: "awaiting_verification" },
        safety_code: { ...CODE, words: [...CODE.words] },
        peer_confirmed: true,
      });
    });
    expect(screen.getByText("对方已确认")).toBeInTheDocument();
    expect(screen.queryByText("已确认 · 等待电脑确认…")).toBeNull();
  });
});
