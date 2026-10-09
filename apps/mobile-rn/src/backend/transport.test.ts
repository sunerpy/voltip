import { TauriBackend, UI_EVENT_NAME, levelFrameSchema } from "@voltip/shared";

import {
  CHANNEL_KEY,
  MICROPHONE_DENIED,
  type NativeBridge,
  createTransport,
  toBase64,
} from "./transport";

type Listener = (event: { json: string } & { id?: number }) => void;

/** A native module that answers from `answers` and records every call. */
function fakeNative(answers: Record<string, (args: unknown) => unknown> = {}) {
  const calls: { command: string; args: unknown }[] = [];
  const listeners: Record<string, Listener[]> = { onEvent: [], onChannel: [] };
  const native: NativeBridge = {
    invoke: (command, args) => {
      const parsed: unknown = JSON.parse(args);
      calls.push({ command, args: parsed });
      const answer = answers[command];
      if (answer === undefined) return Promise.resolve("null");
      try {
        return Promise.resolve(JSON.stringify(answer(parsed) ?? null));
      } catch (e) {
        return Promise.reject(e instanceof Error ? e : new Error(String(e)));
      }
    },
    addListener: (event: "onEvent" | "onChannel", listener: Listener) => {
      listeners[event]?.push(listener);
      return {
        remove: () => {
          listeners[event] = (listeners[event] ?? []).filter((l) => l !== listener);
        },
      };
    },
  } as NativeBridge;
  const emit = (event: "onEvent" | "onChannel", payload: { json: string; id?: number }) => {
    for (const l of listeners[event] ?? []) l(payload);
  };
  return { native, calls, emit, listeners };
}

describe("the React Native transport", () => {
  it("sends the arguments as JSON and parses the answer", async () => {
    const { native, calls } = fakeNative({ history_entry: () => ({ ok: true }) });
    const transport = createTransport(native, { askMicrophone: () => Promise.resolve(true) });
    await expect(transport.invoke("history_entry", { id: "x" })).resolves.toEqual({ ok: true });
    await expect(transport.invoke("pairing_start")).resolves.toBeNull();
    expect(calls).toEqual([
      { command: "history_entry", args: { id: "x" } },
      { command: "pairing_start", args: null },
    ]);
  });

  it("rejects with the shell's error text, as Tauri's invoke does", async () => {
    const { native } = fakeNative({
      device_rename: () => {
        throw new Error("invalid args for device_rename: missing field `name`");
      },
    });
    const transport = createTransport(native, { askMicrophone: () => Promise.resolve(true) });
    await expect(transport.invoke("device_rename", {})).rejects.toBe(
      "invalid args for device_rename: missing field `name`",
    );
  });

  it("asks for the microphone before a take and refuses when it is denied", async () => {
    const { native, calls } = fakeNative();
    const ask = jest.fn(() => Promise.resolve(false));
    const transport = createTransport(native, { askMicrophone: ask });
    await expect(transport.invoke("dictation_start")).rejects.toBe(MICROPHONE_DENIED);
    await expect(transport.invoke("phone_take_start", { publicKey: "k" })).rejects.toBe(
      MICROPHONE_DENIED,
    );
    expect(ask).toHaveBeenCalledTimes(2);
    expect(calls).toEqual([]);
    ask.mockResolvedValue(true);
    await transport.invoke("dictation_start");
    expect(calls).toEqual([{ command: "dictation_start", args: null }]);
  });

  it("hands every core event to the listener of voltip://event, and stops on unlisten", async () => {
    const { native, emit } = fakeNative();
    const transport = createTransport(native, { askMicrophone: () => Promise.resolve(true) });
    const seen: unknown[] = [];
    const unlisten = await transport.listen(UI_EVENT_NAME, (e) => {
      seen.push(e.payload);
    });
    emit("onEvent", { json: JSON.stringify({ type: "message", body: "你好" }) });
    emit("onEvent", { json: "not json" });
    unlisten();
    emit("onEvent", { json: JSON.stringify({ type: "message", body: "再见" }) });
    expect(seen).toEqual([{ type: "message", body: "你好" }]);
    const other = await transport.listen("something-else", () => {
      throw new Error("never");
    });
    other();
  });

  it("routes the level meter's frames to its stream, and forgets the stream on stop", async () => {
    const { native, calls, emit } = fakeNative({ audio_meter_start: () => 41 });
    const transport = createTransport(native, { askMicrophone: () => Promise.resolve(true) });
    const backend = new TauriBackend(transport);
    const frames: number[] = [];
    const stop = await backend.meter(undefined, (f) => {
      frames.push(f.seq);
    });
    const start = calls.find((c) => c.command === "audio_meter_start");
    const sent = start?.args as { onFrame: Record<string, number> };
    const channel = sent.onFrame[CHANNEL_KEY];
    expect(typeof channel).toBe("number");
    const frame = {
      rms_dbfs: -30,
      peak_dbfs: -12,
      clipping: false,
      sample_rate_hz: 48_000,
      channels: 1,
      seq: 7,
    };
    expect(levelFrameSchema.parse(frame)).toEqual(frame);
    emit("onChannel", { id: channel ?? -1, json: JSON.stringify(frame) });
    emit("onChannel", { id: 999, json: JSON.stringify({ ...frame, seq: 8 }) });
    expect(frames).toEqual([7]);
    stop();
    await Promise.resolve();
    expect(calls.at(-1)).toEqual({ command: "audio_meter_stop", args: { id: 41 } });
    emit("onChannel", { id: channel ?? -1, json: JSON.stringify({ ...frame, seq: 9 }) });
    expect(frames).toEqual([7]);
  });

  it("sends a raw body as base64 with its name and type", async () => {
    const { native, calls } = fakeNative({ feedback_attachment_add: () => ({ id: "a" }) });
    const transport = createTransport(native, { askMicrophone: () => Promise.resolve(true) });
    await transport.invokeRaw("feedback_attachment_add", new Uint8Array([1, 2, 3, 250]), {
      "x-voltip-name": encodeURIComponent("截图 1.png"),
      "x-voltip-type": "image/png",
    });
    expect(calls).toEqual([
      {
        command: "feedback_attachment_add",
        args: { data: "AQID+g==", name: "截图 1.png", type: "image/png" },
      },
    ]);
  });

  it("encodes base64 like the standard alphabet with padding", () => {
    const bytes = (text: string) => new TextEncoder().encode(text);
    expect(toBase64(bytes(""))).toBe("");
    expect(toBase64(bytes("f"))).toBe("Zg==");
    expect(toBase64(bytes("fo"))).toBe("Zm8=");
    expect(toBase64(bytes("foo"))).toBe("Zm9v");
    expect(toBase64(bytes("foobar"))).toBe("Zm9vYmFy");
    expect(toBase64(new Uint8Array([255, 254, 253]))).toBe("//79");
  });
});
