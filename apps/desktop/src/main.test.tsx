import { act, screen } from "@testing-library/react";

describe("the entry point", () => {
  it("regression: a first launch on a fresh profile opens the home page, not the setup guide", async () => {
    // Every interval the app starts, to check below that none outlives it.
    const started = vi.spyOn(globalThis, "setInterval");
    const cleared = vi.spyOn(globalThis, "clearInterval");
    // What the window starts with: no stored answers, the home route, the element Vite mounts on.
    window.localStorage.clear();
    window.history.replaceState(null, "", "/");
    document.body.innerHTML = '<div id="root"></div>';
    const { root } = await import("./main");
    expect(await screen.findByTestId("page-home")).toBeInTheDocument();
    expect(window.location.pathname).toBe("/");
    expect(screen.queryByRole("list", { name: "步骤" })).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /设置向导/ })).not.toBeInTheDocument();
    // Regression (main CI, 2026-09-30): the app stayed mounted after the test, the home page's
    // level meter kept ticking, and a tick after the test environment was gone made React render
    // without a `window`. Unmounted here, the app leaves no interval running.
    act(() => root.unmount());
    const handles = started.mock.results.map((r) => r.value as unknown);
    const stopped = new Set(cleared.mock.calls.map(([handle]) => handle as unknown));
    expect(handles.length).toBeGreaterThan(0);
    expect(handles.filter((h) => !stopped.has(h))).toEqual([]);
    started.mockRestore();
    cleared.mockRestore();
  });
});
