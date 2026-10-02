import { onBackButtonPress } from "@tauri-apps/api/app";
import type { PluginListener } from "@tauri-apps/api/core";

/** The phone's system back: Android's edge swipe or back button. */
export interface SystemBack {
  /** `handler` takes every back while the app handles them; the returned function stops that. */
  listen(handler: () => void): () => void;
  /** The backs within `ms` go to the system, which leaves the app: the second of two at 说话. */
  release(ms: number): void;
}

/** Tauri's back-button event (its app plugin): while a listener is registered, a back reaches it
 *  instead of closing the app, and without one the system does what it always does. */
export function tauriBack(): SystemBack {
  let handler: (() => void) | undefined;
  let listener: Promise<PluginListener> | undefined;
  let resume: ReturnType<typeof setTimeout> | undefined;
  const register = () => {
    listener ??= onBackButtonPress(() => handler?.());
  };
  const unregister = () => {
    const current = listener;
    listener = undefined;
    void current?.then((l) => l.unregister());
  };
  return {
    listen(next) {
      handler = next;
      register();
      return () => {
        handler = undefined;
        clearTimeout(resume);
        unregister();
      };
    },
    release(ms) {
      clearTimeout(resume);
      unregister();
      // `listen`'s stop clears this, so it only fires while someone listens.
      resume = setTimeout(register, ms);
    },
  };
}
