import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderHook } from "@testing-library/react";
import { renderApp } from "../test/render";
import {
  TRAY_EVENT,
  type TrayRequestSource,
  trayRequestOf,
  useTrayRequests,
} from "./tray-requests";

/** Stands in for the shell's `voltip://tray` event. */
function fakeTray() {
  let handler: ((payload: unknown) => void) | undefined;
  const source: TrayRequestSource = {
    available: () => true,
    listen: (h) => {
      handler = h;
      return Promise.resolve(() => {
        handler = undefined;
      });
    },
  };
  return {
    source,
    subscribed: () => handler !== undefined,
    send: async (payload: unknown) => {
      await act(async () => {
        handler?.(payload);
        await Promise.resolve();
      });
    },
  };
}

describe("tray requests", () => {
  it("reads only the three actions the webview finishes", () => {
    expect(TRAY_EVENT).toBe("voltip://tray");
    expect(trayRequestOf({ action: "settings" })).toBe("settings");
    expect(trayRequestOf({ action: "update" })).toBe("update");
    // docs/dictation.md §28: the result card's 打开历史.
    expect(trayRequestOf({ action: "history" })).toBe("history");
    for (const payload of [null, undefined, "settings", {}, { action: "quit" }, { action: 1 }])
      expect(trayRequestOf(payload)).toBeUndefined();
  });

  it("subscribes only inside Tauri and unsubscribes on unmount", async () => {
    const outside: TrayRequestSource = { available: () => false, listen: vi.fn() };
    renderHook(() => {
      useTrayRequests(() => undefined, outside);
    });
    expect(outside.listen).not.toHaveBeenCalled();

    const tray = fakeTray();
    const seen: string[] = [];
    const { unmount } = renderHook(() => {
      useTrayRequests((request) => seen.push(request), tray.source);
    });
    await waitFor(() => {
      expect(tray.subscribed()).toBe(true);
    });
    await tray.send({ action: "update" });
    await tray.send({ action: "nonsense" });
    expect(seen).toEqual(["update"]);
    unmount();
    expect(tray.subscribed()).toBe(false);
  });

  it("regression: the tray's 设置… opens the settings dialog, 检查更新… the update dialog with a fresh check", async () => {
    const tray = fakeTray();
    const { backend } = renderApp({ traySource: tray.source });
    const invoke = vi.spyOn(backend, "invoke");
    await waitFor(() => {
      expect(tray.subscribed()).toBe(true);
    });

    await tray.send({ action: "settings" });
    expect(await screen.findByRole("dialog", { name: "设置" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "通用" })).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "设置" })).not.toBeInTheDocument();
    });

    await tray.send({ action: "update" });
    expect(await screen.findByRole("dialog", { name: "软件更新" })).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("update_check");
  });

  it("docs/dictation.md section 28: the result card's 打开历史 brings the main window up on the history page", async () => {
    const tray = fakeTray();
    renderApp({ traySource: tray.source });
    await waitFor(() => {
      expect(tray.subscribed()).toBe(true);
    });
    await tray.send({ action: "history" });
    expect(await screen.findByTestId("page-history")).toBeInTheDocument();
  });
});
