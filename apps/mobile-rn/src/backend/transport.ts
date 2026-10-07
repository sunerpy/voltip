// The React Native transport of `@voltip/shared`'s TauriBackend (docs/mobile-rn.md §2): commands go
// to the Rust shell through the native module, events come back as `onEvent`, and the level meter's
// stream as `onChannel` frames addressed by id. Errors arrive as the shell's text, the way Tauri's
// `invoke` rejects, so the shared label helpers read them unchanged.
import { type ChannelLike, type TauriTransport, UI_EVENT_NAME } from "@voltip/shared";

/** What the transport needs from the native module (`modules/voltip-native`). */
export interface NativeBridge {
  invoke(command: string, args: string): Promise<string>;
  addListener(event: "onEvent", listener: (event: { json: string }) => void): { remove(): void };
  addListener(
    event: "onChannel",
    listener: (event: { id: number; json: string }) => void,
  ): { remove(): void };
}

/** The Rust shell's text for a refused microphone (`microphone::MICROPHONE_DENIED`). */
export const MICROPHONE_DENIED =
  "microphone: 麦克风权限被拒绝，请在系统设置中允许 Voltip 使用麦克风";

/** The commands that open the microphone: the app asks for `RECORD_AUDIO` before them. */
const NEEDS_MICROPHONE: ReadonlySet<string> = new Set(["phone_take_start", "dictation_start"]);

/** How `audio_meter_start`'s `onFrame` is sent: the stream's id (`commands::CHANNEL_KEY`). */
export const CHANNEL_KEY = "__voltipChannel";

export interface TransportOptions {
  /** Ask Android for the microphone; `true` once it is granted. */
  askMicrophone: () => Promise<boolean>;
  warn?: (message: string, detail: unknown) => void;
}

interface Stream extends ChannelLike {
  toJSON(): Record<string, number>;
}

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Standard base64 of `bytes` (the feedback attachment's body crosses the bridge as text). */
export function toBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i] ?? 0;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    const n = (a << 16) | ((b ?? 0) << 8) | (c ?? 0);
    out += BASE64[(n >> 18) & 63];
    out += BASE64[(n >> 12) & 63];
    out += b === undefined ? "=" : BASE64[(n >> 6) & 63];
    out += c === undefined ? "=" : BASE64[n & 63];
  }
  return out;
}

export function createTransport(native: NativeBridge, options: TransportOptions): TauriTransport {
  const streams = new Map<number, Stream>();
  // `audio_meter_stop` names the shell's subscription: which stream it fed.
  const meters = new Map<number, number>();
  let nextStream = 0;
  native.addListener("onChannel", ({ id, json }) => {
    const stream = streams.get(id);
    if (stream === undefined) return;
    try {
      stream.onmessage(JSON.parse(json));
    } catch (e) {
      options.warn?.("a stream frame did not parse", e);
    }
  });

  const call = async (command: string, args: unknown): Promise<unknown> => {
    let answer: string;
    try {
      answer = await native.invoke(command, JSON.stringify(args ?? null));
    } catch (e) {
      // Tauri rejects with the command's error text; so does this transport.
      // oxlint-disable-next-line no-throw-literal
      throw e instanceof Error ? e.message : String(e);
    }
    return answer === "" ? null : (JSON.parse(answer) as unknown);
  };

  // The streams this transport made, by the object `TauriBackend` hands back in `onFrame`.
  const ids = new WeakMap<object, number>();
  const streamOf = (args: Record<string, unknown> | undefined): number | undefined => {
    const onFrame: unknown = args?.["onFrame"];
    return typeof onFrame === "object" && onFrame !== null ? ids.get(onFrame) : undefined;
  };

  return {
    invoke: async (command, args) => {
      if (NEEDS_MICROPHONE.has(command) && !(await options.askMicrophone())) {
        // oxlint-disable-next-line no-throw-literal
        throw MICROPHONE_DENIED;
      }
      if (command === "audio_meter_stop") {
        const id = args?.["id"];
        if (typeof id === "number") {
          const stream = meters.get(id);
          meters.delete(id);
          if (stream !== undefined) streams.delete(stream);
        }
      }
      const answer = await call(command, args);
      if (command === "audio_meter_start" && typeof answer === "number") {
        const stream = streamOf(args);
        if (stream !== undefined) meters.set(answer, stream);
      }
      return answer;
    },
    invokeRaw: (command, bytes, headers) =>
      call(command, {
        data: toBase64(bytes),
        name: decodeURIComponent(headers["x-voltip-name"] ?? ""),
        type: headers["x-voltip-type"] ?? "",
      }),
    listen: (event, handler) => {
      if (event !== UI_EVENT_NAME) return Promise.resolve(() => undefined);
      const subscription = native.addListener("onEvent", ({ json }) => {
        let payload: unknown;
        try {
          payload = JSON.parse(json);
        } catch (e) {
          options.warn?.("an event did not parse", e);
          return;
        }
        handler({ payload });
      });
      return Promise.resolve(() => {
        subscription.remove();
      });
    },
    channel: () => {
      nextStream += 1;
      const id = nextStream;
      const stream: Stream = {
        onmessage: () => undefined,
        toJSON: () => ({ [CHANNEL_KEY]: id }),
      };
      streams.set(id, stream);
      ids.set(stream, id);
      return stream;
    },
    ...(options.warn === undefined ? {} : { warn: options.warn }),
  };
}
