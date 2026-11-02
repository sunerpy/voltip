import { type ChannelLike, TauriBackend } from "./tauri-backend";
import { defaultSettings, idleSnapshot } from "./schema";
import { desktopIdentity } from "./mock-backend";

function fakeTransport() {
  const calls: { command: string; args: unknown }[] = [];
  let handler: ((e: { payload: unknown }) => void) | undefined;
  const unlisten = vi.fn();
  const warn = vi.fn();
  return {
    calls,
    warn,
    unlisten,
    fire(payload: unknown) {
      handler?.({ payload });
    },
    transport: {
      invoke: (command: string, args?: Record<string, unknown>) => {
        calls.push({ command, args });
        if (command === "core_state") {
          return Promise.resolve({
            identity: desktopIdentity(),
            settings: defaultSettings(),
            secret_backend: "keyring",
            relay: { state: "disconnected", attempts: 0 },
            pairing: idleSnapshot(),
            devices: [],
          });
        }
        return Promise.resolve(null);
      },
      listen: (_event: string, h: (e: { payload: unknown }) => void) => {
        handler = h;
        return Promise.resolve(unlisten);
      },
      warn,
    },
  };
}

describe("TauriBackend", () => {
  it("validates core_state and forwards commands with their args", async () => {
    const t = fakeTransport();
    const backend = new TauriBackend(t.transport);
    const state = await backend.getState();
    expect(state.secret_backend).toBe("keyring");
    await backend.invoke("pairing_start");
    await backend.invoke("pairing_join_code", { code: "483 921" });
    await backend.invoke("settings_set_theme", { theme: "dark", followSystem: false });
    expect(t.calls.map((c) => c.command)).toEqual([
      "core_state",
      "pairing_start",
      "pairing_join_code",
      "settings_set_theme",
    ]);
    expect(t.calls[2]?.args).toEqual({ code: "483 921" });
  });

  it("delivers validated events, drops invalid ones with a warning, and unlistens", async () => {
    const t = fakeTransport();
    const backend = new TauriBackend(t.transport);
    const listener = vi.fn();
    const off = backend.on(listener);
    await Promise.resolve();
    t.fire({ type: "error", message: "relay refused" });
    t.fire({ type: "devices", devices: "not-an-array" });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledWith({ type: "error", message: "relay refused" });
    expect(t.warn).toHaveBeenCalledTimes(1);
    off();
    t.fire({ type: "error", message: "after dispose" });
    expect(listener).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    expect(t.unlisten).toHaveBeenCalled();
  });

  it("regression: core_state is asked for only once every event listener is live", async () => {
    // Tauri's `listen` registers the listener with the shell in an IPC round trip; an event sent
    // before it is registered never reaches it. A state taken before then could miss a change
    // the listener never hears of: the relay shown 认证中 for good while it was connected
    // (user report 2026-11-02).
    const t = fakeTransport();
    let register: (() => void) | undefined;
    const backend = new TauriBackend({
      ...t.transport,
      listen: () =>
        new Promise((resolve) => {
          register = () => {
            resolve(t.unlisten);
          };
        }),
    });
    backend.on(() => undefined);
    const state = backend.getState();
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
    expect(t.calls.map((c) => c.command)).toEqual([]);
    register?.();
    await expect(state).resolves.toMatchObject({ secret_backend: "keyring" });
    expect(t.calls.map((c) => c.command)).toEqual(["core_state"]);
  });

  it("unlistens a registration that lands after its listener is gone", async () => {
    const t = fakeTransport();
    let land: (() => void) | undefined;
    const backend = new TauriBackend({
      ...t.transport,
      listen: () =>
        new Promise((resolve) => {
          land = () => {
            resolve(t.unlisten);
          };
        }),
    });
    const listener = vi.fn();
    backend.on(listener)();
    land?.();
    await Promise.resolve();
    await Promise.resolve();
    expect(t.unlisten).toHaveBeenCalledTimes(1);
    t.fire({ type: "error", message: "late" });
    expect(listener).not.toHaveBeenCalled();
  });

  it("asks for core_state even when a listener's registration failed", async () => {
    const t = fakeTransport();
    const backend = new TauriBackend({
      ...t.transport,
      listen: () => Promise.reject(new Error("no event plugin")),
    });
    backend.on(() => undefined);
    await expect(backend.getState()).resolves.toMatchObject({ secret_backend: "keyring" });
  });

  describe("regression: a listener whose registration was refused (gate 2026-11-02)", () => {
    // A refused registration used to leave the listener without events for good: a window that took
    // its first state while the relay authenticated showed 认证中 after the relay had connected.
    function refusedOnce() {
      const t = fakeTransport();
      let attempts = 0;
      let relay = "authenticating";
      let answer: (() => void) | undefined;
      let holding = false;
      const backend = new TauriBackend({
        ...t.transport,
        invoke: (command: string, args?: Record<string, unknown>) => {
          if (command !== "core_state") return t.transport.invoke(command, args);
          const snapshot = t.transport
            .invoke(command, args)
            .then((state) => ({ ...(state as object), relay: { state: relay, attempts: 0 } }));
          if (!holding) return snapshot;
          return new Promise((resolve) => {
            answer = () => {
              resolve(snapshot);
            };
          });
        },
        listen: (event: string, h: (e: { payload: unknown }) => void) => {
          attempts += 1;
          return attempts === 1
            ? Promise.reject(new Error("IPC not ready"))
            : t.transport.listen(event, h);
        },
      });
      return {
        t,
        backend,
        attempts: () => attempts,
        connect() {
          relay = "connected";
        },
        hold() {
          holding = true;
        },
        answer: () => answer?.(),
      };
    }

    beforeEach(() => {
      vi.useFakeTimers();
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    it("registers it again and hands it the whole state it missed", async () => {
      const r = refusedOnce();
      const seen: unknown[] = [];
      r.backend.on((e) => seen.push(e));
      await expect(r.backend.getState()).resolves.toMatchObject({
        relay: { state: "authenticating" },
      });
      // The relay connects while nothing listens.
      r.connect();
      expect(r.t.warn).toHaveBeenCalledWith(
        "voltip://event listener could not be registered; trying again",
        expect.any(Error),
      );
      await vi.advanceTimersByTimeAsync(250);
      expect(r.attempts()).toBe(2);
      expect(seen).toEqual([
        expect.objectContaining({
          type: "state",
          relay: expect.objectContaining({ state: "connected" }),
        }),
      ]);
      // From then on its events arrive as before.
      r.t.fire({ type: "error", message: "relay refused" });
      expect(seen.at(-1)).toEqual({ type: "error", message: "relay refused" });
    });

    it("hands it the events that came while that state was on its way after the state", async () => {
      const r = refusedOnce();
      const seen: unknown[] = [];
      r.backend.on((e) => seen.push(e));
      await r.backend.getState();
      r.hold();
      await vi.advanceTimersByTimeAsync(250);
      r.t.fire({ type: "error", message: "after the state was taken" });
      expect(seen).toEqual([]);
      r.answer();
      await vi.advanceTimersByTimeAsync(0);
      expect(seen.map((e) => (e as { type: string }).type)).toEqual(["state", "error"]);
    });

    it("still hands it the events when the whole state cannot be had", async () => {
      const t = fakeTransport();
      let attempts = 0;
      const backend = new TauriBackend({
        ...t.transport,
        invoke: () => Promise.reject(new Error("core_state refused")),
        listen: (event: string, h: (e: { payload: unknown }) => void) => {
          attempts += 1;
          return attempts === 1
            ? Promise.reject(new Error("IPC not ready"))
            : t.transport.listen(event, h);
        },
      });
      const seen: unknown[] = [];
      backend.on((e) => seen.push(e));
      await vi.advanceTimersByTimeAsync(250);
      t.fire({ type: "error", message: "relay refused" });
      expect(seen).toEqual([{ type: "error", message: "relay refused" }]);
      expect(t.warn).toHaveBeenCalledWith(
        "core_state for a late voltip://event listener failed",
        expect.any(Error),
      );
    });

    it("stops trying once the listener is gone", async () => {
      const t = fakeTransport();
      const listen = vi.fn(() => Promise.reject(new Error("IPC not ready")));
      const backend = new TauriBackend({ ...t.transport, listen });
      const off = backend.on(() => undefined);
      await vi.advanceTimersByTimeAsync(250 + 500);
      expect(listen).toHaveBeenCalledTimes(3);
      off();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(listen).toHaveBeenCalledTimes(3);
      // The first failure is the one that warns.
      expect(t.warn).toHaveBeenCalledTimes(1);
    });
  });

  it("rejects a malformed core_state instead of rendering garbage", async () => {
    const backend = new TauriBackend({
      invoke: () => Promise.resolve({ identity: null }),
      listen: () => Promise.resolve(() => undefined),
    });
    await expect(backend.getState()).rejects.toThrow(/invalid|expected|Invalid/);
  });

  it("falls back to console.warn when no warn sink is provided", async () => {
    const spy = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    let handler: ((e: { payload: unknown }) => void) | undefined;
    const backend = new TauriBackend({
      invoke: () => Promise.resolve(null),
      listen: (_e, h) => {
        handler = h;
        return Promise.resolve(() => undefined);
      },
    });
    backend.on(() => undefined);
    await Promise.resolve();
    handler?.({ payload: { type: "bogus" } });
    expect(spy).toHaveBeenCalled();
  });

  it("constructs with the real @tauri-apps/api transport by default", () => {
    expect(new TauriBackend()).toBeInstanceOf(TauriBackend);
  });

  it("audioDevices validates the native device list", async () => {
    const t = fakeTransport();
    const devices = [
      { id: "mic", name: "USB Mic", is_default: true, sample_rate_hz: 48000, channels: 1 },
    ];
    const backend = new TauriBackend({
      ...t.transport,
      invoke: (command) => Promise.resolve(command === "audio_devices" ? devices : null),
    });
    expect(await backend.audioDevices()).toEqual(devices);
    const broken = new TauriBackend({ ...t.transport, invoke: () => Promise.resolve([{ id: 1 }]) });
    await expect(broken.audioDevices()).rejects.toThrow(/invalid|expected/i);
  });

  it("updateStatus queries update_status and validates the updater status", async () => {
    const t = fakeTransport();
    const status = { state: "available", version: "2.1.0", current: "2.0.0" };
    const backend = new TauriBackend({
      ...t.transport,
      invoke: (command) => Promise.resolve(command === "update_status" ? status : null),
    });
    expect(await backend.updateStatus()).toEqual(status);
    const broken = new TauriBackend({
      ...t.transport,
      invoke: () => Promise.resolve({ state: "dancing" }),
    });
    await expect(broken.updateStatus()).rejects.toThrow(/invalid|expected/i);
  });

  it("regression: vocabularyPreview and rulesExport query the core with the camelCase arguments and validate the answers", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const answer = { corrected: "a", output: "b", corrections: [], rules: [] };
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(command === "rules_export" ? "version = 1\n" : answer);
      },
    });
    expect(await backend.vocabularyPreview("text")).toEqual(answer);
    const rule = {
      name: "r",
      kind: "literal" as const,
      pattern: "a",
      replacement: "b",
      case_sensitive: true,
      enabled: true,
    };
    await backend.vocabularyPreview("text", { id: "id-1", rule });
    expect(await backend.rulesExport()).toBe("version = 1\n");
    expect(calls).toStrictEqual([
      { command: "vocabulary_preview", args: { text: "text", draft: null } },
      { command: "vocabulary_preview", args: { text: "text", draft: { id: "id-1", rule } } },
      { command: "rules_export", args: undefined },
    ]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve({ corrected: 1 }),
    });
    await expect(broken.vocabularyPreview("x")).rejects.toThrow(/invalid|expected/i);
    await expect(broken.rulesExport()).rejects.toThrow(/invalid|expected/i);
  });

  it("the learning suggestions' queries (§16.10) send camelCase arguments and validate the answers", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const ready = {
      status: "ready",
      scanned: 2,
      refined: 1,
      skipped: 0,
      total: 0,
      repeated: 0,
      pairs: [],
    };
    const undo = { id: "u", entry_id: "e", term: "Voltip", created: true, previous_heard_as: [] };
    const answers: Record<string, unknown> = {
      vocabulary_suggestions: ready,
      vocabulary_suggestion_accept: { kind: "accepted", undo },
      vocabulary_suggestion_undo: { kind: "undone" },
      rules_warnings: [{ kind: "duplicate", rule_id: "a", other_id: "b", other_name: "B" }],
    };
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answers[command]);
      },
    });
    expect(await backend.vocabularySuggestions()).toEqual(ready);
    expect(await backend.vocabularySuggestions(20)).toEqual(ready);
    expect(
      await backend.vocabularySuggestionAccept({
        heard: "维他命",
        polished: "Voltip",
        historyId: "h",
      }),
    ).toEqual({ kind: "accepted", undo });
    expect(await backend.vocabularySuggestionUndo("u")).toEqual({ kind: "undone" });
    expect(await backend.rulesWarnings()).toHaveLength(1);
    expect(calls).toStrictEqual([
      { command: "vocabulary_suggestions", args: { limit: null } },
      { command: "vocabulary_suggestions", args: { limit: 20 } },
      {
        command: "vocabulary_suggestion_accept",
        args: { heard: "维他命", polished: "Voltip", historyId: "h" },
      },
      { command: "vocabulary_suggestion_undo", args: { id: "u" } },
      { command: "rules_warnings", args: undefined },
    ]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve({ status: "ready" }),
    });
    await expect(broken.vocabularySuggestions()).rejects.toThrow(/invalid|expected/i);
    await expect(
      broken.vocabularySuggestionAccept({ heard: "a", polished: "b", historyId: "h" }),
    ).rejects.toThrow(/invalid|expected/i);
    await expect(broken.vocabularySuggestionUndo("u")).rejects.toThrow(/invalid|expected/i);
    await expect(broken.rulesWarnings()).rejects.toThrow(/invalid|expected/i);
  });

  it("the learning suggestions' batches (§16.10) send their pairs as they are and validate the answers", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const accepted = {
      items: [
        { kind: "accepted", entry_id: "e", term: "飞书", created: true },
        { kind: "refused", refusal: { kind: "full", max: 500 } },
      ],
      undo: "b1",
    };
    const answers: Record<string, unknown> = {
      vocabulary_suggestions_accept: accepted,
      vocabulary_suggestions_ignore: { kind: "ignored", undo: "b2" },
      vocabulary_suggestions_undo: { kind: "changed", term: "飞书" },
    };
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answers[command]);
      },
    });
    const picks = [
      { heard: "非书", polished: "飞书", history_id: "h1" },
      { heard: "维他命", polished: "Voltip", history_id: "h2" },
    ];
    expect(await backend.vocabularySuggestionsAccept(picks)).toEqual(accepted);
    expect(await backend.vocabularySuggestionsIgnore([{ heard: "非", polished: "飞" }])).toEqual({
      kind: "ignored",
      undo: "b2",
    });
    expect(await backend.vocabularySuggestionsUndo("b1")).toEqual({
      kind: "changed",
      term: "飞书",
    });
    expect(calls).toStrictEqual([
      { command: "vocabulary_suggestions_accept", args: { pairs: picks } },
      {
        command: "vocabulary_suggestions_ignore",
        args: { pairs: [{ heard: "非", polished: "飞" }] },
      },
      { command: "vocabulary_suggestions_undo", args: { id: "b1" } },
    ]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve({ kind: "accepted" }),
    });
    await expect(broken.vocabularySuggestionsAccept(picks)).rejects.toThrow(/invalid|expected/i);
    await expect(broken.vocabularySuggestionsIgnore([])).rejects.toThrow(/invalid|expected/i);
    await expect(broken.vocabularySuggestionsUndo("b1")).rejects.toThrow(/invalid|expected/i);
  });

  it("regression: recentApps queries recent_apps without arguments and validates the list", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const apps = [{ id: "code", name: "Code" }];
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(apps);
      },
    });
    expect(await backend.recentApps()).toEqual(apps);
    expect(calls).toStrictEqual([{ command: "recent_apps", args: undefined }]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve([{ id: "code" }]),
    });
    await expect(broken.recentApps()).rejects.toThrow(/invalid|expected/i);
  });

  it("scenesBuiltin queries scenes_builtin without arguments and validates the packs", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const packs = [{ id: "coding", terms: ["API"] }];
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(packs);
      },
    });
    expect(await backend.scenesBuiltin()).toEqual(packs);
    expect(calls).toStrictEqual([{ command: "scenes_builtin", args: undefined }]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve([{ id: "cooking", terms: [] }]),
    });
    await expect(broken.scenesBuiltin()).rejects.toThrow(/invalid|expected/i);
  });

  it("presetsBuiltin queries presets_builtin without arguments and validates the texts", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const texts = [{ id: "proofread", prompt: "你是语音听写的校对。" }];
    const backend = new TauriBackend({
      ...fakeTransport().transport,
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(texts);
      },
    });
    expect(await backend.presetsBuiltin()).toEqual(texts);
    expect(calls).toStrictEqual([{ command: "presets_builtin", args: undefined }]);
    const broken = new TauriBackend({
      ...fakeTransport().transport,
      invoke: () => Promise.resolve([{ id: "casual", prompt: "x" }]),
    });
    await expect(broken.presetsBuiltin()).rejects.toThrow(/invalid|expected/i);
  });

  it("meter streams validated frames through a Channel and stops through audio_meter_stop", async () => {
    const t = fakeTransport();
    const channel: ChannelLike = { onmessage: () => undefined };
    const backend = new TauriBackend({
      ...t.transport,
      channel: () => channel,
      invoke: (command, args) => {
        t.calls.push({ command, args });
        // The shell answers `audio_meter_start` with the subscription id.
        return Promise.resolve(command === "audio_meter_start" ? 7 : null);
      },
    });
    const frames: unknown[] = [];
    const stop = await backend.meter("mic", (f) => frames.push(f));
    expect(t.calls.at(-1)).toEqual({
      command: "audio_meter_start",
      args: { deviceId: "mic", onFrame: channel },
    });
    const frame = {
      rms_dbfs: -18.4,
      peak_dbfs: -6.1,
      clipping: false,
      sample_rate_hz: 48000,
      channels: 1,
      seq: 1,
    };
    channel.onmessage(frame);
    channel.onmessage({ bogus: true });
    expect(frames).toEqual([frame]);
    expect(t.warn).toHaveBeenCalledTimes(1);
    stop();
    stop();
    // regression: stop names the subscription it got, so other subscribers (the live pill, the
    // home card) keep their frames.
    expect(t.calls.filter((c) => c.command === "audio_meter_stop")).toEqual([
      { command: "audio_meter_stop", args: { id: 7 } },
    ]);
    // Frames after stop are ignored; an undefined device id becomes null on the wire.
    channel.onmessage({ ...frame, seq: 2 });
    expect(frames).toHaveLength(1);
    await backend.meter(undefined, () => undefined);
    expect(t.calls.at(-1)?.args).toMatchObject({ deviceId: null });
  });

  it("meter reports a failing audio_meter_stop through warn instead of throwing", async () => {
    const t = fakeTransport();
    const channel: ChannelLike = { onmessage: () => undefined };
    const backend = new TauriBackend({
      ...t.transport,
      channel: () => channel,
      invoke: (command) =>
        command === "audio_meter_stop"
          ? Promise.reject(new Error("gone"))
          : Promise.resolve(command === "audio_meter_start" ? 1 : null),
    });
    const stop = await backend.meter(undefined, () => undefined);
    stop();
    await new Promise((r) => {
      setTimeout(r, 0);
    });
    expect(t.warn).toHaveBeenCalledWith("audio_meter_stop failed", expect.any(Error));
  });
});
