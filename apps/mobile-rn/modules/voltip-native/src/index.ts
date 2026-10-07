// The Kotlin module `VoltipNative` (android/src/main/java/dev/voltip/rn/VoltipNativeModule.kt):
// the Rust shell's two calls and its two event streams, and the system's accent colour.
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
  addListener<E extends keyof VoltipNativeEvents>(
    event: E,
    listener: VoltipNativeEvents[E],
  ): { remove(): void };
}

export function loadVoltipNative(): VoltipNativeModule {
  return requireNativeModule<VoltipNativeModule>("VoltipNative");
}
