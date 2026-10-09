import { act, render, screen } from "@testing-library/react";
import { type OverlayEventSource, useOverlayWindowState } from "./useOverlayWindowState";

function Probe({ initial, source }: { initial?: string; source: OverlayEventSource }) {
  const state = useOverlayWindowState(initial, source);
  return <div data-testid="state">{state ?? "none"}</div>;
}

describe("useOverlayWindowState", () => {
  it("regression: the pill window follows the shell's voltip://overlay events and ignores malformed payloads", async () => {
    let handler: ((payload: unknown) => void) | undefined;
    const unlisten = vi.fn();
    const source: OverlayEventSource = {
      available: () => true,
      listen: (h) => {
        handler = h;
        return Promise.resolve(unlisten);
      },
      // regression: the shell already switched to `listening` before this page finished loading
      // (prewarmed hidden window); the pull after subscribing must pick that up.
      current: () => Promise.resolve("listening"),
    };
    const view = render(<Probe initial="blank" source={source} />);
    expect(screen.getByTestId("state")).toHaveTextContent("blank");
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByTestId("state")).toHaveTextContent("listening");
    act(() => {
      handler?.({ state: "blank" });
    });
    expect(screen.getByTestId("state")).toHaveTextContent("blank");
    act(() => {
      handler?.({ state: "listening" });
    });
    expect(screen.getByTestId("state")).toHaveTextContent("listening");
    act(() => {
      handler?.({ nope: 1 });
      handler?.("listening");
    });
    expect(screen.getByTestId("state")).toHaveTextContent("listening");
    act(() => {
      handler?.({ state: "blank" });
    });
    expect(screen.getByTestId("state")).toHaveTextContent("blank");
    view.unmount();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("keeps the route value outside Tauri and tolerates a failing or late unlisten", async () => {
    const browser: OverlayEventSource = {
      available: () => false,
      listen: () => Promise.reject(new Error("never")),
      current: () => Promise.reject(new Error("never")),
    };
    const view = render(<Probe initial="listening" source={browser} />);
    expect(screen.getByTestId("state")).toHaveTextContent("listening");
    view.rerender(<Probe initial="processing" source={browser} />);
    expect(screen.getByTestId("state")).toHaveTextContent("processing");
    view.rerender(<Probe source={browser} />);
    expect(screen.getByTestId("state")).toHaveTextContent("none");
    view.unmount();

    // Late subscription: the unlisten resolved after unmount is still called.
    let resolveListen: ((u: () => void) => void) | undefined;
    const late = vi.fn();
    const slow: OverlayEventSource = {
      available: () => true,
      listen: () =>
        new Promise((r) => {
          resolveListen = r;
        }),
      current: () => Promise.resolve("blank"),
    };
    const second = render(<Probe initial="blank" source={slow} />);
    second.unmount();
    resolveListen?.(late);
    await act(async () => {
      await Promise.resolve();
    });
    expect(late).toHaveBeenCalledTimes(1);

    // A throwing unlisten never breaks unmount.
    const throwing: OverlayEventSource = {
      available: () => true,
      listen: () =>
        Promise.resolve(() => {
          throw new Error("no __TAURI_EVENT_PLUGIN_INTERNALS__");
        }),
      // A malformed pull result is ignored, never rendered.
      current: () => Promise.resolve({ nope: 1 }),
    };
    const third = render(<Probe initial="blank" source={throwing} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(() => {
      third.unmount();
    }).not.toThrow();
    const rejecting: OverlayEventSource = {
      available: () => true,
      listen: () => Promise.reject(new Error("gone")),
      current: () => Promise.reject(new Error("gone")),
    };
    const fourth = render(<Probe initial="blank" source={rejecting} />);
    await act(async () => {
      await Promise.resolve();
    });
    fourth.unmount();
  });
});
