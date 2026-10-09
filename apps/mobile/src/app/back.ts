import { addPluginListener, invoke, type PluginListener } from "@tauri-apps/api/core";

/** The phone's system back: Android's edge swipe or back button. */
export interface SystemBack {
  /** `handler` takes every back while the app handles them; the returned function stops that. */
  listen(handler: () => void): () => void;
  /** The backs within `ms` go to the system, which leaves the app: the second of two at 说话. */
  release(ms: number): void;
}

/** The app's own back plugin (`apps/mobile/src-tauri/src/back.rs`, `BackPlugin.kt`). */
export const BACK_PLUGIN = "voltip-back";

/** Android's back through the app's own plugin: while a listener is registered, a back reaches
 *  it instead of the system, and `release` lets the backs of a short window through. Tauri's own
 *  back event (`onBackButtonPress`) is added to the first activity alone, and Android recreates
 *  the activity when an overlay or the font size changes: after that every back left the app
 *  (the android-device CI job, 2026-10-03). */
export function tauriBack(): SystemBack {
  let handler: (() => void) | undefined;
  let listener: Promise<PluginListener> | undefined;
  return {
    listen(next) {
      handler = next;
      listener ??= addPluginListener(BACK_PLUGIN, "back", () => handler?.());
      return () => {
        handler = undefined;
        const current = listener;
        listener = undefined;
        void current?.then((l) => l.unregister());
      };
    },
    release(ms) {
      void invoke(`plugin:${BACK_PLUGIN}|release`, { ms });
    },
  };
}
