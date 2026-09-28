import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

/** Event the desktop shell sends the main window for a tray menu entry the webview finishes
 *  (`voltip_desktop_lib::platform::tray::TRAY_EVENT`); the shell has already shown the window. */
export const TRAY_EVENT = "voltip://tray";

export type TrayRequest = "settings" | "update";

export function trayRequestOf(payload: unknown): TrayRequest | undefined {
  if (typeof payload !== "object" || payload === null) return undefined;
  const action = (payload as { action?: unknown }).action;
  return action === "settings" || action === "update" ? action : undefined;
}

export interface TrayRequestSource {
  available: () => boolean;
  listen: (handler: (payload: unknown) => void) => Promise<() => void>;
}

const tauriSource: TrayRequestSource = {
  available: () => isTauri(),
  listen: (handler) => listen<unknown>(TRAY_EVENT, (e) => handler(e.payload)),
};

/** Run `handle` for every tray request while mounted. Outside Tauri (browser preview, tests
 *  without a source) nothing is subscribed. The latest `handle` is used without resubscribing. */
export function useTrayRequests(
  handle: (request: TrayRequest) => void,
  source: TrayRequestSource = tauriSource,
): void {
  const latest = useRef(handle);
  useEffect(() => {
    latest.current = handle;
  }, [handle]);
  useEffect(() => {
    if (!source.available()) return undefined;
    let alive = true;
    let stop: (() => void) | undefined;
    source
      .listen((payload) => {
        const request = trayRequestOf(payload);
        if (alive && request !== undefined) latest.current(request);
      })
      .then((unlisten) => {
        if (alive) stop = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      try {
        stop?.();
      } catch {
        // A partial host may reject the unlisten; the listener dies with the webview anyway.
      }
    };
  }, [source]);
}
