import type { UnlistenFn } from "@tauri-apps/api/event";
import { act, renderHook, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { type WindowControls, createWindowControls, useWindowChrome } from "./window";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: vi.fn(() => false) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }));

const core = await import("@tauri-apps/api/core");
const tauriWindow = await import("@tauri-apps/api/window");

function fakeAppWindow() {
  return {
    minimize: vi.fn(() => Promise.resolve()),
    toggleMaximize: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    isMaximized: vi.fn(() => Promise.resolve(true)),
    startDragging: vi.fn(() => Promise.resolve()),
    onResized: vi.fn((_handler: (payload: unknown) => void) => Promise.resolve(vi.fn())),
  };
}

/** Each case owns its state: no shared fixtures, or the maximize cases leak into each other. */
function harness(overrides: Partial<WindowControls> = {}) {
  let maximized = false;
  let resizeHandler: (() => void) | undefined;
  const unlisten = vi.fn();
  const controls: WindowControls = {
    minimize: vi.fn(() => Promise.resolve()),
    toggleMaximize: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    startDragging: vi.fn(() => Promise.resolve()),
    isMaximized: vi.fn(() => Promise.resolve(maximized)),
    onResized: vi.fn((handler: () => void) => {
      resizeHandler = handler;
      return Promise.resolve(unlisten);
    }),
    ...overrides,
  };
  return {
    controls,
    factory: () => controls,
    unlisten,
    emitResize: () => resizeHandler?.(),
    setMaximized: (value: boolean) => {
      maximized = value;
    },
  };
}

/** An observable thenable: distinguishes "the caller owns the rejection" from "dropped promise". */
function rejectingUnlisten() {
  const state = { called: 0, rejectionHandled: false };
  const unlisten: UnlistenFn = () => {
    state.called += 1;
    // A thenable on purpose: it observes whether the caller attached a rejection handler.
    const thenable = {
      // oxlint-disable-next-line unicorn/no-thenable
      then: (_onFulfilled?: unknown, onRejected?: unknown) => {
        if (typeof onRejected === "function") {
          state.rejectionHandled = true;
          onRejected(
            new TypeError("Cannot read properties of undefined (reading 'unregisterListener')"),
          );
        }
        return Promise.resolve();
      },
    };
    // Typed `void`, implemented async: exactly what @tauri-apps/api does.
    return thenable as unknown as void;
  };
  return { unlisten, state };
}

describe("createWindowControls", () => {
  it("is null in a plain browser (no Tauri) and never touches the window handle", () => {
    vi.mocked(core.isTauri).mockReturnValue(false);
    expect(createWindowControls()).toBeNull();
    expect(tauriWindow.getCurrentWindow).not.toHaveBeenCalled();
  });

  it("is null when getCurrentWindow throws (partial host without __TAURI_INTERNALS__)", () => {
    vi.mocked(core.isTauri).mockReturnValue(true);
    vi.mocked(tauriWindow.getCurrentWindow).mockImplementation(() => {
      throw new TypeError("Cannot read properties of undefined (reading 'metadata')");
    });
    expect(createWindowControls()).toBeNull();
  });

  it("forwards every command to getCurrentWindow() and drops the resize payload", async () => {
    vi.mocked(core.isTauri).mockReturnValue(true);
    const appWindow = fakeAppWindow();
    vi.mocked(tauriWindow.getCurrentWindow).mockReturnValue(
      appWindow as unknown as ReturnType<typeof tauriWindow.getCurrentWindow>,
    );
    const controls = createWindowControls();
    if (controls === null) throw new Error("expected controls under Tauri");
    await controls.minimize();
    await controls.toggleMaximize();
    await controls.close();
    await controls.startDragging();
    await expect(controls.isMaximized()).resolves.toBe(true);
    expect(appWindow.minimize).toHaveBeenCalledOnce();
    expect(appWindow.toggleMaximize).toHaveBeenCalledOnce();
    expect(appWindow.close).toHaveBeenCalledOnce();
    expect(appWindow.startDragging).toHaveBeenCalledOnce();
    const handler = vi.fn();
    await controls.onResized(handler);
    const forwarded = appWindow.onResized.mock.calls[0]?.[0];
    if (!forwarded) throw new Error("onResized not forwarded");
    forwarded({ width: 1, height: 2 });
    // No arguments: callers re-query isMaximized() instead of guessing from dimensions.
    expect(handler).toHaveBeenCalledWith();
  });
});

describe("useWindowChrome", () => {
  const linux = () => "linux" as const;

  it("browser: no controls, not maximized, platform from the resolver with the identity hint", () => {
    const platformOf = vi.fn(() => "windows" as const);
    const { result } = renderHook(() => useWindowChrome("windows", () => null, platformOf));
    expect(result.current.controls).toBeNull();
    expect(result.current.maximized).toBe(false);
    expect(result.current.platform).toBe("windows");
    expect(platformOf).toHaveBeenCalledWith("windows");
  });

  it("uses the real factory and resolver by default (jsdom: no Tauri, linux)", () => {
    vi.mocked(core.isTauri).mockReturnValue(false);
    const { result } = renderHook(() => useWindowChrome(undefined));
    expect(result.current).toEqual({ platform: "linux", controls: null, maximized: false });
  });

  it("queries the initial maximized state so a window restored maximized paints 还原 at once", async () => {
    const bar = harness();
    bar.setMaximized(true);
    const { result } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    await waitFor(() => {
      expect(result.current.maximized).toBe(true);
    });
    expect(bar.controls.isMaximized).toHaveBeenCalledOnce();
  });

  it("re-syncs on resize events the user never clicked for, in both directions, and unsubscribes on unmount", async () => {
    const bar = harness();
    const { result, unmount } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    await waitFor(() => {
      expect(bar.controls.onResized).toHaveBeenCalledOnce();
    });
    expect(result.current.maximized).toBe(false);
    bar.setMaximized(true);
    act(() => {
      bar.emitResize();
    });
    await waitFor(() => {
      expect(result.current.maximized).toBe(true);
    });
    bar.setMaximized(false);
    act(() => {
      bar.emitResize();
    });
    await waitFor(() => {
      expect(result.current.maximized).toBe(false);
    });
    expect(bar.unlisten).not.toHaveBeenCalled();
    unmount();
    expect(bar.unlisten).toHaveBeenCalledOnce();
  });

  it("wraps the commands so a click never leaks a rejection", async () => {
    const bar = harness({
      minimize: vi.fn(() => Promise.reject(new Error("window.minimize not allowed"))),
      toggleMaximize: vi.fn(() => Promise.reject(new Error("denied"))),
      close: vi.fn(() => Promise.reject(new Error("denied"))),
      startDragging: vi.fn(() => Promise.reject(new Error("denied"))),
    });
    const { result } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    const controls = result.current.controls;
    if (controls === null) throw new Error("expected controls");
    expect(() => {
      controls.minimize();
      controls.toggleMaximize();
      controls.close();
      controls.startDragging?.();
    }).not.toThrow();
    await Promise.resolve();
    expect(bar.controls.minimize).toHaveBeenCalledOnce();
    expect(bar.controls.toggleMaximize).toHaveBeenCalledOnce();
    expect(bar.controls.close).toHaveBeenCalledOnce();
    expect(bar.controls.startDragging).toHaveBeenCalledOnce();
  });

  it("keeps the last known state when isMaximized() or onResized() reject", async () => {
    const bar = harness({
      isMaximized: vi.fn(() => Promise.reject(new Error("no permission"))),
      onResized: vi.fn(() => Promise.reject(new Error("event plugin missing"))),
    });
    const { result, unmount } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.maximized).toBe(false);
    expect(result.current.controls).not.toBeNull();
    expect(() => {
      unmount();
    }).not.toThrow();
  });

  it("regression: a rejected unlisten promise is owned on unmount instead of becoming unhandled", async () => {
    const { unlisten, state } = rejectingUnlisten();
    const bar = harness({ onResized: vi.fn(() => Promise.resolve(unlisten)) });
    const { unmount } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    await waitFor(() => {
      expect(bar.controls.onResized).toHaveBeenCalledOnce();
    });
    await act(async () => {
      await Promise.resolve();
    });
    unmount();
    expect(state.called).toBe(1);
    // `Promise.resolve(thenable)` adopts the thenable on a microtask, so give it one.
    await new Promise<void>((r) => {
      setTimeout(r, 0);
    });
    expect(state.rejectionHandled).toBe(true);
  });

  it("regression: a synchronously throwing unlisten lets unmount finish", async () => {
    const unlisten = vi.fn(() => {
      throw new TypeError("Cannot read properties of undefined (reading 'unregisterListener')");
    });
    const bar = harness({ onResized: vi.fn(() => Promise.resolve(unlisten)) });
    const { unmount } = renderHook(() => useWindowChrome(undefined, bar.factory, linux));
    await act(async () => {
      await Promise.resolve();
    });
    expect(() => {
      unmount();
    }).not.toThrow();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("regression: a subscription that settles after teardown is unsubscribed at once (StrictMode)", async () => {
    let settle: ((fn: UnlistenFn) => void) | undefined;
    const { unlisten, state } = rejectingUnlisten();
    const bar = harness({
      onResized: vi.fn(
        () =>
          new Promise<UnlistenFn>((resolve) => {
            settle = resolve;
          }),
      ),
    });
    const { unmount } = renderHook(() => useWindowChrome(undefined, bar.factory, linux), {
      wrapper: StrictMode,
    });
    unmount();
    expect(state.called).toBe(0);
    await act(async () => {
      settle?.(unlisten);
      await Promise.resolve();
    });
    expect(state.called).toBeGreaterThanOrEqual(1);
    expect(state.rejectionHandled).toBe(true);
  });
});
