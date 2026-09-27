import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

/** Event the desktop shell emits to the pill window (`voltip_desktop_lib::overlay::OVERLAY_EVENT`). */
export const OVERLAY_EVENT = "voltip://overlay";

export interface OverlayEventPayload {
  state: string;
}

function isPayload(value: unknown): value is OverlayEventPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { state?: unknown }).state === "string"
  );
}

export interface OverlayEventSource {
  available: () => boolean;
  listen: (handler: (payload: unknown) => void) => Promise<() => void>;
  /** The state the shell wants shown right now (`overlay_state`). A window that was prewarmed
   *  hidden may finish loading only after the first show, so the event that announced the state
   *  can predate this listener; pulling once after subscribing closes that gap. */
  current: () => Promise<unknown>;
}

const tauriSource: OverlayEventSource = {
  available: () => isTauri(),
  listen: (handler) => listen<unknown>(OVERLAY_EVENT, (e) => handler(e.payload)),
  current: () => invoke<string>("overlay_state"),
};

/** The pill window's current state: the route's initial value until the shell sends the next one.
 *  Outside Tauri (browser preview, tests) there is no event source and the route value stands. */
export function useOverlayWindowState(
  initial: string | undefined,
  source: OverlayEventSource = tauriSource,
): string | undefined {
  // Derived state: a new route value resets the live state during render (React's documented
  // pattern for adjusting state when a prop changes), without an extra effect round-trip.
  const [entry, setEntry] = useState({ initial, state: initial });
  if (entry.initial !== initial) setEntry({ initial, state: initial });
  const state = entry.initial === initial ? entry.state : initial;
  useEffect(() => {
    if (initial === undefined || !source.available()) return;
    let alive = true;
    let stop: (() => void) | undefined;
    source
      .listen((payload) => {
        if (alive && isPayload(payload)) setEntry({ initial, state: payload.state });
      })
      .then((unlisten) => {
        if (alive) stop = unlisten;
        else unlisten();
        return source.current();
      })
      .then((pulled) => {
        if (alive && typeof pulled === "string" && pulled.length > 0)
          setEntry({ initial, state: pulled });
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
  }, [initial, source]);
  return state;
}
