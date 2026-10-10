import { MockBackend } from "@voltip/shared/mock";
import { renderHook } from "@testing-library/react";
import { useRelayRecheck } from "./useRelayRecheck";

describe("useRelayRecheck", () => {
  it("asks the relay link to check its socket when the network is back or the window is shown, and stops when unmounted", () => {
    const backend = new MockBackend();
    const { unmount } = renderHook(() => {
      useRelayRecheck(backend);
    });
    expect(backend.relayChecks).toBe(0);
    window.dispatchEvent(new Event("online"));
    expect(backend.relayChecks).toBe(1);
    const visibility = vi.spyOn(document, "visibilityState", "get");
    visibility.mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(backend.relayChecks).toBe(1);
    visibility.mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(backend.relayChecks).toBe(2);
    unmount();
    window.dispatchEvent(new Event("online"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(backend.relayChecks).toBe(2);
    visibility.mockRestore();
  });
});
