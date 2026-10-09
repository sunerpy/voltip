import { useSyncExternalStore } from "react";

const listeners = new Set<() => void>();
let cached = Math.floor(Date.now() / 1000);
let timer: ReturnType<typeof setInterval> | undefined;

export const NOW_REFRESH_MS = 30_000;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (timer === undefined) {
    // The module-level value may be hours old by the time the first subscriber mounts.
    cached = Math.floor(Date.now() / 1000);
    timer = setInterval(() => {
      cached = Math.floor(Date.now() / 1000);
      for (const l of listeners) l();
    }, NOW_REFRESH_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

function snapshot(): number {
  return cached;
}

/** Unix seconds, refreshed every 30 s while any subscriber is mounted; pure during render. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
