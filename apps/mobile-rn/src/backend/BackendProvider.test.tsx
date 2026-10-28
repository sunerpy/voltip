// The provider every screen reads the core through: `core_state` first, each event folded in after,
// and a state event that overtakes the first answer.
import { act, render, screen } from "@testing-library/react-native";
import { type Backend, type UiEvent, type UiState } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { Component, type ReactNode } from "react";
import { Text } from "react-native";

import { BackendProvider, EMPTY, useBackend, useUiState } from "./BackendProvider";

/** A core whose `getState` answers when told to, and whose events the test sends. */
function fake() {
  let listener: ((event: UiEvent) => void) | undefined;
  let settle: { resolve: (s: UiState) => void; reject: (e: unknown) => void } | undefined;
  const backend = {
    on: (l: (event: UiEvent) => void) => {
      listener = l;
      return () => {
        listener = undefined;
      };
    },
    getState: () =>
      new Promise<UiState>((resolve, reject) => {
        settle = { resolve, reject };
      }),
  } as unknown as Backend;
  return {
    backend,
    send: (event: UiEvent) => listener?.(event),
    answer: (s: UiState) => settle?.resolve(s),
    fail: (e: unknown) => settle?.reject(e),
    listening: () => listener !== undefined,
  };
}

function Probe() {
  const { state, error, lastEvent } = useBackend();
  return (
    <>
      <Text testID="loaded">{state === undefined ? "no" : "yes"}</Text>
      <Text testID="version">{useUiState().app_version}</Text>
      <Text testID="error">{error ?? ""}</Text>
      <Text testID="last">{lastEvent?.type ?? ""}</Text>
    </>
  );
}

const wrap = (backend: Backend, onEvent?: (e: UiEvent) => void) =>
  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <BackendProvider backend={backend} {...(onEvent === undefined ? {} : { onEvent })}>
        {children}
      </BackendProvider>
    );
  };

describe("BackendProvider", () => {
  it("shows the empty state until core_state answers, then folds each event in", async () => {
    const core = fake();
    const seen: string[] = [];
    await render(<Probe />, { wrapper: wrap(core.backend, (e) => seen.push(e.type)) });
    expect(screen.getByTestId("loaded")).toHaveTextContent("no");
    // An event before the first state is passed on but folds into nothing.
    await act(async () => {
      core.send({ type: "error", message: "早到" });
      await Promise.resolve();
    });
    expect(screen.getByTestId("loaded")).toHaveTextContent("no");
    expect(screen.getByTestId("last")).toHaveTextContent("error");
    await act(async () => {
      core.answer({ ...EMPTY, app_version: "0.1.10" });
      await Promise.resolve();
    });
    expect(screen.getByTestId("version")).toHaveTextContent("0.1.10");
    await act(async () => {
      core.send({ type: "sent_texts", texts: [] });
      await Promise.resolve();
    });
    expect(seen).toEqual(["error", "sent_texts"]);
  });

  it("takes a state event that arrives before core_state answers, and keeps it", async () => {
    const core = fake();
    await render(<Probe />, { wrapper: wrap(core.backend) });
    await act(async () => {
      core.send({ ...EMPTY, app_version: "从事件", type: "state" });
      await Promise.resolve();
    });
    expect(screen.getByTestId("version")).toHaveTextContent("从事件");
    // The late answer does not replace what the event brought.
    await act(async () => {
      core.answer({ ...EMPTY, app_version: "迟到的" });
      await Promise.resolve();
    });
    expect(screen.getByTestId("version")).toHaveTextContent("从事件");
  });

  it("says why core_state failed, an Error or anything else", async () => {
    const first = fake();
    const view = await render(<Probe />, { wrapper: wrap(first.backend) });
    await act(async () => {
      first.fail(new Error("核心没有启动"));
      await Promise.resolve();
    });
    expect(screen.getByTestId("error")).toHaveTextContent("核心没有启动");
    await view.unmount();
    const second = fake();
    await render(<Probe />, { wrapper: wrap(second.backend) });
    await act(async () => {
      second.fail("断开了");
      await Promise.resolve();
    });
    expect(screen.getByTestId("error")).toHaveTextContent("断开了");
  });

  it("stops listening when it unmounts, and ignores an answer that comes after", async () => {
    const core = fake();
    const view = await render(<Probe />, { wrapper: wrap(core.backend) });
    expect(core.listening()).toBe(true);
    await view.unmount();
    expect(core.listening()).toBe(false);
    await act(async () => {
      core.answer(EMPTY);
      core.fail(new Error("too late"));
      await Promise.resolve();
    });
  });

  it("ignores an event delivered after it stopped", async () => {
    // A core that keeps calling the listener it was given after `off`.
    const backend = new MockBackend({ role: "phone" });
    let held: ((event: UiEvent) => void) | undefined;
    const on = backend.on.bind(backend);
    jest.spyOn(backend, "on").mockImplementation((l) => {
      held = l;
      return on(l);
    });
    const seen = jest.fn();
    const view = await render(<Probe />, { wrapper: wrap(backend, seen) });
    await view.unmount();
    expect(held).toBeDefined();
    seen.mockClear();
    await act(async () => {
      held?.({ type: "error", message: "after" });
      await Promise.resolve();
    });
    expect(seen).not.toHaveBeenCalled();
    backend.destroy();
  });

  it("needs a provider", async () => {
    // React reports the error it caught; the boundary shows what was thrown.
    jest.spyOn(console, "error").mockImplementation(() => undefined);
    class Boundary extends Component<{ children: ReactNode }, { message?: string }> {
      override state: { message?: string } = {};
      static getDerivedStateFromError(e: unknown) {
        return { message: e instanceof Error ? e.message : String(e) };
      }
      override render() {
        return this.state.message === undefined ? (
          this.props.children
        ) : (
          <Text testID="caught">{this.state.message}</Text>
        );
      }
    }
    function Bare() {
      useBackend();
      return null;
    }
    await render(
      <Boundary>
        <Bare />
      </Boundary>,
    );
    expect(screen.getByTestId("caught")).toHaveTextContent(
      "useBackend must be used inside <BackendProvider>",
    );
  });
});
