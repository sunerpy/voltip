import { isTauri } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Platform } from "@voltip/shared";
import type { TitleBarControls, TitleBarPlatform } from "@voltip/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { resolvePlatform } from "./platform";

/** The window commands the self-drawn title bar needs, bound to the current Tauri window. */
export interface WindowControls {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  startDragging: () => Promise<void>;
  onResized: (handler: () => void) => Promise<UnlistenFn>;
}

/**
 * `null` outside a Tauri webview (Vite dev in a tab, vitest): `getCurrentWindow()` eagerly reads
 * `window.__TAURI_INTERNALS__.metadata.currentWindow.label` and throws a `TypeError` there
 * rather than returning a dead handle. The bar then renders without buttons instead of drawing
 * three that do nothing.
 *
 * `onResized` drops Tauri's size payload on purpose (`() => handler()`): callers must re-query
 * `isMaximized()` rather than guess the state from dimensions. `close()` rather than a custom
 * quit command keeps `tauri://close-requested` interception (tray / confirm dialog) working.
 */
export function createWindowControls(): WindowControls | null {
  if (!isTauri()) return null;
  try {
    const appWindow = getCurrentWindow();
    return {
      minimize: () => appWindow.minimize(),
      toggleMaximize: () => appWindow.toggleMaximize(),
      close: () => appWindow.close(),
      isMaximized: () => appWindow.isMaximized(),
      startDragging: () => appWindow.startDragging(),
      onResized: (handler) => appWindow.onResized(() => handler()),
    };
  } catch (_error) {
    return null;
  }
}

/**
 * `UnlistenFn` is typed `() => void` but implemented `async`, and it first touches
 * `window.__TAURI_EVENT_PLUGIN_INTERNALS__`, a second global a partial host (mock IPC harness,
 * preview build) may not install. So a failure arrives as a synchronous throw *or* a rejected
 * promise; both are contained here because this runs from an effect cleanup, and React turns an
 * exception out of cleanup into a render error that unmounts the whole tree.
 */
function unsubscribeQuietly(unlisten: UnlistenFn): void {
  try {
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch (_error) {
    // Losing the unsubscribe costs nothing: the listener dies with the webview.
  }
}

export interface WindowChrome {
  platform: TitleBarPlatform;
  /** Sync, rejection-swallowing controls for `<TitleBar>`; `null` without a Tauri window. */
  controls: TitleBarControls | null;
  maximized: boolean;
}

/**
 * Window chrome state for the title bar: platform, controls and the live maximized flag.
 *
 * Maximize state needs both the initial `isMaximized()` query (a window restored maximized by the
 * OS emits no resize event at startup) and the resize subscription (double-click on the bar,
 * `Win`+arrow and the OS window menu all bypass our button). StrictMode double-invokes effects,
 * so the async settle is guarded with `disposed`. Both dependencies are injectable for tests.
 */
export function useWindowChrome(
  hint: Platform | undefined,
  factory: () => WindowControls | null = createWindowControls,
  platformOf: (hint: Platform | undefined) => TitleBarPlatform = resolvePlatform,
): WindowChrome {
  const raw = useMemo(() => factory(), [factory]);
  const platform = useMemo(() => platformOf(hint), [platformOf, hint]);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (raw === null) return undefined;
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    const sync = async () => {
      try {
        const value = await raw.isMaximized();
        if (!disposed) setMaximized(value);
      } catch (_error) {
        // A rejected query (no permission, window already gone) keeps the last known state.
      }
    };
    void sync();

    raw
      .onResized(() => {
        void sync();
      })
      .then((fn) => {
        // The effect can be torn down before this settles; unsubscribe at once instead of leaking.
        if (disposed) unsubscribeQuietly(fn);
        else unlisten = fn;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      if (unlisten !== undefined) unsubscribeQuietly(unlisten);
    };
  }, [raw]);

  const run = useCallback((action: () => Promise<void>) => {
    void action().catch(() => undefined);
  }, []);

  const controls = useMemo<TitleBarControls | null>(
    () =>
      raw === null
        ? null
        : {
            minimize: () => {
              run(raw.minimize);
            },
            toggleMaximize: () => {
              run(raw.toggleMaximize);
            },
            close: () => {
              run(raw.close);
            },
            startDragging: () => {
              run(raw.startDragging);
            },
          },
    [raw, run],
  );

  return { platform, controls, maximized };
}
