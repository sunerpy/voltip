// The Kotlin module `VoltipNative` (android/src/main/java/dev/voltip/rn/VoltipNativeModule.kt):
// the Rust shell's two calls and its two event streams, the system's accent colour, and the
// floating bubble's state and options.
import { requireNativeModule } from "expo";

export type VoltipNativeEvents = {
  /** One `UiEvent` as JSON (`voltip://event`). */
  onEvent: (event: { json: string }) => void;
  /** One frame of a stream (the level meter's `onFrame`) as JSON. */
  onChannel: (event: { id: number; json: string }) => void;
};

export interface VoltipNativeModule {
  /** Start the shell once per process: `null`, or why it could not start. */
  start(): Promise<string | null>;
  /** Run `command` with `args` (a JSON object, or `null`): the answer as JSON text. */
  invoke(command: string, args: string): Promise<string>;
  /** The wallpaper's colour the system themes itself with (`#rrggbb`), `null` before Android 12. */
  systemAccent(): string | null;
  /** The floating bubble's state (`BubbleService`): `BubbleStatus` (src/backend/bubble.tsx) as JSON. */
  bubbleStatus(): Promise<string>;
  /** Save the bubble's options (`BubbleConfig` as JSON) and show or hide it: its state after, as
   *  `bubbleStatus` answers. */
  bubbleConfigure(config: string): Promise<string>;
  /** Open the system's 显示在其他应用上层 page for the app. */
  bubbleOpenOverlaySettings(): Promise<void>;
  /** Open the system's notification settings for the app (a permission refused for good, or
   *  notifications switched off before Android 13). */
  bubbleOpenNotificationSettings(): Promise<void>;
  addListener<E extends keyof VoltipNativeEvents>(
    event: E,
    listener: VoltipNativeEvents[E],
  ): { remove(): void };
}

export function loadVoltipNative(): VoltipNativeModule {
  return requireNativeModule<VoltipNativeModule>("VoltipNative");
}
