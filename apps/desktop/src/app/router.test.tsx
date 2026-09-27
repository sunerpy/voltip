import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  AI_ROUTE,
  SPEECH_ROUTE,
  HOME_ROUTE,
  RouterProvider,
  currentLocationPath,
  isBackgroundRoute,
  isSettingsSection,
  parseRoute,
  routePath,
  useRouter,
} from "./router";

describe("parseRoute / routePath", () => {
  it("maps every path and round-trips", () => {
    expect(parseRoute("/")).toEqual({ name: "home" });
    expect(parseRoute("/history")).toEqual({ name: "history" });
    expect(parseRoute("/history?filter=today")).toEqual({ name: "history", filter: "today" });
    expect(parseRoute("/dictionary")).toEqual({ name: "dictionary" });
    expect(parseRoute("/rules")).toEqual({ name: "rules" });
    // regression (2026-09-27): 引擎 became two settings groups, 语音模型 and AI 模型; the old
    // `/engines` page and `/settings/engine` group land on 语音模型, the old 润色 group on AI 模型.
    expect(parseRoute("/engines")).toEqual({ name: "settings", section: "speech" });
    expect(parseRoute("/engines")).toEqual(SPEECH_ROUTE);
    expect(parseRoute("/settings/engine")).toEqual(SPEECH_ROUTE);
    expect(parseRoute("/settings/refine")).toEqual(AI_ROUTE);
    expect(parseRoute("/settings/ai")).toEqual({ name: "settings", section: "ai" });
    expect(routePath(SPEECH_ROUTE)).toBe("/settings/speech");
    expect(routePath(AI_ROUTE)).toBe("/settings/ai");
    expect(isSettingsSection("refine")).toBe(false);
    expect(isSettingsSection("engine")).toBe(false);
    // regression (2026-09-25): the Bridge & MCP page was removed; its old URL is a 404, not a page.
    expect(parseRoute("/bridge")).toEqual({ name: "notfound", path: "/bridge" });
    expect(parseRoute("/devices")).toEqual({ name: "devices" });
    expect(parseRoute("/settings")).toEqual({ name: "settings", section: "appearance" });
    expect(parseRoute("/settings/hotkey")).toEqual({ name: "settings", section: "hotkey" });
    expect(parseRoute("/settings/bogus")).toEqual({ name: "settings", section: "appearance" });
    expect(parseRoute("/onboarding")).toEqual({ name: "onboarding", step: 1 });
    expect(parseRoute("/onboarding?step=3")).toEqual({ name: "onboarding", step: 3 });
    expect(parseRoute("/onboarding?step=9")).toEqual({ name: "onboarding", step: 1 });
    expect(parseRoute("#/overlay?state=listening")).toEqual({
      name: "overlay",
      state: "listening",
    });
    expect(parseRoute("/overlay")).toEqual({ name: "overlay" });
    expect(parseRoute("/rules?new=1")).toEqual({ name: "rules", compose: true });
    expect(parseRoute("/rules?new=0")).toEqual({ name: "rules" });
    expect(routePath({ name: "rules", compose: true })).toBe("/rules?new=1");
    expect(routePath({ name: "rules" })).toBe("/rules");
    expect(parseRoute("/nope")).toEqual({ name: "notfound", path: "/nope" });
    for (const path of [
      "/",
      "/history",
      "/history?filter=week",
      "/dictionary",
      "/rules",
      "/devices",
      "/settings/hotkey",
      "/settings/speech",
      "/settings/ai",
      "/onboarding",
      "/onboarding?step=2",
      "/overlay",
      "/overlay?state=armed",
      "/nope",
    ]) {
      expect(routePath(parseRoute(path))).toBe(path);
    }
    expect(isSettingsSection("about")).toBe(true);
    expect(isSettingsSection("x")).toBe(false);
  });

  it("prefers the hash route for the overlay window", () => {
    expect(currentLocationPath({ hash: "#/overlay?state=armed", pathname: "/", search: "" })).toBe(
      "/overlay?state=armed",
    );
    expect(currentLocationPath({ hash: "", pathname: "/history", search: "?filter=today" })).toBe(
      "/history?filter=today",
    );
  });
});

function Probe() {
  const { path, route, background, navigate } = useRouter();
  return (
    <div>
      <span data-testid="path">{path}</span>
      <span data-testid="route">{route.name}</span>
      <span data-testid="background">{routePath(background)}</span>
      <button
        onClick={() => {
          navigate("/history");
        }}>
        go
      </button>
      <button
        onClick={() => {
          navigate({ name: "settings", section: "hotkey" });
        }}>
        settings
      </button>
    </div>
  );
}

describe("RouterProvider", () => {
  it("navigates in memory mode without touching history", async () => {
    const user = userEvent.setup();
    const push = vi.spyOn(window.history, "pushState");
    render(
      <RouterProvider initialPath="/devices">
        <Probe />
      </RouterProvider>,
    );
    expect(screen.getByTestId("route")).toHaveTextContent("devices");
    await user.click(screen.getByText("go"));
    expect(screen.getByTestId("path")).toHaveTextContent("/history");
    await user.click(screen.getByRole("button", { name: "settings" }));
    expect(screen.getByTestId("path")).toHaveTextContent("/settings/hotkey");
    expect(push).not.toHaveBeenCalled();
  });

  it("uses window.location and history in browser mode, following popstate and hash routes", async () => {
    const user = userEvent.setup();
    window.history.replaceState(null, "", "/rules");
    const { unmount } = render(
      <RouterProvider>
        <Probe />
      </RouterProvider>,
    );
    expect(screen.getByTestId("route")).toHaveTextContent("rules");
    await user.click(screen.getByText("go"));
    expect(window.location.pathname).toBe("/history");
    act(() => {
      window.history.pushState(null, "", "/devices");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(screen.getByTestId("route")).toHaveTextContent("devices");
    unmount();
    window.location.hash = "#/overlay";
    render(
      <RouterProvider>
        <Probe />
      </RouterProvider>,
    );
    expect(screen.getByTestId("route")).toHaveTextContent("overlay");
    await user.click(screen.getByText("go"));
    expect(window.location.hash).toBe("#/history");
    window.location.hash = "";
    window.history.replaceState(null, "", "/");
  });

  it("regression: background tracks the last non-settings route and ignores onboarding / overlay / notfound", async () => {
    const user = userEvent.setup();
    function Walk() {
      const { navigate } = useRouter();
      return (
        <>
          {["/onboarding?step=2", "/overlay?state=armed", "/nope", "/history?filter=today"].map(
            (p) => (
              <button
                key={p}
                onClick={() => {
                  navigate(p);
                }}>
                {p}
              </button>
            ),
          )}
        </>
      );
    }
    // A deep link straight into settings sits over home.
    render(
      <RouterProvider initialPath="/settings/hotkey">
        <Probe />
        <Walk />
      </RouterProvider>,
    );
    expect(screen.getByTestId("route")).toHaveTextContent("settings");
    expect(screen.getByTestId("background")).toHaveTextContent("/");
    // Chrome-less routes never become the background.
    for (const p of ["/onboarding?step=2", "/overlay?state=armed", "/nope"]) {
      await user.click(screen.getByText(p));
      expect(screen.getByTestId("path")).toHaveTextContent(p);
      expect(screen.getByTestId("background")).toHaveTextContent(/^\/$/);
    }
    // A real page does, including its query, and stays while settings is open.
    await user.click(screen.getByText("/history?filter=today"));
    expect(screen.getByTestId("background")).toHaveTextContent("/history?filter=today");
    await user.click(screen.getByRole("button", { name: "settings" }));
    expect(screen.getByTestId("route")).toHaveTextContent("settings");
    expect(screen.getByTestId("background")).toHaveTextContent("/history?filter=today");
    await user.click(screen.getByRole("button", { name: "settings" }));
    expect(screen.getByTestId("background")).toHaveTextContent("/history?filter=today");
    await user.click(screen.getByText("go"));
    expect(screen.getByTestId("background")).toHaveTextContent(/^\/history$/);

    expect(HOME_ROUTE).toEqual({ name: "home" });
    expect(isBackgroundRoute({ name: "devices" })).toBe(true);
    expect(isBackgroundRoute({ name: "settings", section: "about" })).toBe(false);
    expect(isBackgroundRoute({ name: "onboarding", step: 1 })).toBe(false);
    expect(isBackgroundRoute({ name: "overlay" })).toBe(false);
    expect(isBackgroundRoute({ name: "notfound", path: "/x" })).toBe(false);
  });

  it("regression: /engines opens the settings dialog's engines group and keeps the page beneath as background", async () => {
    const user = userEvent.setup();
    function Engines() {
      const { navigate } = useRouter();
      return (
        <button
          onClick={() => {
            navigate("/engines");
          }}>
          engines
        </button>
      );
    }
    render(
      <RouterProvider initialPath="/engines">
        <Probe />
        <Engines />
      </RouterProvider>,
    );
    expect(screen.getByTestId("route")).toHaveTextContent("settings");
    expect(screen.getByTestId("background")).toHaveTextContent(/^\/$/);
    await user.click(screen.getByText("go"));
    expect(screen.getByTestId("route")).toHaveTextContent("history");
    await user.click(screen.getByRole("button", { name: "engines" }));
    expect(screen.getByTestId("route")).toHaveTextContent("settings");
    expect(screen.getByTestId("background")).toHaveTextContent(/^\/history$/);
  });

  it("guards hook usage", () => {
    expect(() => render(<Probe />)).toThrow("useRouter must be used inside <RouterProvider>");
  });
});
