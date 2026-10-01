import { historyEntries } from "./fixtures/history";
import {
  ALWAYS_ON_DESKTOP_ONLY,
  MOCK_ALWAYS_ON_PAUSE_MS,
  MOCK_ASR_MS,
  MOCK_AUDIO_OUTPUTS,
  MOCK_EXPORT_DIR,
  MOCK_PROCESS_ENTRY_GONE,
  MOCK_PROCESS_UNCONFIGURED,
  MOCK_AUDIO_DEVICES,
  MOCK_NEARBY,
  MOCK_TEXT_MS,
  PHONE_TEXT_UNAVAILABLE,
  MOCK_AVAILABLE_VERSION,
  MOCK_CURRENT_VERSION,
  MOCK_UPDATE_CHECK_MS,
  MOCK_UPDATE_TICK_MS,
  MOCK_UPDATE_TICKS,
  MOCK_UPDATE_TOTAL_BYTES,
  MOCK_DICTATION_DWELL_MS,
  MOCK_DICTATION_FAILED_DWELL_MS,
  MOCK_DICTATION_RAW,
  MOCK_DICTATION_TEXT,
  MOCK_COPY_MS,
  MOCK_EDIT_INSTRUCTION,
  MOCK_EDIT_TEXT,
  MOCK_EDIT_UNAVAILABLE,
  MOCK_EDIT_IN_TERMINAL,
  MOCK_NO_SELECTION,
  MOCK_EMPTY_STREAM_ERROR,
  MOCK_ENGINE_BUILTIN,
  MOCK_FINALIZE_MS,
  MOCK_HOTKEY_BACKEND,
  MOCK_LIVE_SCRIPT,
  MOCK_LIVE_STEP_MS,
  MOCK_METER_INTERVAL_MS,
  MOCK_MIC_READY_MS,
  MOCK_MODEL_CATALOGUE,
  MOCK_MODEL_FILE,
  MOCK_MODEL_TICK_MS,
  MOCK_MODEL_TICKS,
  MOCK_MODELS_ROOT,
  MOCK_PROBE_MS,
  MOCK_PRESET_SAMPLES,
  MOCK_BUILTIN_SCENES,
  MOCK_PUBLIC_KEYS,
  MOCK_REFINE_MS,
  MOCK_NO_SPEECH,
  MOCK_SCENE_MODE_NOT_READY,
  MOCK_STREAMING_MODEL_ID,
  MockBackend,
  mockSameChord,
  SCENES_UNAVAILABLE,
  SHARE_UNAVAILABLE,
  PRESET_TRY_UNCONFIGURED,
  VOCABULARY_UNAVAILABLE,
  mockLevel,
  desktopIdentity,
  desktopPeer,
  phoneIdentity,
  sampleDevices,
  sampleHistory,
  seededRandom,
  startOfLocalDay,
} from "./mock-backend";
import {
  DEFAULT_MAX_MINUTES,
  BUILTIN_PRESETS,
  BUILTIN_SCENES,
  HISTORY_LIMIT,
  HISTORY_RECENT,
  MAX_EDIT_SELECTION_CHARS,
  MAX_PRESETS,
  type PresetsTryArgs,
  MAX_PHONE_TEXT_CHARS,
  type DictionaryDraft,
  type HistoryEntry,
  NIL_ID,
  type RuleDraft,
  type SceneDraft,
  type LevelFrame,
  MAX_ACTIVATION_MS,
  type UiEvent,
  type UpdateStatus,
  defaultEngineSettings,
  defaultSettings,
  engineReady,
  providerStatus,
} from "./schema";
import { livePreviewText } from "./labels";

function collect(backend: MockBackend) {
  const events: UiEvent[] = [];
  backend.on((e) => events.push(e));
  return events;
}

async function flush() {
  await Promise.resolve();
}

describe("MockBackend pairing (desktop role)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("walks idle → creating_session → waiting_for_peer with code, QR ticket and countdown", async () => {
    const backend = new MockBackend({ now: () => 1_700_000_000_000 });
    const events = collect(backend);
    await backend.invoke("pairing_start");
    expect(backend.peek().pairing.state).toEqual({ state: "creating_session" });
    vi.advanceTimersByTime(300);
    const snap = backend.peek().pairing;
    expect(snap.state).toEqual({ state: "waiting_for_peer" });
    expect(snap.code).toMatch(/^\d{3} \d{3}$/);
    expect(snap.ticket_uri).toMatch(/^voltip:\/\/pair\?v=1&s=[0-9a-f]{16}&t=[0-9a-f]{32}$/);
    expect(snap.remaining_secs).toBe(120);
    expect(snap.expires_at).toBe(1_700_000_000 + 120);
    vi.advanceTimersByTime(3000);
    expect(backend.peek().pairing.remaining_secs).toBe(117);
    expect(events.filter((e) => e.type === "pairing").length).toBeGreaterThanOrEqual(5);
    backend.destroy();
  });

  it("regression: countdown reaching zero yields `expired` with remaining_secs 0 and stops ticking", async () => {
    const backend = new MockBackend({ ttlSecs: 3 });
    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    vi.advanceTimersByTime(3000);
    const snap = backend.peek().pairing;
    expect(snap.state).toEqual({ state: "expired" });
    expect(snap.remaining_secs).toBe(0);
    const count = backend.log.length;
    vi.advanceTimersByTime(5000);
    expect(backend.log.length).toBe(count);
    await backend.invoke("pairing_reset");
    expect(backend.peek().pairing.state).toEqual({ state: "idle" });
  });

  it("completes the full flow: peer joins, both confirm, device becomes trusted and online", async () => {
    const backend = new MockBackend({ now: () => 1_700_000_000_000 });
    const events = collect(backend);
    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    backend.simulatePeerJoined();
    expect(backend.peek().pairing.state).toEqual({ state: "key_exchange" });
    vi.advanceTimersByTime(400);
    const verifying = backend.peek().pairing;
    expect(verifying.state).toEqual({ state: "awaiting_verification" });
    expect(verifying.safety_code?.words).toHaveLength(4);
    expect(verifying.safety_code?.fingerprint).toMatch(/^[0-9A-F:]{11} · [0-9A-F:]{11}$/);
    expect(verifying.peer?.name).toBe("Pixel 8");
    await backend.invoke("pairing_confirm");
    expect(backend.peek().pairing.local_confirmed).toBe(true);
    expect(backend.peek().pairing.state).toEqual({ state: "awaiting_verification" });
    backend.simulatePeerConfirmed();
    const done = backend.peek();
    expect(done.pairing.state).toEqual({ state: "trusted" });
    expect(done.devices).toHaveLength(1);
    expect(done.devices[0]?.connection).toEqual({ state: "online", via: "direct" });
    expect(done.devices[0]?.device.public_key).toBe(MOCK_PUBLIC_KEYS.phone);
    expect(done.devices[0]?.device.trusted_at).toBe(1_700_000_000);
    expect(events.some((e) => e.type === "trusted")).toBe(true);
    // A second pairing with the same phone replaces the record instead of duplicating it.
    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    backend.simulatePeerJoined();
    vi.advanceTimersByTime(400);
    backend.simulatePeerConfirmed();
    await backend.invoke("pairing_confirm");
    expect(backend.peek().devices).toHaveLength(1);
  });

  it("auto-drives the peer when autoPeer is set", async () => {
    const backend = new MockBackend({ autoPeer: { joinAfterMs: 1000, confirmAfterMs: 500 } });
    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300 + 1000 + 400);
    expect(backend.peek().pairing.state).toEqual({ state: "awaiting_verification" });
    vi.advanceTimersByTime(500);
    expect(backend.peek().pairing.peer_confirmed).toBe(true);
    await backend.invoke("pairing_confirm");
    expect(backend.peek().pairing.state).toEqual({ state: "trusted" });
  });

  it("regression: rejection, cancel and peer-left end the session and ignore late confirmations", async () => {
    const backend = new MockBackend();
    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    backend.simulatePeerJoined();
    vi.advanceTimersByTime(400);
    await backend.invoke("pairing_reject");
    expect(backend.peek().pairing.state).toEqual({ state: "rejected" });
    backend.simulatePeerConfirmed();
    await backend.invoke("pairing_confirm");
    expect(backend.peek().pairing.state).toEqual({ state: "rejected" });

    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    await backend.invoke("pairing_cancel");
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "cancelled" },
    });
    backend.simulatePeerJoined();
    expect(backend.peek().pairing.state.state).toBe("failed");

    await backend.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    backend.simulatePeerLeft();
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "peer_left" },
    });
    backend.simulatePeerRejected();
    expect(backend.peek().pairing.state).toEqual({ state: "rejected" });
    await backend.invoke("pairing_reset");
    backend.simulatePeerRejected();
    expect(backend.peek().pairing.state).toEqual({ state: "idle" });
  });
});

describe("MockBackend pairing (phone role)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("joins with a six-digit code and reaches verification with the desktop as peer", async () => {
    const backend = new MockBackend({ role: "phone" });
    expect(backend.peek().identity?.platform).toBe("android");
    await backend.invoke("pairing_join_code", { code: "483 921" });
    expect(backend.peek().pairing.state).toEqual({ state: "creating_session" });
    vi.advanceTimersByTime(300);
    expect(backend.peek().pairing.state).toEqual({ state: "key_exchange" });
    vi.advanceTimersByTime(400);
    expect(backend.peek().pairing.state).toEqual({ state: "awaiting_verification" });
    expect(backend.peek().pairing.peer).toEqual(desktopPeer());
    backend.simulatePeerConfirmed();
    await backend.invoke("pairing_confirm");
    expect(backend.peek().pairing.state).toEqual({ state: "trusted" });
    expect(backend.peek().devices[0]?.device.platform).toBe("windows");
  });

  it("rejects malformed or unexpected codes with a relay invalid_code failure", async () => {
    const backend = new MockBackend({ role: "phone", expectedCode: "111 222" });
    await backend.invoke("pairing_join_code", { code: "12" });
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "relay", code: "invalid_code" },
    });
    await backend.invoke("pairing_join_code", { code: "333333" });
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "relay", code: "invalid_code" },
    });
    await backend.invoke("pairing_join_code", { code: "111222" });
    expect(backend.peek().pairing.state).toEqual({ state: "creating_session" });
  });

  it("joins with a ticket URI and refuses foreign links", async () => {
    const backend = new MockBackend({
      role: "phone",
      autoPeer: { joinAfterMs: 0, confirmAfterMs: 100 },
    });
    await backend.invoke("pairing_join_ticket", { uri: "https://example.com/pair" });
    expect(backend.peek().pairing.state).toEqual({ state: "failed", reason: { kind: "protocol" } });
    await backend.invoke("pairing_join_ticket", { uri: "not a url" });
    expect(backend.peek().pairing.state).toEqual({ state: "failed", reason: { kind: "protocol" } });
    await backend.invoke("pairing_join_ticket", { uri: "voltip://pair?v=1&s=abcd&t=ef01" });
    vi.advanceTimersByTime(300 + 400 + 100);
    expect(backend.peek().pairing.peer_confirmed).toBe(true);
  });
});

describe("MockBackend devices, relay, identity and messages", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("forgets, refreshes and renames", async () => {
    const backend = new MockBackend({ devices: sampleDevices(1_700_000_000) });
    const events = collect(backend);
    await backend.invoke("device_forget", { publicKey: MOCK_PUBLIC_KEYS.laptop });
    expect(backend.peek().devices.map((d) => d.device.name)).toEqual(["Pixel 8"]);
    await backend.invoke("devices_refresh");
    expect(events.filter((e) => e.type === "devices")).toHaveLength(2);
    await backend.invoke("device_rename", { name: "Studio" });
    expect(backend.peek().identity?.name).toBe("Studio");
  });

  it("regression: identity change flags the device and never auto-trusts", () => {
    const backend = new MockBackend({ devices: sampleDevices(1_700_000_000) });
    const events = collect(backend);
    backend.simulateIdentityChanged(MOCK_PUBLIC_KEYS.phone, "FF:00:11:22 · 33:44:55:66");
    const row = backend.peek().devices.find((d) => d.device.public_key === MOCK_PUBLIC_KEYS.phone);
    expect(row?.connection).toEqual({
      state: "identity_changed",
      presented_fingerprint: "FF:00:11:22 · 33:44:55:66",
    });
    expect(row?.device.fingerprint).toBe(phoneIdentity().fingerprint);
    expect(events[0]?.type).toBe("identity_changed");
    backend.simulateIdentityChanged("deadbeef", "x");
    expect(events).toHaveLength(2);
  });

  it("tracks device connection changes and last_seen", () => {
    const backend = new MockBackend({
      devices: sampleDevices(1_700_000_000),
      now: () => 1_700_000_500_000,
    });
    backend.simulateDeviceConnection(MOCK_PUBLIC_KEYS.laptop, { state: "online", via: "relay" });
    const laptop = backend
      .peek()
      .devices.find((d) => d.device.public_key === MOCK_PUBLIC_KEYS.laptop);
    expect(laptop?.device.last_seen).toBe(1_700_000_500);
    expect(laptop?.device.last_connection).toBe("relay");
    backend.simulateDeviceConnection(MOCK_PUBLIC_KEYS.laptop, { state: "offline" });
    expect(backend.peek().devices[1]?.connection).toEqual({ state: "offline" });
  });

  it("regression: relay settings drive the link state and reconnect readouts", async () => {
    const backend = new MockBackend();
    await backend.invoke("settings_set_relay", { url: "wss://relay.example.test", enabled: true });
    expect(backend.peek().settings.relay_url).toBe("wss://relay.example.test");
    expect(backend.peek().relay).toEqual({
      endpoint: "wss://relay.example.test",
      source: "user",
      state: "connecting",
      attempts: 0,
    });
    vi.advanceTimersByTime(500);
    expect(backend.peek().relay.state).toBe("connected");
    backend.simulateRelay({ state: "reconnecting", attempts: 3 });
    expect(backend.peek().relay.attempts).toBe(3);
    await backend.invoke("settings_set_relay", { url: null, enabled: false });
    expect(backend.peek().settings.relay_url).toBeUndefined();
    expect(backend.peek().relay.state).toBe("disconnected");
  });

  it("echoes messages to online devices and errors for offline ones", async () => {
    const backend = new MockBackend({ devices: sampleDevices(1_700_000_000) });
    const events = collect(backend);
    await backend.invoke("send_text", { publicKey: MOCK_PUBLIC_KEYS.phone, body: "hi" });
    vi.advanceTimersByTime(150);
    expect(events.at(-1)).toEqual({ type: "message", from: MOCK_PUBLIC_KEYS.phone, body: "hi" });
    await backend.invoke("send_text", { publicKey: MOCK_PUBLIC_KEYS.laptop, body: "hi" });
    expect(events.at(-1)?.type).toBe("error");
    backend.simulateMessage("abc", "manual");
    backend.simulateError("boom");
    expect(events.at(-1)).toEqual({ type: "error", message: "boom" });
  });

  it("regression: the hotkey setting is validated like the core does and lands in settings", async () => {
    const backend = new MockBackend();
    const events: string[] = [];
    backend.on((e) => {
      events.push(e.type === "error" ? `error:${e.message}` : e.type);
    });
    await backend.invoke("settings_set_hotkey", { hotkey: "Ctrl+Shift+D" });
    expect((await backend.getState()).settings.hotkey).toBe("Ctrl+Shift+D");
    for (const bad of ["Space", "Ctrl+Alt", "Ctrl+A+B", "Ctrl++A"]) {
      await backend.invoke("settings_set_hotkey", { hotkey: bad });
    }
    expect((await backend.getState()).settings.hotkey).toBe("Ctrl+Shift+D");
    expect(events.filter((e) => e.startsWith("error:hotkey"))).toHaveLength(4);
    expect(events.filter((e) => e === "settings")).toHaveLength(1);
    await flush();
  });

  it("regression: opening the recorder suspends the hotkey registration and closing it restores the saved chord", async () => {
    const backend = new MockBackend({ settings: { hotkey: "Ctrl+Shift+D" } });
    expect((await backend.getState()).hotkey).toMatchObject({
      registered: "Ctrl+Shift+D",
      capturing: false,
    });
    await backend.invoke("hotkey_capture", { active: true });
    const suspended = (await backend.getState()).hotkey;
    expect(suspended.capturing).toBe(true);
    expect(suspended.registered).toBeUndefined();
    await backend.invoke("hotkey_capture", { active: false });
    expect((await backend.getState()).hotkey).toMatchObject({
      registered: "Ctrl+Shift+D",
      capturing: false,
      backend: MOCK_HOTKEY_BACKEND,
    });
    // A shell-published status (what `Bridge::publish` does) folds into the same state.
    backend.publish({
      type: "hotkey",
      pressed: true,
      capturing: false,
      backend: "x",
      registered: "Ctrl+Shift+D",
    });
    expect((await backend.getState()).hotkey.pressed).toBe(true);
    await flush();
  });

  it("audioDevices lists the sample microphones with the default first and meter streams a deterministic level", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend();
      const devices = await backend.audioDevices();
      expect(devices[0]?.is_default).toBe(true);
      expect(devices.map((d) => d.id)).toEqual(MOCK_AUDIO_DEVICES.map((d) => d.id));
      const frames: LevelFrame[] = [];
      const stop = await backend.meter(undefined, (f) => frames.push(f));
      expect(backend.activeMeters()).toBe(1);
      vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 3 + 1);
      expect(frames.map((f) => f.seq)).toEqual([1, 2, 3]);
      expect(frames[0]).toMatchObject({ sample_rate_hz: 48_000, channels: 1, clipping: false });
      expect(frames.every((f) => f.rms_dbfs < 0 && f.peak_dbfs > f.rms_dbfs)).toBe(true);
      expect(mockLevel(7)).toEqual(mockLevel(7));
      stop();
      expect(backend.activeMeters()).toBe(0);
      vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 3);
      expect(frames).toHaveLength(3);
      const stopSecond = await backend.meter(MOCK_AUDIO_DEVICES[1]?.id, (f) => frames.push(f));
      vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS + 1);
      expect(frames.at(-1)?.channels).toBe(2);
      stopSecond();
      // A choice that is not connected meters the default input until it is back, as takes do
      // (the shell's `connected_or_default`).
      const stopUnplugged = await backend.meter("Blue Yeti", (f) => frames.push(f));
      vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS + 1);
      expect(frames.at(-1)?.channels).toBe(devices[0]?.channels);
      stopUnplugged();
    } finally {
      vi.useRealTimers();
    }
  });

  it("sets theme, exposes getState clones, unsubscribes listeners and rejects missing args", async () => {
    const backend = new MockBackend({ settings: { theme: "warm" } });
    const listener = vi.fn();
    const off = backend.on(listener);
    await backend.invoke("settings_set_theme", { theme: "graphite", followSystem: true });
    expect(listener).toHaveBeenCalledTimes(1);
    const state = await backend.getState();
    expect(state.settings).toMatchObject({ theme: "graphite", follow_system_theme: true });
    off();
    await backend.invoke("devices_refresh");
    expect(listener).toHaveBeenCalledTimes(1);
    // @ts-expect-error -- exercising the runtime guard against missing arguments.
    await expect(backend.invoke("pairing_join_code")).rejects.toThrow("missing command arguments");
    await flush();
  });

  it("rename without identity is a no-op and seededRandom is deterministic", async () => {
    const backend = new MockBackend();
    const events = collect(backend);
    const r1 = seededRandom(7);
    const r2 = seededRandom(7);
    expect(r1()).toBe(r2());
    expect(events).toHaveLength(0);
    await backend.invoke("pairing_confirm");
    expect(events).toHaveLength(0);
  });
});

describe("MockBackend dictation pipeline (docs/dictation.md §2)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };

  it("regression: 开始听写 starts a real session and shows the phase; 停止 finishes with the inserted text", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    expect(backend.peek().dictation).toEqual({
      session: 0,
      phase: { phase: "idle" },
      kind: "dictation",
    });
    await backend.invoke("dictation_start");
    // The device has not delivered samples yet; the timer is re-based once it has (§11).
    // docs/dictation.md §22: a take on this computer names what it records.
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "listening", started_at: T0, ready: false, locked: false },
      kind: "dictation",
      source: "microphone",
    });
    tick(MOCK_MIC_READY_MS);
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "listening", started_at: T0 + MOCK_MIC_READY_MS, ready: true, locked: false },
      kind: "dictation",
      source: "microphone",
    });
    tick(3200);
    await backend.invoke("dictation_stop");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "processing",
      stage: "transcribing",
      started_at: T0 + MOCK_MIC_READY_MS + 3200,
      stage_started_at: T0 + MOCK_MIC_READY_MS + 3200,
    });
    tick(MOCK_ASR_MS);
    // Like the core: the run keeps its start, the step clock restarts (user feedback 2026-09-29).
    expect(backend.peek().dictation.phase).toMatchObject({
      started_at: T0 + MOCK_MIC_READY_MS + 3200,
      stage_started_at: T0 + MOCK_MIC_READY_MS + 3200 + MOCK_ASR_MS,
    });
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "processing",
      stage: "refining",
    });
    tick(MOCK_REFINE_MS);
    const done = backend.peek().dictation.phase;
    expect(done).toEqual({
      phase: "done",
      text: MOCK_DICTATION_TEXT,
      raw_text: MOCK_DICTATION_RAW,
      chars: MOCK_DICTATION_TEXT.length,
      via: "paste",
      refined: true,
      duration_ms: 3200,
      asr_ms: MOCK_ASR_MS,
      refine_ms: MOCK_REFINE_MS,
      mode: "whole_take",
    });
    // The history row lands before the done phase so a UI reading both sees them together.
    const history = backend.peek().history_recent;
    expect(history).toHaveLength(1);
    expect(history[0]).toMatchObject({
      text: MOCK_DICTATION_TEXT,
      raw_text: MOCK_DICTATION_RAW,
      refined: true,
      asr_model: MOCK_ENGINE_BUILTIN.asr_model,
      refine_model: MOCK_ENGINE_BUILTIN.refine_model,
      duration_ms: 3200,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
    });
    expect(history[0]?.id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-8[0-9a-f]{3}-[0-9a-f]{12}$/,
    );
    const order = events.map((e) => e.type);
    expect(order.indexOf("history")).toBeLessThan(order.lastIndexOf("dictation"));
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "idle" },
      kind: "dictation",
    });
    backend.destroy();
  });

  it("names what a take records and counts a long take's segments while it runs (docs/dictation.md §22); a phone's take records nothing here", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    backend.simulateLongTakeProgress(1, 2);
    expect(backend.peek().dictation).toEqual({
      session: 0,
      phase: { phase: "idle" },
      kind: "dictation",
    });
    await backend.invoke("settings_set_recording", {
      recording: { source: "system", output_device: null, max_minutes: 60, echo_cancel: true },
    });
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    expect(backend.peek().dictation.source).toBe("system");
    expect(backend.peek().dictation.segments).toBeUndefined();
    backend.simulateLongTakeProgress(3, 4);
    expect(backend.peek().dictation.segments).toEqual({ done: 3, total: 4 });
    await backend.invoke("dictation_stop");
    backend.simulateLongTakeProgress(4, 5);
    expect(backend.peek().dictation).toMatchObject({
      phase: { phase: "processing", stage: "transcribing" },
      source: "system",
      segments: { done: 4, total: 5 },
    });
    tick(MOCK_ASR_MS);
    tick(MOCK_REFINE_MS);
    // The count ends with the take; the source stays until the pill goes back to idle.
    expect(backend.peek().dictation.phase.phase).toBe("done");
    expect(backend.peek().dictation.segments).toBeUndefined();
    expect(backend.peek().dictation.source).toBe("system");
    backend.simulateLongTakeProgress(5, 5);
    expect(backend.peek().dictation.segments).toBeUndefined();
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "idle" },
      kind: "dictation",
    });
    backend.simulatePhoneTake("Pixel 8");
    expect(backend.peek().dictation.remote).toBe("Pixel 8");
    expect(backend.peek().dictation.source).toBeUndefined();
    backend.destroy();
  });

  it("answers audio_outputs like the shells: the computer's sound on two outputs here, none on the phone, or what a test sets (docs/dictation.md §22)", async () => {
    const desktop = await new MockBackend().audioOutputs();
    expect(desktop).toEqual({
      system_audio: { state: "available" },
      devices: [...MOCK_AUDIO_OUTPUTS],
    });
    desktop.devices.pop();
    expect((await new MockBackend().audioOutputs()).devices).toHaveLength(2);
    expect(await new MockBackend({ role: "phone" }).audioOutputs()).toEqual({
      system_audio: { state: "unsupported" },
      devices: [],
    });
    const old = { system_audio: { state: "macos_too_old" as const, version: "14.5" }, devices: [] };
    expect(await new MockBackend({ audioOutputs: old }).audioOutputs()).toEqual(old);
  });

  it("processes an entry like the core: parts at MOCK_REFINE_MS, 要点纪要's summary, the result stored; a cancel or a missing entry stores nothing; exports answer like the shell (docs/dictation.md §22)", async () => {
    const text = "今天的会议讨论了三件事。".repeat(300);
    const long: HistoryEntry = {
      id: "long",
      at_ms: T0,
      raw_text: text,
      text,
      refined: false,
      asr_model: "m",
      duration_ms: 600_000,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      segments: [{ text: "第一段。", start_ms: 0, end_ms: 1000 }],
    };
    const backend = new MockBackend({ now: () => clock, history: [long] });
    const events = collect(backend);
    const answers = () => events.flatMap((e) => (e.type === "history_process" ? [e.state] : []));
    await backend.invoke("history_process", { requestId: 1, id: "long", preset: "notes" });
    expect(answers()).toEqual([{ state: "running", done: 0, total: 4 }]);
    tick(MOCK_REFINE_MS * 4);
    const done = answers().at(-1);
    expect(answers().map((a) => a.state)).toEqual([
      "running",
      "running",
      "running",
      "running",
      "done",
    ]);
    expect(done).toMatchObject({
      state: "done",
      processed: { preset: { id: "notes" }, at_ms: clock },
    });
    expect(backend.peek().history_recent[0]?.processed).toEqual(
      done?.state === "done" ? done.processed : null,
    );
    await backend.invoke("history_process", { requestId: 2, id: "long", preset: "formal" });
    await backend.invoke("history_process_cancel", { requestId: 2 });
    await backend.invoke("history_process_cancel", { requestId: 99 });
    tick(MOCK_REFINE_MS * 5);
    expect(answers().at(-1)).toEqual({ state: "cancelled" });
    expect(backend.peek().history_recent[0]?.processed?.preset.id).toBe("notes");
    await backend.invoke("history_process", { requestId: 3, id: "gone", preset: "notes" });
    expect(answers().at(-1)).toEqual({ state: "failed", reason: MOCK_PROCESS_ENTRY_GONE });
    // Deleted while it ran.
    await backend.invoke("history_process", { requestId: 4, id: "long", preset: "proofread" });
    await backend.invoke("history_delete", { id: "long" });
    tick(MOCK_REFINE_MS * 3);
    expect(answers().at(-1)).toEqual({ state: "failed", reason: MOCK_PROCESS_ENTRY_GONE });
    const unconfigured = new MockBackend({ history: [long], builtIn: {} });
    const their = collect(unconfigured);
    await unconfigured.invoke("history_process", { requestId: 5, id: "long", preset: "notes" });
    expect(their.at(-1)).toMatchObject({
      type: "history_process",
      state: { state: "failed", reason: MOCK_PROCESS_UNCONFIGURED },
    });
    expect(await unconfigured.historyExport("long", "srt", "a")).toEqual({
      kind: "saved",
      path: `${MOCK_EXPORT_DIR}/a.srt`,
    });
    expect(await unconfigured.historyExport("long", "txt", "b")).toEqual({
      kind: "saved",
      path: `${MOCK_EXPORT_DIR}/b.txt`,
    });
    expect(unconfigured.exports).toEqual([
      { id: "long", format: "srt", fileName: "a" },
      { id: "long", format: "txt", fileName: "b" },
    ]);
    expect(await unconfigured.historyExport("none", "txt", "c")).toMatchObject({
      kind: "failed",
      code: "gone",
    });
    const plain = new MockBackend({ history: [{ ...long, segments: undefined }] });
    expect(await plain.historyExport("long", "srt", "d")).toMatchObject({
      kind: "failed",
      code: "empty",
    });
    backend.destroy();
  });

  it("skips the refine stage and pastes the raw text when refine is off; clipboard_only reports via clipboard", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), refine_enabled: false, inject: "clipboard_only" },
    });
    expect(backend.peek().engines).toMatchObject({
      refine_enabled: false,
      inject: "clipboard_only",
    });
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(1000);
    await backend.invoke("dictation_stop");
    tick(MOCK_ASR_MS);
    expect(backend.peek().dictation.phase).toEqual({
      phase: "done",
      text: MOCK_DICTATION_RAW,
      raw_text: MOCK_DICTATION_RAW,
      chars: MOCK_DICTATION_RAW.length,
      via: "clipboard",
      refined: false,
      duration_ms: 1000,
      asr_ms: MOCK_ASR_MS,
      mode: "whole_take",
    });
    expect(backend.peek().history_recent[0]).toMatchObject({
      refined: false,
      outcome: { kind: "inserted", via: "clipboard" },
    });
    expect(backend.peek().history_recent[0]?.refine_model).toBeUndefined();
    backend.destroy();
  });

  it("cancel discards the recording, a new start interrupts the dwell, stop/cancel while idle are no-ops", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("dictation_stop");
    await backend.invoke("dictation_cancel");
    expect(backend.log).toHaveLength(0);
    await backend.invoke("dictation_start");
    tick(500);
    await backend.invoke("dictation_cancel");
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "cancelled", injected_chars: 0 },
      kind: "dictation",
      source: "microphone",
    });
    expect(backend.peek().history_recent).toEqual([]);
    tick(1000);
    // Starting again mid-dwell moves straight to listening on a new session.
    await backend.invoke("dictation_start");
    expect(backend.peek().dictation).toEqual({
      session: 2,
      phase: { phase: "listening", started_at: T0 + 1500, ready: false, locked: false },
      kind: "dictation",
      source: "microphone",
    });
    tick(MOCK_MIC_READY_MS);
    tick(MOCK_DICTATION_DWELL_MS - MOCK_MIC_READY_MS);
    // The old dwell timer was cancelled: still listening, and the device is ready by now.
    expect(backend.peek().dictation.phase).toEqual({
      phase: "listening",
      started_at: T0 + 1500 + MOCK_MIC_READY_MS,
      ready: true,
      locked: false,
    });
    // Cancelling during processing also discards the run and writes no history.
    await backend.invoke("dictation_stop");
    tick(100);
    await backend.invoke("dictation_cancel");
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().history_recent).toEqual([]);
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    backend.destroy();
  });

  it("simulateDictationFailed reports the failure and dwells longer when the text was kept on the clipboard", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("dictation_start");
    backend.simulateDictationFailed("没有听到声音");
    expect(backend.peek().dictation.phase).toEqual({ phase: "failed", message: "没有听到声音" });
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    await backend.invoke("dictation_start");
    backend.simulateDictationFailed("粘贴超时", "kept text");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "failed",
      message: "粘贴超时",
      text: "kept text",
    });
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.phase.phase).toBe("failed");
    tick(MOCK_DICTATION_FAILED_DWELL_MS - MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    backend.destroy();
  });

  it("caps the in-memory history at the core's limit", async () => {
    const full: HistoryEntry[] = Array.from({ length: HISTORY_LIMIT }, (_, i) => ({
      id: `id-${i}`,
      at_ms: T0 - i,
      raw_text: "r",
      text: "t",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
    }));
    const backend = new MockBackend({ now: () => clock, history: full });
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), refine_enabled: false },
    });
    await backend.invoke("dictation_start");
    await backend.invoke("dictation_stop");
    tick(MOCK_ASR_MS);
    expect(backend.peek().history_total).toBe(HISTORY_LIMIT);
    expect(backend.peek().history_recent).toHaveLength(HISTORY_RECENT);
    expect(backend.peek().history_recent[0]?.text).toBe(MOCK_DICTATION_RAW);
    // The oldest entry went: the last one kept is the one before it.
    const oldest = await backend.historyQuery({ offset: HISTORY_LIMIT - 1, limit: 1 });
    expect(oldest.entries[0]?.id).toBe(`id-${HISTORY_LIMIT - 2}`);
    backend.destroy();
  });

  /** A desktop with the streaming model on disk: live preview is ready by default. */
  function liveBackend(live_preview = true) {
    return new MockBackend({
      now: () => clock,
      history: [],
      settings: { engines: { ...defaultEngineSettings(), live_preview } },
      models: {
        [MOCK_STREAMING_MODEL_ID]: {
          kind: "installed",
          path: `~/.local/share/voltip/models/${MOCK_STREAMING_MODEL_ID}`,
          installed_at: 1_758_600_000,
        },
      },
    });
  }

  it("regression: with live preview ready the mock emits ready → throttled partials → a committed sentence → processing with the preview (docs/dictation.md §11)", async () => {
    const backend = liveBackend();
    expect(backend.peek().engines.live_preview_ready).toBe(true);
    expect(backend.peek().engines.asr_provider).toBe("builtin");
    const events = collect(backend);
    await backend.invoke("dictation_start");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "listening",
      started_at: T0,
      ready: false,
      locked: false,
    });
    tick(MOCK_MIC_READY_MS);
    expect(backend.peek().dictation.phase).toEqual({
      phase: "listening",
      started_at: T0 + MOCK_MIC_READY_MS,
      ready: true,
      locked: false,
    });
    // Each step is one full `dictation` push with the script's LiveText.
    for (const [n, live] of MOCK_LIVE_SCRIPT.entries()) {
      tick(MOCK_LIVE_STEP_MS);
      expect(backend.peek().dictation.phase, `step ${n}`).toEqual({
        phase: "listening",
        started_at: T0 + MOCK_MIC_READY_MS,
        ready: true,
        live,
        locked: false,
      });
    }
    const last = MOCK_LIVE_SCRIPT.at(-1);
    if (!last) throw new Error("script");
    expect(last.committed).toHaveLength(1);
    expect(last.current.length).toBeGreaterThan(0);
    // The script is exhausted: more time changes nothing.
    tick(MOCK_LIVE_STEP_MS * 3);
    const phase = backend.peek().dictation.phase;
    expect(phase.phase === "listening" && phase.live).toEqual(last);
    // Stop: the preview (committed + current, CJK joined without spaces) rides into processing
    // and through the refine stage; the final text is the whole-take result as before.
    await backend.invoke("dictation_stop");
    const preview = livePreviewText(last);
    expect(preview).toBe("把这段逻辑抽成一个 helper，然后在 session assembly 里复用");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "processing",
      stage: "transcribing",
      started_at: T0 + MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * (MOCK_LIVE_SCRIPT.length + 3),
      stage_started_at: T0 + MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * (MOCK_LIVE_SCRIPT.length + 3),
      preview,
    });
    tick(MOCK_ASR_MS);
    expect(backend.peek().dictation.phase).toMatchObject({ stage: "refining", preview });
    tick(MOCK_REFINE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "done",
      text: MOCK_DICTATION_TEXT,
    });
    // One listening push per step plus ready and start, all on the same session.
    const dictation = events.filter((e) => e.type === "dictation");
    expect(dictation.filter((e) => e.phase.phase === "listening")).toHaveLength(
      MOCK_LIVE_SCRIPT.length + 2,
    );
    expect(new Set(dictation.map((e) => e.session))).toEqual(new Set([1]));
    backend.destroy();
  });

  it("regression: without the streaming model or with live_preview off there are no partials and no preview", async () => {
    for (const backend of [
      new MockBackend({ now: () => clock, history: [] }),
      liveBackend(false),
    ]) {
      expect(backend.peek().engines.live_preview_ready).toBe(false);
      await backend.invoke("dictation_start");
      tick(MOCK_MIC_READY_MS);
      tick(MOCK_LIVE_STEP_MS * (MOCK_LIVE_SCRIPT.length + 1));
      const listening = backend.peek().dictation.phase;
      expect(listening).toEqual({
        phase: "listening",
        started_at: T0 + MOCK_MIC_READY_MS,
        ready: true,
        locked: false,
      });
      expect(listening.phase === "listening" && "live" in listening).toBe(false);
      await backend.invoke("dictation_stop");
      const processing = backend.peek().dictation.phase;
      expect(processing.phase === "processing" && "preview" in processing).toBe(false);
      backend.destroy();
      clock = T0;
    }
  });

  it("regression: simulateLiveDegraded keeps the text shown so far, ignores later partials and leaves the final text alone; stop / cancel before ready drop the late ready mark", async () => {
    const backend = liveBackend();
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(MOCK_LIVE_STEP_MS * 2);
    const shown = MOCK_LIVE_SCRIPT[1];
    backend.simulateLiveDegraded("live tap overrun: the decoder fell behind the microphone");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "listening",
      started_at: T0 + MOCK_MIC_READY_MS,
      ready: true,
      live: { ...shown, degraded: "live tap overrun: the decoder fell behind the microphone" },
      locked: false,
    });
    tick(MOCK_LIVE_STEP_MS * MOCK_LIVE_SCRIPT.length);
    const phase = backend.peek().dictation.phase;
    expect(phase.phase === "listening" && phase.live?.current).toBe(shown?.current);
    await backend.invoke("dictation_stop");
    // The degraded preview still rides into processing; the final text is unaffected.
    expect(backend.peek().dictation.phase).toMatchObject({
      stage: "transcribing",
      preview: shown?.current,
    });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "done",
      text: MOCK_DICTATION_TEXT,
    });
    tick(MOCK_DICTATION_DWELL_MS);
    // Degrading while not listening is a no-op; a degraded mark before any partial creates an
    // empty LiveText so the home card can show the note.
    backend.simulateLiveDegraded("ignored");
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    backend.simulateLiveDegraded("open failed");
    expect(backend.peek().dictation.phase).toMatchObject({
      live: { committed: [], current: "", degraded: "open failed" },
    });
    await backend.invoke("dictation_cancel");
    tick(MOCK_DICTATION_DWELL_MS);
    // Stop before the device is ready: the ready timer must not resurrect `listening`.
    await backend.invoke("dictation_start");
    await backend.invoke("dictation_stop");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_DICTATION_DWELL_MS);
    await backend.invoke("dictation_start");
    await backend.invoke("dictation_cancel");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS);
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    backend.destroy();
  });
});

describe("MockBackend history, engines and secrets", () => {
  const NOW = 1_758_700_000_000;

  it("seeds the sample rows as real HistoryEntry records dated by local calendar day, newest first", () => {
    const backend = new MockBackend({ now: () => NOW });
    const history = backend.peek().history_recent;
    expect(history).toEqual(sampleHistory(NOW));
    // The protected-field placeholder (no text) is not a history entry.
    expect(history).toHaveLength(historyEntries.filter((r) => r.text.length > 0).length);
    for (let i = 1; i < history.length; i += 1)
      expect(history[i - 1]?.at_ms ?? 0).toBeGreaterThanOrEqual(history[i]?.at_ms ?? 0);
    const today = startOfLocalDay(NOW);
    const todays = history.filter((e) => e.at_ms >= today);
    expect(todays).toHaveLength(2);
    for (const e of todays) expect(e.at_ms).toBeLessThanOrEqual(NOW);
    expect(history.filter((e) => e.at_ms < today && e.at_ms >= today - 86_400_000)).toHaveLength(3);
    const kinds = history.map((e) => e.outcome.kind);
    expect(kinds.filter((k) => k === "inserted")).toHaveLength(4);
    expect(kinds.filter((k) => k === "clipboard")).toHaveLength(1);
    expect(kinds.filter((k) => k === "failed")).toHaveLength(1);
    expect(history.find((e) => e.refined)?.refine_model).toBe(MOCK_ENGINE_BUILTIN.refine_model);
    expect(history.find((e) => !e.refined)?.refine_model).toBeUndefined();
    // Just after midnight the "today" rows still land on today, in order, never in the future.
    const midnight = startOfLocalDay(NOW) + 60_000;
    const early = sampleHistory(midnight);
    for (const row of early.filter((e) => e.at_ms >= startOfLocalDay(midnight)))
      expect(row.at_ms).toBeLessThanOrEqual(midnight);
    // (Both today rows may sit on the day floor, so only non-increasing is required here.)
    for (let i = 1; i < early.length; i += 1)
      expect(early[i - 1]?.at_ms ?? 0).toBeGreaterThanOrEqual(early[i]?.at_ms ?? 0);
    // Mid-morning: the 14:32 row is pulled back but stays ahead of the 11:08 row.
    const morning = startOfLocalDay(NOW) + 11 * 3_600_000 + 10 * 60_000;
    const [first, second] = sampleHistory(morning);
    expect(first?.at_ms).toBeGreaterThan(second?.at_ms ?? 0);
    expect(first?.at_ms).toBeLessThanOrEqual(morning);
  });

  it("history_star, history_delete and history_clear are real on the in-memory list", async () => {
    const backend = new MockBackend({ now: () => NOW });
    const events = collect(backend);
    const first = backend.peek().history_recent[0];
    if (!first) throw new Error("fixture");
    await backend.invoke("history_star", { id: first.id, starred: !first.starred });
    expect(backend.peek().history_recent[0]?.starred).toBe(!first.starred);
    await backend.invoke("history_delete", { id: first.id });
    expect(backend.peek().history_recent.find((e) => e.id === first.id)).toBeUndefined();
    const remaining = backend.peek().history_recent.length;
    await backend.invoke("history_delete", { id: "nope" });
    expect(backend.peek().history_recent).toHaveLength(remaining);
    await backend.invoke("history_clear");
    expect(backend.peek().history_recent).toEqual([]);
    expect(events.every((e) => e.type === "history")).toBe(true);
    expect(events).toHaveLength(4);
  });

  it("resolves the built-in engines without their host, folds settings_set_engines into settings and re-emits engines", async () => {
    const backend = new MockBackend({ now: () => NOW });
    const engines = backend.peek().engines;
    expect(engines).toMatchObject({
      asr_provider: "builtin",
      asr_ready: true,
      asr_model: MOCK_ENGINE_BUILTIN.asr_model,
      asr_host: "",
      local_ready: false,
      live_preview_ready: false,
      effective_output_mode: "whole_take",
      refine_enabled: true,
      llm_provider: "builtin",
      refine_ready: true,
      refine_model: MOCK_ENGINE_BUILTIN.refine_model,
      refine_host: "",
      inject: "paste",
    });
    expect(engines.providers.map((p) => p.id)).toEqual([
      "builtin",
      "local",
      "openai",
      "groq",
      "siliconflow",
      "deepseek",
      "ollama",
      "custom",
    ]);
    expect(backend.peek().settings.engines).toEqual(defaultEngineSettings());
    const events = collect(backend);
    await backend.invoke("settings_set_engines", {
      engines: {
        ...defaultEngineSettings(),
        asr_provider: "custom",
        llm_provider: "custom",
        providers: {
          custom: {
            asr_url: "https://asr.corp.example/v1",
            asr_model: "whisper-large-v3",
            llm_url: "https://llm.corp.example",
            llm_model: "gpt-x",
          },
        },
        chinese_script: "traditional",
        language: "zh",
        refine_enabled: false,
        inject: "clipboard_only",
      },
    });
    expect(events.map((e) => e.type)).toEqual(["settings", "engines", "models"]);
    expect(backend.peek().settings.engines.providers?.custom?.asr_model).toBe("whisper-large-v3");
    expect(backend.peek().settings.engines.chinese_script).toBe("traditional");
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "custom",
      asr_ready: true,
      asr_host: "asr.corp.example",
      asr_model: "whisper-large-v3",
      language: "zh",
      refine_enabled: false,
      refine_host: "llm.corp.example",
      refine_model: "gpt-x",
      inject: "clipboard_only",
    });
    // Back to the defaults: the built-in service again, and still no host.
    await backend.invoke("settings_set_engines", { engines: defaultEngineSettings() });
    expect(backend.peek().engines.asr_host).toBe("");
    expect(backend.peek().engines.language).toBeUndefined();
  });

  it("a build without the built-in service falls back to the on-device model and no clean-up", () => {
    const backend = new MockBackend({ now: () => NOW, builtIn: {} });
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "local",
      asr_ready: false,
      asr_issue: "model_not_installed",
      local_model: "qwen3-asr-0.6b",
      refine_ready: false,
      refine_issue: "no_provider",
    });
    expect(backend.peek().engines.llm_provider).toBeUndefined();
    expect(backend.peek().engines.providers.some((p) => p.id === "builtin")).toBe(false);
    expect(backend.peek().models[0]?.active).toBe(true);
  });

  it("regression: provider_key_set flips only the set/source flags and never stores or echoes the value", async () => {
    const backend = new MockBackend({ now: () => NOW });
    const groq = () => providerStatus(backend.peek().engines, "groq");
    expect(groq()?.llm?.key).toEqual({ set: false, source: "none" });
    expect(groq()?.llm?.issue).toBe("key_missing");
    await backend.invoke("provider_key_set", {
      provider: "groq",
      kind: "llm",
      value: "gsk_secret_value",
    });
    expect(groq()?.llm?.key).toEqual({ set: true, source: "user" });
    expect(groq()?.asr?.key).toEqual({ set: true, source: "user" });
    expect(groq()?.llm?.issue).toBeUndefined();
    expect(JSON.stringify(backend.peek())).not.toContain("gsk_secret_value");
    expect(JSON.stringify(backend.log)).not.toContain("gsk_secret_value");
    await backend.invoke("provider_key_set", { provider: "groq", kind: "asr", value: null });
    expect(groq()?.llm?.key).toEqual({ set: false, source: "none" });
    // The custom endpoint keeps one key per service.
    await backend.invoke("provider_key_set", { provider: "custom", kind: "asr", value: "tok" });
    const custom = providerStatus(backend.peek().engines, "custom");
    expect(custom?.asr?.key.set).toBe(true);
    expect(custom?.llm?.key.set).toBe(false);
    expect(backend.log.every((e) => e.type === "engines")).toBe(true);
    // A key-less provider refuses one.
    await backend.invoke("provider_key_set", { provider: "local", kind: "asr", value: "x" });
    expect(backend.log.at(-1)).toMatchObject({ type: "error" });
  });

  it("provider_probe refuses what the core refuses up front and lists models otherwise", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend({
        now: () => NOW,
        probeModels: { custom: ["b", "a"] },
      });
      const events = collect(backend);
      await backend.invoke("provider_probe", { provider: "openai", kind: "asr" });
      expect(events.at(-1)).toMatchObject({
        type: "provider_probe",
        result: "failed",
        reason: "key_missing",
      });
      await backend.invoke("provider_probe", { provider: "custom", kind: "llm" });
      expect(events.at(-1)).toMatchObject({ result: "failed", reason: "invalid_url" });
      await backend.invoke("provider_probe", { provider: "local", kind: "asr" });
      expect(events.at(-1)).toMatchObject({ result: "failed", reason: "unsupported" });
      await backend.invoke("provider_probe", {
        provider: "custom",
        kind: "asr",
        baseUrl: "http://10.0.0.2:8000/v1",
      });
      vi.advanceTimersByTime(MOCK_PROBE_MS);
      expect(events.at(-1)).toMatchObject({ result: "ok", models: ["a", "b"] });
      await backend.invoke("provider_probe", { provider: "groq", kind: "llm", key: "gsk_draft" });
      vi.advanceTimersByTime(MOCK_PROBE_MS);
      expect(events.at(-1)).toMatchObject({ provider: "groq", result: "ok" });
      expect(JSON.stringify(backend.log)).not.toContain("gsk_draft");
      await backend.providerConsoleOpen("groq");
      expect(backend.consolesOpened).toEqual(["groq"]);
      await expect(backend.providerConsoleOpen("custom")).rejects.toThrow("custom: no key page");
      backend.destroy();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("MockBackend local models (docs/dictation.md §10)", () => {
  const NOW = 1_758_700_000_000;
  let clock = NOW;
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  beforeEach(() => {
    clock = NOW;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("seeds the five catalogue tiers as not installed and inactive; the phone has none", () => {
    const backend = new MockBackend({ now: () => clock });
    expect(backend.peek().models.map((m) => m.id)).toEqual([
      "qwen3-asr-0.6b",
      "qwen3-asr-1.7b",
      "sense-voice-small",
      "paraformer-zh",
      "zipformer-stream-zh-en",
    ]);
    expect(backend.peek().models.map((m) => m.state)).toEqual(
      MOCK_MODEL_CATALOGUE.map(() => ({ kind: "not_installed" })),
    );
    expect(backend.peek().models.every((m) => !m.active)).toBe(true);
    // Names and descriptions in the core's words; tiers, capabilities and sizes per docs §10 / §11.
    expect(backend.peek().models.map((m) => [m.name, m.engine, m.tier, m.capabilities])).toEqual([
      ["均衡", "transcribe_cpp", "balanced", ["offline"]],
      ["高精度", "transcribe_cpp", "accurate", ["offline"]],
      ["轻量", "sense_voice", "light", ["offline"]],
      ["轻量 · 中文", "paraformer", "light", ["offline"]],
      ["实时预览", "zipformer_streaming", "streaming", ["streaming"]],
    ]);
    expect(backend.peek().models.map((m) => m.recommended)).toEqual([
      true,
      false,
      false,
      false,
      false,
    ]);
    expect(MOCK_MODEL_CATALOGUE.map((m) => m.size_bytes)).toEqual([
      690_417_824, 1_692_554_208, 239_549_735, 227_405_559, 169_347_218,
    ]);
    expect(MOCK_STREAMING_MODEL_ID).toBe("zipformer-stream-zh-en");
    const phone = new MockBackend({ role: "phone", now: () => clock });
    expect(phone.peek().models).toEqual([]);
    backend.destroy();
    phone.destroy();
  });

  it("regression: model_download streams progress → verifying → installed and flips local_ready for the selected model", async () => {
    const backend = new MockBackend({ now: () => clock });
    await backend.invoke("settings_set_engines", {
      engines: {
        ...defaultEngineSettings(),
        asr_provider: "local",
        local_model: "sense-voice-small",
      },
    });
    // Local mode before the download: resolved to the model, host "local", not ready.
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "local",
      local_model: "sense-voice-small",
      local_ready: false,
      asr_host: "",
      asr_model: "轻量",
    });
    expect(engineReady(backend.peek().engines)).toBe(false);
    expect(backend.peek().models.map((m) => m.active)).toEqual([false, false, true, false, false]);
    const events = collect(backend);
    await backend.invoke("model_download", { id: "sense-voice-small" });
    const total = 239_549_735;
    const state = () => backend.peek().models[2]?.state;
    expect(state()).toEqual({ kind: "downloading", received: 0, total, file: MOCK_MODEL_FILE });
    for (let n = 1; n <= MOCK_MODEL_TICKS; n += 1) {
      tick(MOCK_MODEL_TICK_MS);
      expect(state()).toEqual({
        kind: "downloading",
        received: Math.round((total * n) / MOCK_MODEL_TICKS),
        total,
        file: MOCK_MODEL_FILE,
      });
    }
    tick(MOCK_MODEL_TICK_MS);
    expect(state()).toEqual({ kind: "verifying" });
    expect(backend.peek().engines.local_ready).toBe(false);
    tick(MOCK_MODEL_TICK_MS);
    expect(state()).toEqual({
      kind: "installed",
      path: `${MOCK_MODELS_ROOT}/sense-voice-small`,
      installed_at: Math.floor(clock / 1000),
    });
    expect(backend.peek().engines.local_ready).toBe(true);
    expect(engineReady(backend.peek().engines)).toBe(true);
    // Every progress step is a full `models` push; `engines` is re-emitted once, on readiness.
    expect(events.filter((e) => e.type === "models")).toHaveLength(MOCK_MODEL_TICKS + 3);
    expect(events.filter((e) => e.type === "engines")).toHaveLength(1);
    // A second download of an installed model is a no-op.
    await backend.invoke("model_download", { id: "sense-voice-small" });
    expect(state()?.kind).toBe("installed");
    backend.destroy();
  });

  it("regression: installing the on-device model clears 模型未下载 on its card while the built-in service is in use", async () => {
    // Screenshot check 2026-09-29: after 均衡 finished downloading, the 本机 card still said
    // 模型未下载 until something else re-reported the engines.
    const backend = new MockBackend({ now: () => clock });
    const localIssue = () =>
      backend.peek().engines.providers.find((p) => p.id === "local")?.asr?.issue;
    expect(backend.peek().engines.asr_provider).toBe("builtin");
    expect(localIssue()).toBe("model_not_installed");
    const events = collect(backend);
    const id = backend.peek().models[0]?.id ?? "";
    await backend.invoke("model_download", { id });
    for (let n = 0; n <= MOCK_MODEL_TICKS + 1; n += 1) tick(MOCK_MODEL_TICK_MS);
    expect(backend.peek().models[0]?.state.kind).toBe("installed");
    expect(localIssue()).toBeUndefined();
    expect(backend.peek().engines.asr_provider).toBe("builtin");
    expect(events.filter((e) => e.type === "engines")).toHaveLength(1);
    backend.destroy();
  });

  it("regression: cancel returns to not_installed, simulateModelFailed keeps the failure until a retry succeeds", async () => {
    const backend = new MockBackend({ now: () => clock });
    const state = () => backend.peek().models[3]?.state;
    await backend.invoke("model_download", { id: "paraformer-zh" });
    tick(MOCK_MODEL_TICK_MS);
    expect(state()?.kind).toBe("downloading");
    await backend.invoke("model_cancel", { id: "paraformer-zh" });
    expect(state()).toEqual({ kind: "not_installed" });
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
    expect(state()).toEqual({ kind: "not_installed" });
    // Cancelling something that is not downloading is a no-op.
    await backend.invoke("model_cancel", { id: "paraformer-zh" });
    expect(state()).toEqual({ kind: "not_installed" });
    // A failure mid-way: the timer stops, the message is kept, retry = download again.
    await backend.invoke("model_download", { id: "paraformer-zh" });
    tick(MOCK_MODEL_TICK_MS);
    backend.simulateModelFailed("paraformer-zh", "sha256 mismatch · model.int8.onnx");
    expect(state()).toEqual({ kind: "failed", message: "sha256 mismatch · model.int8.onnx" });
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
    expect(state()?.kind).toBe("failed");
    backend.simulateModelFailed("nope", "ignored");
    await backend.invoke("model_download", { id: "paraformer-zh" });
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 2));
    expect(state()?.kind).toBe("installed");
    // Removing deletes the directory; removing an absent model is a no-op.
    await backend.invoke("model_remove", { id: "paraformer-zh" });
    expect(state()).toEqual({ kind: "not_installed" });
    await backend.invoke("model_remove", { id: "paraformer-zh" });
    expect(state()).toEqual({ kind: "not_installed" });
    backend.destroy();
  });

  it("regression: activation writes settings_set_engines; removing the active model drops local_ready; unknown ids are refused", async () => {
    const backend = new MockBackend({
      now: () => clock,
      models: {
        "sense-voice-small": {
          kind: "installed",
          path: `${MOCK_MODELS_ROOT}/sense-voice-small`,
          installed_at: 1_758_600_000,
        },
      },
    });
    expect(backend.peek().models[2]?.state.kind).toBe("installed");
    expect(backend.peek().engines.asr_provider).toBe("builtin");
    // `local_model: null` in local mode resolves to the catalogue default (qwen3-asr-0.6b, not
    // installed here): not ready.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), asr_provider: "local" },
    });
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "local",
      local_model: "qwen3-asr-0.6b",
      local_ready: false,
      asr_host: "",
      asr_model: "均衡",
    });
    expect(backend.peek().models.map((m) => m.active)).toEqual([true, false, false, false, false]);
    // Picking the installed light model: active moves, readiness follows.
    await backend.invoke("settings_set_engines", {
      engines: {
        ...defaultEngineSettings(),
        asr_provider: "local",
        local_model: "sense-voice-small",
      },
    });
    expect(backend.peek().engines).toMatchObject({
      local_model: "sense-voice-small",
      local_ready: true,
      asr_model: "轻量",
    });
    expect(backend.peek().models.map((m) => m.active)).toEqual([false, false, true, false, false]);
    // Picking the other (not installed) light model: readiness drops.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), asr_provider: "local", local_model: "paraformer-zh" },
    });
    expect(backend.peek().engines).toMatchObject({
      local_model: "paraformer-zh",
      local_ready: false,
      asr_model: "轻量 · 中文",
    });
    expect(backend.peek().models.map((m) => m.active)).toEqual([false, false, false, true, false]);
    // Back to the built-in service: nothing is active, no host, local fields cleared.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), local_model: "sense-voice-small" },
    });
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "builtin",
      local_ready: false,
      asr_host: "",
    });
    expect(backend.peek().engines.local_model).toBeUndefined();
    expect(backend.peek().models.every((m) => !m.active)).toBe(true);
    // Removing the model the local mode points at makes it not ready again.
    await backend.invoke("settings_set_engines", {
      engines: {
        ...defaultEngineSettings(),
        asr_provider: "local",
        local_model: "sense-voice-small",
      },
    });
    expect(backend.peek().engines.local_ready).toBe(true);
    const events = collect(backend);
    await backend.invoke("model_remove", { id: "sense-voice-small" });
    expect(backend.peek().engines.local_ready).toBe(false);
    expect(events.map((e) => e.type)).toEqual(["models", "engines"]);
    // Unknown ids: SetEngines refuses (nothing changes), download reports an error.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), asr_provider: "local", local_model: "whisper" },
    });
    expect(backend.peek().settings.engines.local_model).toBe("sense-voice-small");
    await backend.invoke("model_download", { id: "whisper" });
    expect(events.filter((e) => e.type === "error").map((e) => e.message)).toEqual([
      "engines: 未知的本地模型 whisper",
      "models: 未知的本地模型 whisper",
    ]);
    const phone = new MockBackend({ role: "phone", now: () => clock });
    const phoneEvents = collect(phone);
    await phone.invoke("model_download", { id: "sense-voice-small" });
    expect(phoneEvents).toEqual([{ type: "error", message: "models: 手机端不支持本地模型" }]);
    // destroy() stops an in-flight download.
    await backend.invoke("model_download", { id: "paraformer-zh" });
    backend.destroy();
    phone.destroy();
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
    expect(backend.peek().models[3]?.state.kind).toBe("downloading");
  });

  it("regression: live_preview_ready is the switch plus the streaming model's install state, whatever the provider is; the switch folds into settings", async () => {
    const backend = new MockBackend({ now: () => clock });
    expect(backend.peek().settings.engines.live_preview).toBe(true);
    expect(backend.peek().engines.live_preview_ready).toBe(false);
    const events = collect(backend);
    // Installing the streaming model flips readiness (one `engines` push, local_ready untouched).
    await backend.invoke("model_download", { id: MOCK_STREAMING_MODEL_ID });
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
    expect(backend.peek().models[4]?.state.kind).toBe("installed");
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "builtin",
      local_ready: false,
      live_preview_ready: true,
    });
    expect(events.filter((e) => e.type === "engines")).toHaveLength(1);
    // The streaming model is never the recognition model: local mode still resolves the default.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), asr_provider: "local" },
    });
    expect(backend.peek().engines).toMatchObject({
      local_model: "qwen3-asr-0.6b",
      live_preview_ready: true,
    });
    expect(backend.peek().models[4]?.active).toBe(false);
    // Switching live preview off: the setting is persisted and readiness drops at once.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), live_preview: false },
    });
    expect(backend.peek().settings.engines.live_preview).toBe(false);
    expect(backend.peek().engines.live_preview_ready).toBe(false);
    await backend.invoke("settings_set_engines", { engines: defaultEngineSettings() });
    expect(backend.peek().engines.live_preview_ready).toBe(true);
    // Removing the streaming model drops readiness again.
    await backend.invoke("model_remove", { id: MOCK_STREAMING_MODEL_ID });
    expect(backend.peek().engines.live_preview_ready).toBe(false);
    backend.destroy();
  });
});

describe("MockBackend locale, auto-update and updater (docs/frontend.md §7)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("settings_set_locale and settings_set_auto_update fold into settings", async () => {
    const backend = new MockBackend();
    expect(backend.peek().settings.locale).toBe("system");
    expect(backend.peek().settings.auto_update).toBe(false);
    await backend.invoke("settings_set_locale", { locale: "en" });
    await backend.invoke("settings_set_auto_update", { enabled: true });
    expect(backend.peek().settings.locale).toBe("en");
    expect(backend.peek().settings.auto_update).toBe(true);
    expect(backend.log.filter((e) => e.type === "settings")).toHaveLength(2);
    // A seeded locale survives construction.
    expect(new MockBackend({ settings: { locale: "zh-cn" } }).peek().settings.locale).toBe("zh-cn");
  });

  it("settings_set_recording folds into settings and refuses what the core refuses (docs/dictation.md section 22)", async () => {
    const backend = new MockBackend();
    expect(backend.peek().settings.recording).toEqual({
      source: "microphone",
      output_device: null,
      max_minutes: DEFAULT_MAX_MINUTES,
      echo_cancel: true,
    });
    // Echo cancellation off (docs/dictation.md §22.6): kept as saved, like the rest.
    const mixed = {
      source: "mixed" as const,
      output_device: "fake:speakers",
      max_minutes: 60,
      echo_cancel: false,
    };
    await backend.invoke("settings_set_recording", { recording: mixed });
    expect(backend.peek().settings.recording).toEqual(mixed);
    for (const bad of [
      { ...mixed, max_minutes: 15 },
      { ...mixed, output_device: " " },
      { ...mixed, output_device: "x".repeat(1025) },
    ]) {
      await backend.invoke("settings_set_recording", { recording: bad });
      const last = backend.log.at(-1);
      expect(last?.type === "error" && last.message.startsWith("recording.")).toBe(true);
    }
    expect(backend.peek().settings.recording).toEqual(mixed);
  });

  it("update_check answers checking → available and update_install streams download → ready → installing", async () => {
    const backend = new MockBackend();
    expect(backend.peek().update).toEqual({ state: "idle" });
    expect(await backend.updateStatus()).toEqual({ state: "idle" });
    await backend.invoke("update_check");
    expect(backend.peek().update.state).toBe("checking");
    // A second check while checking is a no-op.
    await backend.invoke("update_check");
    expect(backend.log.filter((e) => e.type === "update")).toHaveLength(1);
    vi.advanceTimersByTime(MOCK_UPDATE_CHECK_MS);
    expect(backend.peek().update).toMatchObject({
      state: "available",
      version: MOCK_AVAILABLE_VERSION,
      current: MOCK_CURRENT_VERSION,
    });
    await backend.invoke("update_install");
    const ticks: UpdateStatus[] = [];
    for (let i = 0; i < MOCK_UPDATE_TICKS; i += 1) {
      ticks.push(backend.peek().update);
      vi.advanceTimersByTime(MOCK_UPDATE_TICK_MS);
    }
    expect(ticks.map((u) => u.state)).toEqual(["downloading", "downloading", "downloading"]);
    const received = ticks.map((u) => (u.state === "downloading" ? u.received : -1));
    expect(received).toEqual([16_000_000, 32_000_000, MOCK_UPDATE_TOTAL_BYTES]);
    expect(ticks.map((u) => (u.state === "downloading" ? u.total : -1))).toEqual([
      MOCK_UPDATE_TOTAL_BYTES,
      MOCK_UPDATE_TOTAL_BYTES,
      MOCK_UPDATE_TOTAL_BYTES,
    ]);
    expect(backend.peek().update).toEqual({ state: "ready", version: MOCK_AVAILABLE_VERSION });
    // install while downloading / installing is ignored; a second install from ready re-emits installing.
    vi.advanceTimersByTime(MOCK_UPDATE_TICK_MS);
    expect(backend.peek().update).toEqual({ state: "installing", version: MOCK_AVAILABLE_VERSION });
    await backend.invoke("update_install");
    await backend.invoke("update_check");
    expect(backend.peek().update.state).toBe("installing");
    expect(await backend.updateStatus()).toEqual({
      state: "installing",
      version: MOCK_AVAILABLE_VERSION,
    });
  });

  it("update_install from ready installs at once; from idle it is ignored", async () => {
    const backend = new MockBackend({ update: { state: "ready", version: "9.9.9" } });
    await backend.invoke("update_install");
    expect(backend.peek().update).toEqual({ state: "installing", version: "9.9.9" });
    const idle = new MockBackend();
    await idle.invoke("update_install");
    expect(idle.peek().update).toEqual({ state: "idle" });
  });

  it("simulateUpdate reaches disabled / failed, and a disabled build never checks", async () => {
    const backend = new MockBackend({ update: { state: "disabled" } });
    await backend.invoke("update_check");
    expect(backend.peek().update).toEqual({ state: "disabled" });
    backend.simulateUpdate({ state: "failed", message: "offline" });
    expect(backend.peek().update).toEqual({ state: "failed", message: "offline" });
    await backend.invoke("update_check");
    expect(backend.peek().update.state).toBe("checking");
    backend.destroy();
    vi.advanceTimersByTime(MOCK_UPDATE_CHECK_MS);
    // destroy() cleared the pending check.
    expect(backend.peek().update.state).toBe("checking");
  });
});

describe("MockBackend activation (docs/dictation.md §13)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const edge = (backend: MockBackend, pressed: boolean, source?: "hotkey" | "cli" | "ui") =>
    backend.invoke("hotkey_edge", source === undefined ? { pressed } : { pressed, source });

  it("regression: settings_set_activation persists all three values and re-emits settings; a value above MAX_ACTIVATION_MS is refused with an error event and nothing changes", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    expect(backend.peek().settings).toMatchObject({
      activation: "hold",
      hold_threshold_ms: 300,
      extra_recording_ms: 0,
    });
    const events = collect(backend);
    await backend.invoke("settings_set_activation", {
      activation: "hold_or_toggle",
      holdThresholdMs: 450,
      extraRecordingMs: 200,
    });
    expect(backend.peek().settings).toMatchObject({
      activation: "hold_or_toggle",
      hold_threshold_ms: 450,
      extra_recording_ms: 200,
    });
    expect(events.map((e) => e.type)).toEqual(["settings"]);
    await backend.invoke("settings_set_activation", {
      activation: "toggle",
      holdThresholdMs: MAX_ACTIVATION_MS + 1,
      extraRecordingMs: 0,
    });
    await backend.invoke("settings_set_activation", {
      activation: "toggle",
      holdThresholdMs: 300,
      extraRecordingMs: MAX_ACTIVATION_MS + 1,
    });
    expect(events.slice(1).map((e) => e.type)).toEqual(["error", "error"]);
    expect(events[1]?.type === "error" && events[1].message).toMatch(/5000/);
    expect(backend.peek().settings.activation).toBe("hold_or_toggle");
    expect(backend.peek().settings.hold_threshold_ms).toBe(450);
    // The cap itself is fine.
    await backend.invoke("settings_set_activation", {
      activation: "toggle",
      holdThresholdMs: MAX_ACTIVATION_MS,
      extraRecordingMs: MAX_ACTIVATION_MS,
    });
    expect(backend.peek().settings.activation).toBe("toggle");
    // The other settings are untouched.
    const {
      activation: _a,
      hold_threshold_ms: _h,
      extra_recording_ms: _e,
      ...rest
    } = backend.peek().settings;
    const {
      activation: _da,
      hold_threshold_ms: _dh,
      extra_recording_ms: _de,
      ...defaults
    } = defaultSettings();
    expect(rest).toEqual(defaults);
    backend.destroy();
  });

  it("regression: hold mode starts on press and stops on release; a press while listening is ignored (auto-repeat)", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await edge(backend, true, "hotkey");
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "listening", locked: false });
    tick(MOCK_MIC_READY_MS);
    await edge(backend, true, "hotkey");
    expect(backend.peek().dictation).toMatchObject({ session: 1, phase: { phase: "listening" } });
    tick(1000);
    await edge(backend, false, "hotkey");
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "processing",
      stage: "transcribing",
    });
    // A release while processing does nothing; the take finishes as a whole take.
    await edge(backend, false, "hotkey");
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "done", mode: "whole_take" });
    // Done / failed / cancelled count as idle (§13): a press starts the next take at once. While
    // processing the press is dropped (the core's pending press is not simulated).
    await edge(backend, true, "hotkey");
    expect(backend.peek().dictation).toMatchObject({ session: 2, phase: { phase: "listening" } });
    await edge(backend, false, "hotkey");
    await edge(backend, true, "hotkey");
    expect(backend.peek().dictation).toMatchObject({ session: 2, phase: { phase: "processing" } });
    backend.destroy();
  });

  it("regression: toggle mode starts on press, ignores release and stops on the next press", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      settings: { activation: "toggle" },
    });
    await edge(backend, true);
    await edge(backend, false);
    tick(MOCK_MIC_READY_MS + 500);
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "listening", locked: false });
    await edge(backend, true);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    await edge(backend, false);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    backend.destroy();
  });

  it("regression: hold_or_toggle stops on a release held past the threshold; a short press locks (listening.locked) and the next press stops; releases while locked are ignored", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      settings: { activation: "hold_or_toggle", hold_threshold_ms: 300 },
    });
    const events = collect(backend);
    // Long hold: stop.
    await edge(backend, true);
    tick(MOCK_MIC_READY_MS + 400);
    await edge(backend, false);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    // Short press: lock. The flag rides on every later listening push (ready, partials).
    await edge(backend, true);
    tick(120);
    await edge(backend, false);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "listening",
      ready: false,
      locked: true,
    });
    tick(MOCK_MIC_READY_MS);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "listening",
      ready: true,
      locked: true,
    });
    await edge(backend, false);
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "listening", locked: true });
    tick(2000);
    await edge(backend, true);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    // Exactly the threshold counts as a hold.
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_DICTATION_DWELL_MS);
    await edge(backend, true);
    tick(300);
    await edge(backend, false);
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    const locked = events.filter(
      (e) => e.type === "dictation" && e.phase.phase === "listening" && e.phase.locked,
    );
    expect(locked.length).toBeGreaterThanOrEqual(2);
    backend.destroy();
  });

  it("regression: CLI edges toggle on press and cancel on release whatever the mode; atMs is honoured", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("hotkey_edge", { pressed: true, source: "cli", atMs: T0 });
    expect(backend.peek().dictation.phase.phase).toBe("listening");
    tick(MOCK_MIC_READY_MS);
    await backend.invoke("hotkey_edge", { pressed: true, source: "cli", atMs: clock });
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    await backend.invoke("hotkey_edge", { pressed: false, source: "cli" });
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    // While processing a CLI press is ignored, like a hotkey press.
    tick(MOCK_DICTATION_DWELL_MS);
    await backend.invoke("hotkey_edge", { pressed: true, source: "cli" });
    await backend.invoke("dictation_stop");
    await backend.invoke("hotkey_edge", { pressed: true, source: "cli" });
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    backend.destroy();
  });

  it("regression: extra_recording_ms keeps the microphone open after the stop; a second stop closes it at once and a cancel in the window drops the take", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      settings: { extra_recording_ms: 500 },
    });
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(1000);
    await backend.invoke("dictation_stop");
    expect(backend.peek().dictation.phase.phase).toBe("listening");
    tick(499);
    expect(backend.peek().dictation.phase.phase).toBe("listening");
    tick(1);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "processing",
      stage: "transcribing",
    });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    // The tail counts: 1000 ms of speech plus the 500 ms window.
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "done", duration_ms: 1500 });
    tick(MOCK_DICTATION_DWELL_MS);
    // Second stop inside the window: close now.
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + 200);
    await backend.invoke("dictation_stop");
    tick(100);
    await backend.invoke("dictation_stop");
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "processing",
      started_at: clock,
    });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_DICTATION_DWELL_MS);
    // Cancel inside the window: nothing is recognised.
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    await backend.invoke("dictation_stop");
    await backend.invoke("dictation_cancel");
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    tick(600);
    expect(backend.peek().dictation.phase.phase).toBe("cancelled");
    expect(backend.peek().history_recent).toHaveLength(2);
    backend.destroy();
  });
});

describe("MockBackend output modes (docs/dictation.md §12)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const installed = {
    [MOCK_STREAMING_MODEL_ID]: {
      kind: "installed" as const,
      path: `${MOCK_MODELS_ROOT}/${MOCK_STREAMING_MODEL_ID}`,
      installed_at: 1_758_600_000,
    },
  };
  const last = MOCK_LIVE_SCRIPT.at(-1);
  if (!last) throw new Error("script");
  const preview = livePreviewText(last);

  it("regression: effective_output_mode is the requested mode once the streaming model is installed and whole_take until then; it follows the install state and the live_preview switch", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      settings: { engines: { ...defaultEngineSettings(), output_mode: "streaming_final" } },
    });
    expect(backend.peek().engines).toMatchObject({
      live_preview_ready: false,
      effective_output_mode: "whole_take",
    });
    const events = collect(backend);
    await backend.invoke("model_download", { id: MOCK_STREAMING_MODEL_ID });
    tick(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
    expect(backend.peek().engines).toMatchObject({
      live_preview_ready: true,
      effective_output_mode: "streaming_final",
    });
    expect(events.some((e) => e.type === "engines")).toBe(true);
    await backend.invoke("settings_set_engines", {
      engines: { ...backend.peek().settings.engines, live_preview: false },
    });
    expect(backend.peek().engines.effective_output_mode).toBe("whole_take");
    await backend.invoke("settings_set_engines", {
      engines: {
        ...backend.peek().settings.engines,
        live_preview: true,
        output_mode: "live_inject",
      },
    });
    expect(backend.peek().engines.effective_output_mode).toBe("live_inject");
    await backend.invoke("model_remove", { id: MOCK_STREAMING_MODEL_ID });
    expect(backend.peek().engines.effective_output_mode).toBe("whole_take");
    // whole_take never depends on the model.
    await backend.invoke("settings_set_engines", {
      engines: { ...backend.peek().settings.engines, output_mode: "whole_take" },
    });
    expect(backend.peek().engines.effective_output_mode).toBe("whole_take");
    // A test may pin the resolved value directly.
    const pinned = new MockBackend({ engines: { effective_output_mode: "live_inject" } });
    expect(pinned.peek().engines.effective_output_mode).toBe("live_inject");
    backend.destroy();
  });

  it("regression: streaming_final goes finalizing then refining then done with the streaming text, the sentences and the short asr_ms; the history row carries the same mode and segments", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      models: installed,
      settings: { engines: { ...defaultEngineSettings(), output_mode: "streaming_final" } },
    });
    expect(backend.peek().engines.effective_output_mode).toBe("streaming_final");
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(MOCK_LIVE_STEP_MS * MOCK_LIVE_SCRIPT.length);
    const listening = backend.peek().dictation.phase;
    expect(listening.phase === "listening" && listening.live?.injected).toBe(0);
    await backend.invoke("dictation_stop");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "processing",
      stage: "finalizing",
      started_at: clock,
      stage_started_at: clock,
      preview,
    });
    tick(MOCK_FINALIZE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({ stage: "refining", preview });
    tick(MOCK_REFINE_MS);
    const done = backend.peek().dictation.phase;
    const committed = last.committed[0];
    if (!committed) throw new Error("script");
    const duration = MOCK_LIVE_STEP_MS * MOCK_LIVE_SCRIPT.length;
    expect(done).toEqual({
      phase: "done",
      text: MOCK_DICTATION_TEXT,
      raw_text: preview,
      chars: Array.from(MOCK_DICTATION_TEXT).length,
      via: "paste",
      refined: true,
      duration_ms: duration,
      asr_ms: MOCK_FINALIZE_MS,
      refine_ms: MOCK_REFINE_MS,
      mode: "streaming_final",
      segments: [committed, { text: last.current, start_ms: committed.end_ms, end_ms: duration }],
    });
    expect(backend.peek().history_recent[0]).toMatchObject({
      mode: "streaming_final",
      raw_text: preview,
      refined: true,
      segments: [committed, { text: last.current, start_ms: committed.end_ms, end_ms: duration }],
    });
    expect(backend.peek().history_recent[0]?.live_error).toBeUndefined();
    backend.destroy();
  });

  it("regression: live_inject counts pasted sentences in live.injected, goes finalizing then inserting then done unrefined even with refine on, and a cancel reports injected_chars", async () => {
    const backend = new MockBackend({
      now: () => clock,
      history: [],
      models: installed,
      settings: { engines: { ...defaultEngineSettings(), output_mode: "live_inject" } },
    });
    expect(backend.peek().engines.refine_enabled).toBe(true);
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * 3);
    let phase = backend.peek().dictation.phase;
    expect(phase.phase === "listening" && phase.live?.injected).toBe(0);
    tick(MOCK_LIVE_STEP_MS);
    phase = backend.peek().dictation.phase;
    expect(phase.phase === "listening" && phase.live?.committed).toHaveLength(1);
    expect(phase.phase === "listening" && phase.live?.injected).toBe(1);
    tick(MOCK_LIVE_STEP_MS * 2);
    await backend.invoke("dictation_stop");
    expect(backend.peek().dictation.phase).toMatchObject({ stage: "finalizing", preview });
    tick(MOCK_FINALIZE_MS);
    expect(backend.peek().dictation.phase).toEqual({
      phase: "processing",
      stage: "inserting",
      started_at: clock - MOCK_FINALIZE_MS,
      stage_started_at: clock,
    });
    tick(MOCK_FINALIZE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "done",
      mode: "live_inject",
      refined: false,
      text: preview,
      raw_text: preview,
      asr_ms: MOCK_FINALIZE_MS,
    });
    expect(backend.peek().dictation.phase).not.toHaveProperty("refine_ms");
    expect(backend.peek().history_recent[0]).toMatchObject({ mode: "live_inject", refined: false });
    expect(backend.peek().history_recent[0]?.refine_model).toBeUndefined();
    tick(MOCK_DICTATION_DWELL_MS);
    // Cancel after the first paste: the pasted characters stay and are reported.
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * 4);
    await backend.invoke("dictation_cancel");
    const committed = last.committed[0];
    if (!committed) throw new Error("script");
    expect(backend.peek().dictation.phase).toEqual({
      phase: "cancelled",
      injected_chars: Array.from(committed.text).length,
    });
    tick(MOCK_DICTATION_DWELL_MS);
    // Cancel before any endpoint: nothing was pasted.
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS);
    await backend.invoke("dictation_cancel");
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    backend.destroy();
  });

  it("regression: a streaming mode without the model runs as a whole take (no finalizing, no live_error); with the model but no text it degrades with live_error; a degraded preview degrades with its reason", async () => {
    const missing = new MockBackend({
      now: () => clock,
      history: [],
      settings: { engines: { ...defaultEngineSettings(), output_mode: "streaming_final" } },
    });
    await missing.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + 500);
    await missing.invoke("dictation_stop");
    expect(missing.peek().dictation.phase).toMatchObject({ stage: "transcribing" });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(missing.peek().dictation.phase).toMatchObject({ phase: "done", mode: "whole_take" });
    expect(missing.peek().dictation.phase).not.toHaveProperty("live_error");
    expect(missing.peek().dictation.phase).not.toHaveProperty("segments");
    missing.destroy();

    const empty = new MockBackend({
      now: () => clock,
      history: [],
      models: installed,
      settings: {
        engines: { ...defaultEngineSettings(), output_mode: "live_inject", refine_enabled: false },
      },
    });
    await empty.invoke("dictation_start");
    // Stopped before any partial: the streaming final is empty → whole take with the reason.
    tick(MOCK_MIC_READY_MS);
    await empty.invoke("dictation_stop");
    expect(empty.peek().dictation.phase).toMatchObject({ stage: "transcribing" });
    tick(MOCK_ASR_MS);
    expect(empty.peek().dictation.phase).toMatchObject({
      phase: "done",
      mode: "whole_take",
      live_error: MOCK_EMPTY_STREAM_ERROR,
      text: MOCK_DICTATION_RAW,
    });
    expect(empty.peek().history_recent[0]).toMatchObject({
      mode: "whole_take",
      live_error: MOCK_EMPTY_STREAM_ERROR,
    });
    empty.destroy();

    const degraded = new MockBackend({
      now: () => clock,
      history: [],
      models: installed,
      settings: { engines: { ...defaultEngineSettings(), output_mode: "streaming_final" } },
    });
    await degraded.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * 2);
    degraded.simulateLiveDegraded("live tap overrun");
    await degraded.invoke("dictation_stop");
    expect(degraded.peek().dictation.phase).toMatchObject({ stage: "transcribing" });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(degraded.peek().dictation.phase).toMatchObject({
      phase: "done",
      mode: "whole_take",
      live_error: "live tap overrun",
      text: MOCK_DICTATION_TEXT,
    });
    degraded.destroy();
  });
});

describe("MockBackend personal dictionary and replacement rules (docs/dictation.md section 16)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const HISTORY_ID = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
  const draft = (term: string, heard: string[], enabled = true): DictionaryDraft => ({
    term,
    heard_as: heard,
    enabled,
  });
  const rule = (
    name: string,
    pattern: string,
    replacement: string,
    extra: Partial<RuleDraft> = {},
  ): RuleDraft => ({
    name,
    kind: "literal",
    pattern,
    replacement,
    case_sensitive: true,
    enabled: true,
    ...extra,
  });
  const errors = (events: UiEvent[]) =>
    events.flatMap((e) => (e.type === "error" ? [e.message] : []));

  it("regression: dictionary commands behave like the core: drafts wrong on their own reject the call and list clashes are error events with the list kept", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    expect(backend.peek().dictionary).toEqual([]);
    const events = collect(backend);
    await backend.invoke("dictionary_add", { entry: draft(" Voltip ", ["沃提普", ""]) });
    tick(1000);
    await backend.invoke("dictionary_add", {
      entry: draft("good idea", ["谷歌IDR"]),
      historyId: HISTORY_ID,
    });
    const [voltip, good] = backend.peek().dictionary;
    if (!voltip || !good) throw new Error("two entries");
    expect(voltip).toMatchObject({
      term: "Voltip",
      heard_as: ["沃提普"],
      enabled: true,
      source: { kind: "manual" },
      created_at_ms: T0,
      updated_at_ms: T0,
    });
    expect(good.source).toEqual({ kind: "history", history_id: HISTORY_ID });
    expect(events.map((e) => e.type)).toEqual(["dictionary", "dictionary"]);
    // Wrong on its own: the call rejects and nothing is emitted.
    await expect(backend.invoke("dictionary_add", { entry: draft(" ", []) })).rejects.toThrow(
      "dictionary: 正确写法不能为空",
    );
    await expect(
      backend.invoke("dictionary_add", { entry: draft("x", []), historyId: "not-a-uuid" }),
    ).rejects.toThrow("id must be a UUID");
    await expect(backend.invoke("dictionary_remove", { id: "nope" })).rejects.toThrow(
      "id must be a UUID",
    );
    expect(events).toHaveLength(2);
    // A clash with the list: an error event, the list unchanged.
    await backend.invoke("dictionary_add", { entry: draft("voltip", []) });
    await backend.invoke("dictionary_add", { entry: draft("World", ["谷歌IDR"]) });
    expect(errors(events)).toEqual([
      "dictionary: 词典里已有「Voltip」",
      "dictionary: 「谷歌IDR」已是「good idea」的误识别写法",
    ]);
    expect(backend.peek().dictionary).toHaveLength(2);
    // Update keeps the id, the source and the creation time.
    tick(500);
    await backend.invoke("dictionary_update", {
      id: voltip.id,
      entry: draft("Voltip", ["沃提普", "volt ip"], false),
    });
    expect(backend.peek().dictionary[0]).toMatchObject({
      id: voltip.id,
      heard_as: ["沃提普", "volt ip"],
      enabled: false,
      created_at_ms: T0,
      updated_at_ms: T0 + 1500,
    });
    const unknown = "11111111-1111-4111-8111-111111111111";
    await backend.invoke("dictionary_update", { id: unknown, entry: draft("x", []) });
    await backend.invoke("dictionary_remove", { id: unknown });
    // Reorder takes exactly every id.
    await backend.invoke("dictionary_reorder", { ids: [good.id, voltip.id] });
    expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["good idea", "Voltip"]);
    await backend.invoke("dictionary_reorder", { ids: [good.id] });
    await backend.invoke("dictionary_reorder", { ids: [good.id, good.id] });
    await backend.invoke("dictionary_reorder", { ids: [good.id, unknown] });
    await backend.invoke("dictionary_remove", { id: good.id });
    expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip"]);
    expect(errors(events).slice(2)).toEqual([
      `dictionary: 没有 id 为 ${unknown} 的词条`,
      `dictionary: 没有 id 为 ${unknown} 的词条`,
      "dictionary: 新的顺序必须恰好包含现有的全部词条",
      "dictionary: 新的顺序必须恰好包含现有的全部词条",
      "dictionary: 新的顺序必须恰好包含现有的全部词条",
    ]);
    backend.destroy();
  });

  it("regression: rule commands and the TOML import merge or replace all or nothing and the export is the core format", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    await backend.invoke("rules_add", { rule: rule(" git push ", "给他push", "git push") });
    await backend.invoke("rules_add", {
      rule: rule("PR 编号", "\\bpr (\\d+)", "PR #$1", { kind: "regex", case_sensitive: false }),
    });
    await expect(
      backend.invoke("rules_add", { rule: rule("bad", "(", "", { kind: "regex" }) }),
    ).rejects.toThrow(/^rules: 规则「bad」：正则无法编译/);
    await backend.invoke("rules_add", { rule: rule("git push", "x", "y") });
    expect(errors(events)).toEqual(["rules: 已有名为「git push」的规则"]);
    const [git, pr] = backend.peek().rules;
    if (!git || !pr) throw new Error("two rules");
    expect(git.name).toBe("git push");
    tick(10);
    await backend.invoke("rules_update", {
      id: git.id,
      rule: rule("git push", "给他push", "git push --force"),
    });
    expect(backend.peek().rules[0]).toMatchObject({
      id: git.id,
      replacement: "git push --force",
      created_at_ms: T0,
      updated_at_ms: T0 + 10,
    });
    const unknown = "11111111-1111-4111-8111-111111111111";
    await backend.invoke("rules_update", { id: unknown, rule: rule("z", "z", "") });
    await backend.invoke("rules_remove", { id: unknown });
    await backend.invoke("rules_reorder", { ids: [git.id] });
    await backend.invoke("rules_reorder", { ids: [pr.id, git.id] });
    expect(backend.peek().rules.map((r) => r.name)).toEqual(["PR 编号", "git push"]);
    expect(errors(events).slice(1)).toEqual([
      `rules: 没有 id 为 ${unknown} 的规则`,
      `rules: 没有 id 为 ${unknown} 的规则`,
      "rules: 新的顺序必须恰好包含现有的全部规则",
    ]);
    // Export → the core's TOML; merge updates same-name rules in place and appends the rest.
    const exported = await backend.rulesExport();
    expect(exported).toContain("pattern = '\\bpr (\\d+)'");
    await backend.invoke("rules_import", {
      toml: 'version = 1\n[[rule]]\nname = "git push"\npattern = "推"\nreplacement = "push"\n[[rule]]\nname = "new"\npattern = "n"\n',
      mode: "merge",
    });
    expect(backend.peek().rules.map((r) => [r.name, r.pattern])).toEqual([
      ["PR 编号", "\\bpr (\\d+)"],
      ["git push", "推"],
      ["new", "n"],
    ]);
    expect(backend.peek().rules[1]?.id).toBe(git.id);
    // A bad file rejects the call; the list stays as it was.
    await expect(
      backend.invoke("rules_import", { toml: "version = 2\n", mode: "replace" }),
    ).rejects.toThrow("rules: 不支持的 version = 2（应为 1）");
    expect(backend.peek().rules).toHaveLength(3);
    // Replace: the file is the whole list (the exported text round-trips).
    await backend.invoke("rules_import", { toml: exported, mode: "replace" });
    expect(backend.peek().rules.map((r) => r.name)).toEqual(["PR 编号", "git push"]);
    expect(backend.peek().rules[1]?.replacement).toBe("git push --force");
    // A merge that would pass the cap is an error event, nothing is added.
    const full = Array.from({ length: 199 }, (_, i) => `[[rule]]\nname = "r${i}"\npattern = "p"`);
    await backend.invoke("rules_import", {
      toml: ["version = 1", ...full].join("\n"),
      mode: "merge",
    });
    expect(errors(events).at(-1)).toBe("rules: 规则最多 200 条");
    expect(backend.peek().rules).toHaveLength(2);
    await backend.invoke("rules_remove", { id: backend.peek().rules[0]?.id ?? "" });
    expect(backend.peek().rules).toHaveLength(1);
    backend.destroy();
  });

  it("regression: a mock take corrects the transcript with the dictionary and runs the rules last and records what fired and an emptied text is no speech", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("dictionary_add", { entry: draft("Helper", ["helper"]) });
    await backend.invoke("rules_add", { rule: rule("代码", "逻辑", "代码") });
    await backend.invoke("rules_add", { rule: rule("off", "复用", "x", { enabled: false }) });
    const [helper] = backend.peek().dictionary;
    const [code] = backend.peek().rules;
    const run = async () => {
      await backend.invoke("dictation_start");
      tick(MOCK_MIC_READY_MS);
      tick(1000);
      await backend.invoke("dictation_stop");
      tick(MOCK_ASR_MS);
      tick(MOCK_REFINE_MS);
    };
    await run();
    const done = backend.peek().dictation.phase;
    expect(done.phase === "done" && [done.raw_text, done.text]).toEqual([
      MOCK_DICTATION_RAW,
      "把这段代码抽成一个 Helper，然后在 session_assembly 里复用。",
    ]);
    expect(backend.peek().history_recent[0]?.vocabulary).toEqual({
      corrections: [{ id: helper?.id, count: 1 }],
      rules: [{ id: code?.id, count: 1 }],
    });
    tick(MOCK_DICTATION_DWELL_MS);
    // Refine off: the corrected transcript itself goes through the rules.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), refine_enabled: false },
    });
    await run();
    const raw = backend.peek().dictation.phase;
    expect(raw.phase === "done" && raw.text).toBe(
      "把这段代码抽成一个 Helper 然后在 session assembly 里复用",
    );
    tick(MOCK_DICTATION_DWELL_MS);
    // A rule that deletes everything: nothing to insert, no history row.
    await backend.invoke("rules_add", { rule: rule("all", ".+", "", { kind: "regex" }) });
    const rows = backend.peek().history_recent.length;
    await run();
    expect(backend.peek().dictation.phase).toEqual({
      phase: "failed",
      message: MOCK_NO_SPEECH,
      code: "no_speech",
    });
    expect(backend.peek().history_recent).toHaveLength(rows);
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.phase).toEqual({ phase: "idle" });
    // Nothing fired: the row has no vocabulary key.
    const plain = new MockBackend({ now: () => clock, history: [] });
    await plain.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS + 500);
    await plain.invoke("dictation_stop");
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    const row = plain.peek().history_recent[0];
    expect(row?.text).toBe(MOCK_DICTATION_TEXT);
    expect(Object.hasOwn(row ?? {}, "vocabulary")).toBe(false);
    plain.destroy();
    backend.destroy();
  });

  it("regression: vocabularyPreview answers with the core semantics and seeded lists are kept and the phone refuses every vocabulary command", async () => {
    const seeded = new MockBackend({ now: () => clock, history: [] });
    await seeded.invoke("dictionary_add", { entry: draft("good idea", ["谷歌IDR"]) });
    const dictionary = seeded.peek().dictionary;
    const backend = new MockBackend({ now: () => clock, history: [], dictionary });
    expect(backend.peek().dictionary).toEqual(dictionary);
    const preview = await backend.vocabularyPreview("一个谷歌IDR的app", {
      rule: rule("app", "app", "App"),
    });
    expect(preview).toEqual({
      corrected: "一个good idea的app",
      output: "一个good idea的App",
      corrections: [{ id: dictionary[0]?.id, count: 1 }],
      rules: [{ id: NIL_ID, count: 1 }],
    });
    await expect(
      backend.vocabularyPreview("x", { rule: rule("r", "(", "", { kind: "regex" }) }),
    ).rejects.toThrow(/正则无法编译/);
    const phone = new MockBackend({ role: "phone", dictionary, rules: [] });
    expect(phone.peek().dictionary).toEqual([]);
    await expect(phone.invoke("dictionary_add", { entry: draft("a", []) })).rejects.toThrow(
      VOCABULARY_UNAVAILABLE,
    );
    await expect(
      phone.invoke("rules_import", { toml: "version = 1", mode: "merge" }),
    ).rejects.toThrow(VOCABULARY_UNAVAILABLE);
    await expect(phone.vocabularyPreview("x")).rejects.toThrow(VOCABULARY_UNAVAILABLE);
    await expect(phone.rulesExport()).rejects.toThrow(VOCABULARY_UNAVAILABLE);
    seeded.destroy();
    backend.destroy();
    phone.destroy();
  });
});

describe("MockBackend scenes and context (docs/dictation.md section 18)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const scene = (
    name: string,
    apps: string[],
    overrides: SceneDraft["overrides"] = {},
  ): SceneDraft => ({
    name,
    enabled: true,
    match: { apps, title_contains: [] },
    overrides,
  });
  const errors = (events: UiEvent[]) =>
    events.flatMap((e) => (e.type === "error" ? [e.message] : []));
  const take = async (backend: MockBackend) => {
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(1000);
    await backend.invoke("dictation_stop");
    tick(MOCK_ASR_MS);
    tick(MOCK_REFINE_MS);
  };

  it("regression: scene commands behave like the core: drafts wrong on their own reject the call and list clashes are error events with the list kept", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    // The desktop's list always holds the built-in scenes (§18.10); the user's come before them.
    const mine = () => backend.peek().scenes.filter((s) => s.builtin === undefined);
    const builtinIds = () =>
      backend.peek().scenes.flatMap((s) => (s.builtin === undefined ? [] : [s.id]));
    expect(mine()).toEqual([]);
    const events = collect(backend);
    await backend.invoke("scenes_add", { scene: scene(" 聊天 ", ["Slack.exe", "slack"]) });
    tick(1000);
    await backend.invoke("scenes_add", {
      scene: scene("代码", ["code"], { refine_enabled: false }),
    });
    const [chat, code] = mine();
    if (!chat || !code) throw new Error("two scenes");
    expect(chat).toMatchObject({ name: "聊天", match: { apps: ["slack"] }, created_at_ms: T0 });
    expect(code.overrides).toEqual({ refine_enabled: false });
    await expect(backend.invoke("scenes_add", { scene: scene("x", []) })).rejects.toThrow(
      "scenes: 场景「x」至少要有一个应用",
    );
    await expect(backend.invoke("scenes_remove", { id: "nope" })).rejects.toThrow(
      "id must be a UUID",
    );
    await backend.invoke("scenes_add", { scene: scene("聊天", ["wechat"]) });
    expect(errors(events)).toEqual(["scenes: 已有名为「聊天」的场景"]);
    tick(10);
    await backend.invoke("scenes_update", {
      id: chat.id,
      scene: { ...scene("聊天", ["slack"]), enabled: false },
    });
    expect(mine()[0]).toMatchObject({
      id: chat.id,
      enabled: false,
      created_at_ms: T0,
      updated_at_ms: T0 + 1010,
    });
    const unknown = "11111111-1111-4111-8111-111111111111";
    await backend.invoke("scenes_update", { id: unknown, scene: scene("z", ["z"]) });
    await backend.invoke("scenes_remove", { id: unknown });
    await backend.invoke("scenes_reorder", { ids: [code.id] });
    await backend.invoke("scenes_reorder", { ids: [code.id, chat.id, ...builtinIds()] });
    expect(mine().map((s) => s.name)).toEqual(["代码", "聊天"]);
    expect(errors(events).slice(1)).toEqual([
      `scenes: 没有 id 为 ${unknown} 的场景`,
      `scenes: 没有 id 为 ${unknown} 的场景`,
      "scenes: 新的顺序必须恰好包含现有的全部场景",
    ]);
    await backend.invoke("scenes_remove", { id: code.id });
    expect(mine().map((s) => s.name)).toEqual(["聊天"]);
    await backend.invoke("settings_set_context_sharing", { appName: false, windowTitle: true });
    expect(backend.peek().settings.context_sharing).toEqual({
      app_name: false,
      window_title: true,
    });
    backend.destroy();
  });

  it("regression: the fake probe picks the scene of a take: its refine switch and output mode apply to that take only, the status and the history carry the context, recent apps follow the history", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("scenes_add", {
      scene: scene("代码", ["code"], { refine_enabled: false }),
    });
    await backend.invoke("scenes_add", {
      scene: scene("流式", ["slack"], { output_mode: "streaming_final" }),
    });
    const [code] = backend.peek().scenes;
    // No answer: no context, no scene, the globals refine.
    await take(backend);
    let done = backend.peek().dictation;
    expect(done.context).toBeUndefined();
    expect(done.phase.phase === "done" && done.phase.refined).toBe(true);
    expect(backend.peek().history_recent[0]?.app).toBeUndefined();
    tick(MOCK_DICTATION_DWELL_MS);
    // In Code: the scene switches refining off; the listening status already names it.
    // The probe's id is kept normalised, like the core's `ForegroundApp::sanitized`.
    backend.setForegroundApp({ id: "Code.exe", name: " Code\n", title: "main.rs" });
    await backend.invoke("dictation_start");
    const listening = backend.peek().dictation;
    expect(listening.phase.phase).toBe("listening");
    expect(listening.context).toEqual({
      app: { id: "code", name: "Code" },
      scene: { id: code?.id, name: "代码" },
    });
    tick(MOCK_MIC_READY_MS + 1000);
    await backend.invoke("dictation_stop");
    tick(MOCK_ASR_MS);
    done = backend.peek().dictation;
    expect(done.phase.phase === "done" && [done.phase.refined, done.phase.text]).toEqual([
      false,
      MOCK_DICTATION_RAW,
    ]);
    expect(done.context?.scene?.name).toBe("代码");
    expect(backend.peek().history_recent[0]).toMatchObject({
      app: { id: "code", name: "Code" },
      scene: { id: code?.id, name: "代码" },
      refined: false,
    });
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation).toEqual({
      session: 2,
      phase: { phase: "idle" },
      kind: "dictation",
    });
    // A scene's streaming mode without the streaming model: a whole take that says why.
    backend.setForegroundApp({ id: "slack", name: "Slack" });
    await take(backend);
    const streamed = backend.peek().dictation.phase;
    expect(streamed.phase === "done" && [streamed.mode, streamed.live_error]).toEqual([
      "whole_take",
      MOCK_SCENE_MODE_NOT_READY,
    ]);
    tick(MOCK_DICTATION_DWELL_MS);
    // An app no scene names: a context without a scene, the globals.
    backend.setForegroundApp({ id: "winword", name: "WINWORD" });
    await take(backend);
    const plain = backend.peek().dictation;
    expect(plain.context).toEqual({ app: { id: "winword", name: "WINWORD" } });
    expect(plain.phase.phase === "done" && plain.phase.refined).toBe(true);
    expect(await backend.recentApps()).toEqual([
      { id: "winword", name: "WINWORD" },
      { id: "slack", name: "Slack" },
      { id: "code", name: "Code" },
    ]);
    // An id that normalises to nothing is no answer at all: no context.
    tick(MOCK_DICTATION_DWELL_MS);
    backend.setForegroundApp({ id: " .EXE ", name: "?" });
    await take(backend);
    expect(backend.peek().dictation.context).toBeUndefined();
    backend.destroy();
  });

  it("the desktop's list holds the built-in scenes like the core: off after the user's, not deleted or renamed, restored, with their term packs (section 18.10)", async () => {
    const mac = new MockBackend({
      now: () => clock,
      history: [],
      identity: { ...desktopIdentity(), platform: "macos" },
      scenes: [
        {
          id: "00000000-0000-4000-8000-000000000001",
          name: "聊天",
          enabled: true,
          match: { apps: ["slack"], title_contains: [] },
          overrides: {},
          created_at_ms: 1,
          updated_at_ms: 1,
        },
      ],
    });
    const scenes = mac.peek().scenes;
    expect(scenes.map((s) => s.builtin ?? s.name)).toEqual(["聊天", ...BUILTIN_SCENES]);
    expect(scenes.slice(1).every((s) => !s.enabled && s.name === s.builtin)).toBe(true);
    expect(scenes[1]?.match.apps).toContain("com.microsoft.vscode");
    mac.destroy();

    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    const legal = backend.peek().scenes.find((s) => s.builtin === "legal");
    if (!legal) throw new Error("the 法律 scene");
    expect(legal.match.apps).toEqual([]);
    // A built-in scene may list no application; a scene of the user's may not.
    await backend.invoke("scenes_update", {
      id: legal.id,
      scene: {
        name: "legal",
        enabled: true,
        match: { apps: [], title_contains: [] },
        overrides: {},
      },
    });
    expect(backend.peek().scenes.find((s) => s.id === legal.id)).toMatchObject({
      enabled: true,
      overrides: {},
      builtin: "legal",
    });
    await backend.invoke("scenes_update", {
      id: legal.id,
      scene: {
        name: "法律",
        enabled: true,
        match: { apps: [], title_contains: [] },
        overrides: {},
      },
    });
    await backend.invoke("scenes_remove", { id: legal.id });
    await backend.invoke("scenes_add", { scene: scene("我的", ["code"]) });
    const mine = backend.peek().scenes[0];
    if (!mine) throw new Error("the user's scene");
    expect(mine.name).toBe("我的");
    await backend.invoke("scenes_update", { id: mine.id, scene: scene("我的", [" "]) });
    await backend.invoke("scenes_restore", { id: mine.id });
    expect(errors(events)).toEqual([
      "scenes: 内置场景不能改名",
      "scenes: 内置场景不能删除，可以关闭",
      "scenes: 场景「我的」至少要有一个应用",
      "scenes: 只有内置场景可以恢复默认",
    ]);
    // 恢复默认: the defaults back, the switch as it is.
    await backend.invoke("scenes_restore", { id: legal.id });
    const template = MOCK_BUILTIN_SCENES.find((r) => r.id === "legal")?.templates.windows;
    expect(backend.peek().scenes.find((s) => s.id === legal.id)).toMatchObject({
      enabled: true,
      match: template?.match,
      overrides: template?.overrides,
    });
    const packs = await backend.scenesBuiltin();
    expect(packs.map((p) => p.id)).toEqual([...BUILTIN_SCENES]);
    expect(packs.find((p) => p.id === "chat")?.terms).toEqual([]);
    expect(packs.find((p) => p.id === "coding")?.terms).toContain("Kubernetes");
    backend.destroy();
  });

  it("regression: the phone refuses every scene command and the query", async () => {
    const phone = new MockBackend({ role: "phone", scenes: [] });
    expect(phone.peek().scenes).toEqual([]);
    await expect(phone.invoke("scenes_add", { scene: scene("a", ["x"]) })).rejects.toThrow(
      SCENES_UNAVAILABLE,
    );
    await expect(
      phone.invoke("settings_set_context_sharing", { appName: true, windowTitle: false }),
    ).rejects.toThrow(SCENES_UNAVAILABLE);
    await expect(phone.recentApps()).rejects.toThrow(SCENES_UNAVAILABLE);
    await expect(phone.scenesBuiltin()).rejects.toThrow(SCENES_UNAVAILABLE);
    phone.destroy();
  });
});

describe("MockBackend presets (docs/dictation.md section 21)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const draft = (name: string, prompt = "整理成周报") => ({ name, prompt });
  const errors = (events: UiEvent[]) =>
    events.flatMap((e) => (e.type === "error" ? [e.message] : []));
  const take = async (backend: MockBackend) => {
    await backend.invoke("dictation_start");
    tick(MOCK_MIC_READY_MS);
    tick(1000);
    await backend.invoke("dictation_stop");
    tick(MOCK_ASR_MS);
    tick(MOCK_REFINE_MS);
  };
  const setPreset = (backend: MockBackend, refine_preset: string) =>
    backend.invoke("settings_set_engines", {
      engines: { ...backend.peek().settings.engines, refine_preset },
    });

  it("regression: preset commands behave like the core: drafts wrong on their own reject the call and list clashes are error events with the list kept", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    expect(backend.peek().presets).toEqual([]);
    const events = collect(backend);
    await backend.invoke("presets_add", { preset: draft(" 周报 ", " 整理成周报\r\n按项目分组 ") });
    const [weekly] = backend.peek().presets;
    if (!weekly) throw new Error("one preset");
    expect(weekly).toMatchObject({
      name: "周报",
      prompt: "整理成周报\n按项目分组",
      created_at_ms: T0,
      updated_at_ms: T0,
    });
    const refusals: [{ name: string; prompt: string }, string][] = [
      [draft(""), "presets: 预设名称不能为空"],
      [draft("a", "  "), "presets: 预设内容不能为空"],
      [draft("字".repeat(25)), "presets: 预设名称最多 24 个字符（当前 25）"],
      [draft("a", "x".repeat(4001)), "presets: 预设内容最多 4000 个字符（当前 4001）"],
      [draft("a\nb"), "presets: 预设名称不能包含换行或控制字符"],
      [draft("a", "x\u0007"), "presets: 预设内容不能包含控制字符"],
    ];
    for (const [preset, message] of refusals)
      await expect(backend.invoke("presets_add", { preset })).rejects.toThrow(message);
    await expect(backend.invoke("presets_remove", { id: "nope" })).rejects.toThrow(
      "id must be a UUID",
    );
    // Names are unique ignoring ASCII case, as the core compares them.
    await backend.invoke("presets_add", { preset: draft("Mail") });
    await backend.invoke("presets_add", { preset: draft("mail") });
    await backend.invoke("presets_add", { preset: draft("周报", "y") });
    expect(errors(events)).toEqual([
      "presets: 已有名为「Mail」的预设",
      "presets: 已有名为「周报」的预设",
    ]);
    tick(10);
    await backend.invoke("presets_update", { id: weekly.id, preset: draft("周报", "新的内容") });
    expect(backend.peek().presets[0]).toMatchObject({
      id: weekly.id,
      prompt: "新的内容",
      created_at_ms: T0,
      updated_at_ms: T0 + 10,
    });
    const unknown = "11111111-1111-4111-8111-111111111111";
    await backend.invoke("presets_update", { id: unknown, preset: draft("z") });
    await backend.invoke("presets_remove", { id: unknown });
    expect(errors(events).slice(2)).toEqual([
      `presets: 没有 id 为 ${unknown} 的预设`,
      `presets: 没有 id 为 ${unknown} 的预设`,
    ]);
    await backend.invoke("presets_remove", { id: weekly.id });
    expect(backend.peek().presets.map((p) => p.name)).toEqual(["Mail"]);
    for (let n = backend.peek().presets.length; n < MAX_PRESETS; n += 1)
      await backend.invoke("presets_add", { preset: draft(`预设 ${n}`) });
    await backend.invoke("presets_add", { preset: draft("再多一个") });
    expect(backend.peek().presets).toHaveLength(MAX_PRESETS);
    expect(errors(events).at(-1)).toBe(`presets: 自定义预设最多 ${MAX_PRESETS} 个`);
    backend.destroy();
  });

  it("regression: a take refines with the preset of its start: the status names it from processing to idle, the history records it, a custom preset that is gone is 校对, a scene's preset wins", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    await backend.invoke("presets_add", { preset: draft("周报") });
    const [weekly] = backend.peek().presets;
    if (!weekly) throw new Error("one preset");
    await setPreset(backend, weekly.id);
    await backend.invoke("dictation_start");
    expect(backend.peek().dictation.preset).toBeUndefined();
    tick(MOCK_MIC_READY_MS + 1000);
    // Switched mid-take: the next take refines with it, this one keeps the preset of its start.
    await setPreset(backend, "notes");
    await backend.invoke("dictation_stop");
    const named = { id: weekly.id, name: "周报" };
    expect(backend.peek().dictation.preset).toEqual(named);
    tick(MOCK_ASR_MS);
    expect(backend.peek().dictation.phase).toMatchObject({
      phase: "processing",
      stage: "refining",
    });
    tick(MOCK_REFINE_MS);
    expect(backend.peek().dictation.preset).toEqual(named);
    expect(backend.peek().history_recent[0]?.preset).toEqual(named);
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation.preset).toBeUndefined();
    await take(backend);
    expect(backend.peek().history_recent[0]?.preset).toEqual({ id: "notes", name: "要点纪要" });
    tick(MOCK_DICTATION_DWELL_MS);
    // A custom preset that is gone refines with 校对, and the history says so.
    await setPreset(backend, weekly.id);
    await backend.invoke("presets_remove", { id: weekly.id });
    await take(backend);
    expect(backend.peek().history_recent[0]?.preset).toEqual({ id: "proofread", name: "校对" });
    tick(MOCK_DICTATION_DWELL_MS);
    // A scene's preset wins over the engines'; a take the scene does not refine names none.
    await backend.invoke("scenes_add", {
      scene: {
        name: "代码",
        enabled: true,
        match: { apps: ["code"], title_contains: [] },
        overrides: { refine_preset: "formal" },
      },
    });
    await backend.invoke("scenes_add", {
      scene: {
        name: "聊天",
        enabled: true,
        match: { apps: ["slack"], title_contains: [] },
        overrides: { refine_enabled: false, refine_preset: "chat" },
      },
    });
    backend.setForegroundApp({ id: "code", name: "Code" });
    await take(backend);
    expect(backend.peek().history_recent[0]?.preset).toEqual({ id: "formal", name: "书面语" });
    tick(MOCK_DICTATION_DWELL_MS);
    backend.setForegroundApp({ id: "slack", name: "Slack" });
    await take(backend);
    expect(backend.peek().history_recent[0]).toMatchObject({ refined: false });
    expect(backend.peek().history_recent[0]?.preset).toBeUndefined();
    expect(backend.peek().dictation.preset).toBeUndefined();
    backend.destroy();
  });

  it("试一试 answers by id with the canned clean-up, refuses arguments the bridge refuses, and fails at once without a clean-up", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    const answers = () => events.flatMap((e) => (e.type === "preset_try" ? [e] : []));
    const sample = MOCK_PRESET_SAMPLES.proofread;
    if (!sample) throw new Error("the 校对 example");
    await backend.invoke("presets_try", {
      id: 1,
      preset: "proofread",
      prompt: null,
      text: ` ${sample.input} `,
    });
    expect(answers()).toEqual([]);
    tick(MOCK_REFINE_MS);
    expect(answers()).toEqual([
      {
        type: "preset_try",
        id: 1,
        outcome: {
          status: "ok",
          text: sample.output,
          latency_ms: MOCK_REFINE_MS,
          model: MOCK_ENGINE_BUILTIN.refine_model,
        },
      },
    ]);
    await backend.invoke("presets_try", {
      id: 2,
      preset: null,
      prompt: "整理一下",
      text: "嗯今天下午开会",
    });
    tick(MOCK_REFINE_MS);
    expect(answers()[1]).toMatchObject({
      id: 2,
      outcome: { status: "ok", text: "今天下午开会。" },
    });
    expect(backend.peek().history_recent).toEqual([]);
    const refusals: [PresetsTryArgs, string][] = [
      [
        { id: 3, preset: null, prompt: null, text: "x" },
        "presets: 试一试需要一个预设或一段预设内容",
      ],
      [
        { id: 3, preset: "notes", prompt: "x", text: "x" },
        "presets: 试一试需要一个预设或一段预设内容",
      ],
      [{ id: 3, preset: "casual", prompt: null, text: "x" }, "presets: 没有名为「casual」的预设"],
      [{ id: 3, preset: null, prompt: " ", text: "x" }, "presets: 预设内容不能为空"],
      [{ id: 3, preset: "notes", prompt: null, text: "  " }, "presets: 请输入要试运行的文字"],
      [
        { id: 3, preset: "notes", prompt: null, text: "字".repeat(2001) },
        "presets: 试运行的文字最多 2000 个字符（当前 2001）",
      ],
    ];
    for (const [args, message] of refusals)
      await expect(backend.invoke("presets_try", args)).rejects.toThrow(message);
    backend.destroy();
    const bare = new MockBackend({ now: () => clock, history: [], builtIn: {} });
    const bareEvents = collect(bare);
    await bare.invoke("presets_try", { id: 9, preset: "chat", prompt: null, text: "你好" });
    expect(bareEvents).toContainEqual({
      type: "preset_try",
      id: 9,
      outcome: { status: "failed", reason: PRESET_TRY_UNCONFIGURED },
    });
    bare.destroy();
  });

  it("presetsBuiltin answers the shell's texts in the interface's order, on the phone too", async () => {
    const backend = new MockBackend();
    const texts = await backend.presetsBuiltin();
    expect(texts.map((t) => t.id)).toEqual([...BUILTIN_PRESETS]);
    // The confirmed examples of docs/dictation.md §21 are the bodies' own.
    for (const [id, sample] of Object.entries(MOCK_PRESET_SAMPLES)) {
      const body = texts.find((t) => t.id === id)?.prompt ?? "";
      expect(body).toContain(`输入：${sample.input}\n输出：${sample.output}`);
    }
    backend.destroy();
    // User decision 2026-10-01: the phone has its own presets (they were refused before), and it
    // opens the project's pages in its browser.
    const phone = new MockBackend({ role: "phone", presets: [] });
    expect(phone.peek().presets).toEqual([]);
    await phone.invoke("presets_add", { preset: draft("a") });
    expect(phone.peek().presets.map((p) => p.name)).toEqual(["a"]);
    expect((await phone.presetsBuiltin()).map((t) => t.id)).toEqual([...BUILTIN_PRESETS]);
    await phone.projectLinkOpen("source");
    expect(phone.linksOpened).toEqual(["source"]);
    phone.destroy();
  });
});

describe("MockBackend voice edit (section 19)", () => {
  const T0 = 1_758_700_000_000;
  let clock = T0;
  beforeEach(() => {
    clock = T0;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const tick = (ms: number) => {
    clock += ms;
    vi.advanceTimersByTime(ms);
  };
  const editEdge = (backend: MockBackend, pressed: boolean, source: "hotkey" | "cli" = "hotkey") =>
    backend.invoke("hotkey_edge", { pressed, source, purpose: "edit" });
  const SELECTION = "大家好，会议改到周四十点哈";
  const phases = (events: UiEvent[]) =>
    events.flatMap((e) =>
      e.type === "dictation"
        ? [`${e.kind}:${e.phase.phase}${e.phase.phase === "processing" ? `/${e.phase.stage}` : ""}`]
        : [],
    );

  it("regression: the edit key copies the selection and rewrites it by the instruction and records an edit row; the rules never run on the rewrite and the output mode does not apply", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    backend.setSelection(SELECTION);
    // A rule that would change the rewrite (各位 → 诸位) and a streaming output mode: neither applies.
    await backend.invoke("rules_add", {
      rule: {
        name: "各位",
        kind: "literal",
        pattern: "各位",
        replacement: "诸位",
        case_sensitive: true,
        enabled: true,
      },
    });
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), output_mode: "streaming_final" },
    });
    await editEdge(backend, true);
    expect(backend.peek().dictation).toMatchObject({
      session: 1,
      kind: "edit",
      phase: { phase: "listening", ready: false },
    });
    tick(MOCK_COPY_MS);
    tick(MOCK_MIC_READY_MS);
    tick(1000);
    await editEdge(backend, false);
    tick(MOCK_ASR_MS);
    tick(MOCK_REFINE_MS);
    tick(MOCK_FINALIZE_MS);
    expect(phases(events)).toEqual([
      "edit:listening",
      "edit:listening",
      "edit:processing/transcribing",
      "edit:processing/refining",
      "edit:processing/inserting",
      "edit:done",
    ]);
    const done = backend.peek().dictation.phase;
    expect(done).toMatchObject({
      phase: "done",
      text: MOCK_EDIT_TEXT,
      raw_text: MOCK_EDIT_INSTRUCTION,
      chars: Array.from(MOCK_EDIT_TEXT).length,
      via: "paste",
      refined: true,
      mode: "whole_take",
    });
    const [row] = backend.peek().history_recent;
    expect(row).toMatchObject({
      kind: "edit",
      edit: { instruction: MOCK_EDIT_INSTRUCTION, selection: SELECTION },
      text: MOCK_EDIT_TEXT,
      raw_text: MOCK_EDIT_INSTRUCTION,
      refined: true,
      mode: "whole_take",
    });
    expect(row?.vocabulary).toBeUndefined();
    // The status keeps the last take's kind through the dwell.
    tick(MOCK_DICTATION_DWELL_MS);
    expect(backend.peek().dictation).toEqual({
      session: 1,
      phase: { phase: "idle" },
      kind: "edit",
    });
    // The next dictation is a dictation again.
    await backend.invoke("dictation_start");
    expect(backend.peek().dictation.kind).toBe("dictation");
    backend.destroy();
  });

  it("regression: nothing selected or a selection over the limit or a missing refine key is refused like the core before anything is recorded", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    // Nothing selected (the default): refused right after the copy, while still listening.
    await editEdge(backend, true);
    expect(backend.peek().dictation.phase.phase).toBe("listening");
    tick(MOCK_COPY_MS);
    expect(backend.peek().dictation).toMatchObject({
      kind: "edit",
      phase: { phase: "failed", code: "no_selection", message: MOCK_NO_SELECTION },
    });
    // The release that follows does nothing.
    await editEdge(backend, false);
    expect(backend.peek().dictation.phase.phase).toBe("failed");
    backend.setSelection("   ");
    await editEdge(backend, true);
    tick(MOCK_COPY_MS);
    expect(backend.peek().dictation.phase).toMatchObject({ code: "no_selection" });
    await editEdge(backend, false);
    backend.setSelection("字".repeat(MAX_EDIT_SELECTION_CHARS + 1));
    await editEdge(backend, true);
    tick(MOCK_COPY_MS);
    const long = backend.peek().dictation.phase;
    expect(long).toMatchObject({ phase: "failed", code: "selection_too_long" });
    expect(long.phase === "failed" && long.message).toContain(String(MAX_EDIT_SELECTION_CHARS + 1));
    await editEdge(backend, false);
    // Exactly at the limit is fine.
    backend.setSelection("字".repeat(MAX_EDIT_SELECTION_CHARS));
    await editEdge(backend, true);
    tick(MOCK_COPY_MS);
    expect(backend.peek().dictation.phase.phase).toBe("listening");
    await backend.invoke("dictation_cancel");
    expect(backend.peek().dictation.phase.phase).toBe("cancelled");
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_FINALIZE_MS);
    expect(backend.peek().history_recent).toEqual([]);
    backend.destroy();
    // No clean-up provider ready: refused at the press, the microphone never opens.
    const keyless = new MockBackend({
      now: () => clock,
      history: [],
      settings: { engines: { ...defaultEngineSettings(), llm_provider: "groq" } },
    });
    const events = collect(keyless);
    keyless.setSelection(SELECTION);
    await editEdge(keyless, true);
    expect(phases(events)).toEqual(["edit:failed"]);
    expect(keyless.peek().dictation.phase).toEqual({
      phase: "failed",
      message: MOCK_EDIT_UNAVAILABLE,
      code: "edit_unavailable",
    });
    keyless.destroy();
  });

  it("regression: the edit key with a terminal in front is refused before the copy and the microphone and nothing is recorded on Windows and Linux but not on macOS", async () => {
    for (const [platform, id] of [
      ["windows", "WindowsTerminal.exe"],
      ["linux", "gnome-terminal-server"],
    ] as const) {
      const backend = new MockBackend({
        now: () => clock,
        history: [],
        identity: { ...desktopIdentity(), platform },
        foregroundApp: { id, name: "Terminal" },
      });
      const events = collect(backend);
      backend.setSelection(SELECTION);
      await editEdge(backend, true);
      expect(phases(events)).toEqual(["edit:listening", "edit:failed"]);
      expect(backend.peek().dictation.phase).toEqual({
        phase: "failed",
        message: MOCK_EDIT_IN_TERMINAL,
        code: "edit_in_terminal",
      });
      await editEdge(backend, false);
      tick(MOCK_COPY_MS + MOCK_MIC_READY_MS + MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_FINALIZE_MS);
      expect(phases(events)).toEqual(["edit:listening", "edit:failed"]);
      expect(backend.peek().history_recent).toEqual([]);
      backend.destroy();
    }
    // macOS copies with Cmd+C: Terminal is no hazard there, and an editor anywhere goes through.
    for (const [platform, id] of [
      ["macos", "com.apple.Terminal"],
      ["windows", "Code.exe"],
    ] as const) {
      const backend = new MockBackend({
        now: () => clock,
        history: [],
        identity: { ...desktopIdentity(), platform },
        foregroundApp: { id, name: "App" },
      });
      backend.setSelection(SELECTION);
      await editEdge(backend, true);
      tick(MOCK_COPY_MS + MOCK_MIC_READY_MS);
      expect(backend.peek().dictation.phase.phase).toBe("listening");
      backend.destroy();
    }
  });

  it("regression: one take at a time: the edges of the other key are dropped while a take runs; a CLI edit edge toggles", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    backend.setSelection(SELECTION);
    await backend.invoke("hotkey_edge", { pressed: true, source: "hotkey" });
    tick(MOCK_MIC_READY_MS);
    await editEdge(backend, true);
    await editEdge(backend, false);
    expect(backend.peek().dictation).toMatchObject({
      session: 1,
      kind: "dictation",
      phase: { phase: "listening" },
    });
    await backend.invoke("hotkey_edge", { pressed: false, source: "hotkey" });
    expect(backend.peek().dictation.phase.phase).toBe("processing");
    await editEdge(backend, true);
    expect(backend.peek().dictation).toMatchObject({ session: 1, kind: "dictation" });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(backend.peek().dictation.phase.phase).toBe("done");
    // The CLI (`voltip --edit-toggle`): a press starts, the next press stops.
    await editEdge(backend, true, "cli");
    expect(backend.peek().dictation).toMatchObject({ session: 2, kind: "edit" });
    tick(MOCK_COPY_MS + MOCK_MIC_READY_MS);
    await backend.invoke("hotkey_edge", { pressed: true, source: "hotkey" });
    expect(backend.peek().dictation).toMatchObject({ session: 2, phase: { phase: "listening" } });
    await editEdge(backend, true, "cli");
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "processing" });
    tick(MOCK_ASR_MS + MOCK_REFINE_MS + MOCK_FINALIZE_MS);
    expect(backend.peek().dictation.phase).toMatchObject({ phase: "done", text: MOCK_EDIT_TEXT });
    backend.destroy();
  });

  it("regression: settings_set_edit_hotkey validates like the core: never the dictation chord and null switches off and the hotkey status follows both chords", async () => {
    const backend = new MockBackend({ now: () => clock, history: [] });
    const events = collect(backend);
    expect(backend.peek().settings.edit_hotkey).toBe("Ctrl+Alt+E");
    expect(backend.peek().hotkey.edit_registered).toBe("Ctrl+Alt+E");
    await backend.invoke("settings_set_edit_hotkey", { hotkey: "Ctrl+Alt+Shift+E" });
    expect(backend.peek().settings.edit_hotkey).toBe("Ctrl+Alt+Shift+E");
    expect(backend.peek().hotkey).toMatchObject({
      registered: "Ctrl+Alt+Space",
      edit_registered: "Ctrl+Alt+Shift+E",
    });
    const errors = () => events.flatMap((e) => (e.type === "error" ? [e.message] : []));
    await backend.invoke("settings_set_edit_hotkey", { hotkey: "E" });
    await backend.invoke("settings_set_edit_hotkey", { hotkey: "alt+ctrl+space" });
    await backend.invoke("settings_set_hotkey", { hotkey: "shift+ctrl+alt+e" });
    expect(errors()).toEqual([
      expect.stringContaining("修饰键"),
      "edit_hotkey: alt+ctrl+space 已用作听写快捷键",
      "hotkey: shift+ctrl+alt+e 已用作「编辑选中文本」的快捷键",
    ]);
    expect(backend.peek().settings).toMatchObject({
      hotkey: "Ctrl+Alt+Space",
      edit_hotkey: "Ctrl+Alt+Shift+E",
    });
    // The recorder suspends both registrations and closing it restores both.
    await backend.invoke("hotkey_capture", { active: true });
    expect(backend.peek().hotkey.registered).toBeUndefined();
    expect(backend.peek().hotkey.edit_registered).toBeUndefined();
    await backend.invoke("hotkey_capture", { active: false });
    expect(backend.peek().hotkey.edit_registered).toBe("Ctrl+Alt+Shift+E");
    await backend.invoke("settings_set_edit_hotkey", { hotkey: null });
    expect(backend.peek().settings.edit_hotkey).toBeNull();
    expect(backend.peek().hotkey.edit_registered).toBeUndefined();
    expect(backend.peek().hotkey.registered).toBe("Ctrl+Alt+Space");
    // Off: the dictation key may take any chord again.
    await backend.invoke("settings_set_hotkey", { hotkey: "Ctrl+Alt+E" });
    expect(backend.peek().settings.hotkey).toBe("Ctrl+Alt+E");
    const seeded = new MockBackend({ settings: { ...defaultSettings(), edit_hotkey: null } });
    expect(seeded.peek().hotkey.edit_registered).toBeUndefined();
    expect(mockSameChord("Ctrl+Alt+E", "alt + control + e")).toBe(true);
    expect(mockSameChord("Ctrl+Alt+E", "Ctrl+Alt+Shift+E")).toBe(false);
    expect(mockSameChord("Cmd+E", "Meta+e")).toBe(true);
    seeded.destroy();
    backend.destroy();
  });
});

describe("MockBackend LAN pairing, always-on pairing and the phone's commands", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** The desktops a phone has paired, the second one online. */
  function onlineDesktop(now: number) {
    const desktop = sampleDevices(now)[1];
    if (!desktop) throw new Error("fixture");
    return { ...desktop, connection: { state: "online" as const, via: "direct" as const } };
  }

  it("regression: a nearby desktop is joined only while it waits; discovery off lists nothing, and the desktop lists none", async () => {
    const phone = new MockBackend({ role: "phone" });
    const events = collect(phone);
    expect(phone.peek().nearby).toEqual([...MOCK_NEARBY]);
    await phone.invoke("pairing_join_nearby", { fingerprint: "0000000000000000" });
    expect(events.at(-1)).toEqual({ type: "error", message: "pairing: 附近没有找到此设备" });
    const waiting = MOCK_NEARBY[0];
    if (!waiting) throw new Error("fixture");
    phone.publish({ type: "nearby", devices: [{ ...waiting, pairing: false }] });
    await phone.invoke("pairing_join_nearby", { fingerprint: waiting.fingerprint });
    expect(events.at(-1)).toEqual({ type: "error", message: "pairing: 此设备当前没有等待配对" });
    phone.publish({ type: "nearby", devices: [...MOCK_NEARBY] });
    await phone.invoke("pairing_join_nearby", { fingerprint: waiting.fingerprint });
    expect(phone.peek().pairing.state).toEqual({ state: "creating_session" });
    await phone.invoke("settings_set_lan_discovery", { enabled: false });
    expect(phone.peek().settings.lan_discovery).toBe(false);
    expect(phone.peek().nearby).toEqual([]);
    phone.destroy();
    const desktop = new MockBackend();
    await desktop.invoke("settings_set_lan_discovery", { enabled: true });
    expect(desktop.peek().nearby).toEqual([]);
    desktop.destroy();
  });

  it("regression: always-on pairing renews before the code lapses, opens the next window after a pairing or a cancel, and closes the waiting one when off; the phone refuses it", async () => {
    const phone = new MockBackend({ role: "phone" });
    const phoneEvents = collect(phone);
    await phone.invoke("settings_set_pairing_always_on", { enabled: true });
    expect(phoneEvents.at(-1)).toEqual({ type: "error", message: ALWAYS_ON_DESKTOP_ONLY });
    expect(phone.peek().settings.pairing_always_on).toBe(false);
    phone.destroy();

    const desktop = new MockBackend({ ttlSecs: 12 });
    await desktop.invoke("settings_set_pairing_always_on", { enabled: true });
    expect(desktop.peek().settings.pairing_always_on).toBe(true);
    vi.advanceTimersByTime(300);
    expect(desktop.peek().pairing.state).toEqual({ state: "waiting_for_peer" });
    const first = desktop.peek().pairing.session_id;
    // Switching it on again while a window is open changes nothing.
    await desktop.invoke("settings_set_pairing_always_on", { enabled: true });
    expect(desktop.peek().pairing.session_id).toBe(first);
    // 12 s code, renewed with 10 s left: a new session, never the expired screen.
    vi.advanceTimersByTime(2000);
    vi.advanceTimersByTime(300);
    expect(desktop.peek().pairing.state).toEqual({ state: "waiting_for_peer" });
    expect(desktop.peek().pairing.session_id).not.toBe(first);
    // A pairing, then the next window after the pause.
    desktop.simulatePeerJoined();
    vi.advanceTimersByTime(400);
    await desktop.invoke("pairing_confirm");
    desktop.simulatePeerConfirmed();
    expect(desktop.peek().pairing.state).toEqual({ state: "trusted" });
    vi.advanceTimersByTime(MOCK_ALWAYS_ON_PAUSE_MS + 300);
    expect(desktop.peek().pairing.state).toEqual({ state: "waiting_for_peer" });
    // A cancel is followed by the next window too, and 完成 (reset) opens one straight away.
    await desktop.invoke("pairing_cancel");
    expect(desktop.peek().pairing.state.state).toBe("failed");
    vi.advanceTimersByTime(MOCK_ALWAYS_ON_PAUSE_MS + 300);
    expect(desktop.peek().pairing.state).toEqual({ state: "waiting_for_peer" });
    await desktop.invoke("pairing_reset");
    expect(desktop.peek().pairing.state).toEqual({ state: "idle" });
    vi.advanceTimersByTime(300);
    expect(desktop.peek().pairing.state).toEqual({ state: "waiting_for_peer" });
    // Off: the waiting window closes, and nothing opens after.
    await desktop.invoke("settings_set_pairing_always_on", { enabled: false });
    expect(desktop.peek().pairing.state).toEqual({ state: "idle" });
    vi.advanceTimersByTime(MOCK_ALWAYS_ON_PAUSE_MS + 20_000);
    expect(desktop.peek().pairing.state).toEqual({ state: "idle" });
    // Off during a pairing under way: it runs to its end.
    await desktop.invoke("pairing_start");
    vi.advanceTimersByTime(300);
    desktop.simulatePeerJoined();
    await desktop.invoke("settings_set_pairing_always_on", { enabled: false });
    expect(desktop.peek().pairing.state.state).not.toBe("idle");
    desktop.destroy();
  });

  it("regression: the phone's texts are refused on the desktop, when empty, too long or to an offline computer; ids keep counting after 清空", async () => {
    const desktop = new MockBackend();
    const desktopEvents = collect(desktop);
    await desktop.invoke("phone_text_send", {
      publicKey: MOCK_PUBLIC_KEYS.desktop,
      body: "x",
      source: "typed",
    });
    expect(desktopEvents.at(-1)).toEqual({ type: "error", message: PHONE_TEXT_UNAVAILABLE });
    desktop.destroy();

    const now = Date.now();
    const target = onlineDesktop(now);
    const offline = { ...target, connection: { state: "offline" as const } };
    const phone = new MockBackend({ role: "phone", devices: [target] });
    const events = collect(phone);
    const send = (body: string, publicKey = target.device.public_key) =>
      phone.invoke("phone_text_send", { publicKey, body, source: "typed" });
    await send("   ");
    expect(events.at(-1)).toEqual({ type: "error", message: "phone text: 没有要发送的文字" });
    await send("字".repeat(MAX_PHONE_TEXT_CHARS + 1));
    expect(events.at(-1)).toMatchObject({ type: "error" });
    await send("x", MOCK_PUBLIC_KEYS.phone);
    expect(events.at(-1)).toEqual({ type: "error", message: "设备不在线" });
    await send("第一条");
    vi.advanceTimersByTime(MOCK_TEXT_MS);
    expect(phone.peek().sent_texts[0]).toMatchObject({
      id: 1,
      state: { state: "delivered", pasted: true },
    });
    await phone.invoke("sent_texts_clear");
    expect(phone.peek().sent_texts).toEqual([]);
    await send("清空后");
    expect(phone.peek().sent_texts[0]?.id).toBe(2);
    phone.publish({ type: "devices", devices: [offline] });
    await send("离线");
    expect(events.at(-1)).toEqual({ type: "error", message: "设备不在线" });
    phone.destroy();
  });

  it("a phone take needs an online computer and one at a time; stop and cancel without one are errors; a second connection check waits for the first", async () => {
    const now = Date.now();
    const target = onlineDesktop(now);
    const phone = new MockBackend({ role: "phone", devices: [target] });
    const events = collect(phone);
    await phone.invoke("phone_take_stop");
    expect(events.at(-1)).toEqual({ type: "error", message: "phone take: 没有进行中的录音" });
    await phone.invoke("phone_take_cancel");
    expect(events.at(-1)).toEqual({ type: "error", message: "phone take: 没有进行中的录音" });
    await phone.invoke("phone_take_start", { publicKey: MOCK_PUBLIC_KEYS.phone });
    expect(events.at(-1)).toEqual({ type: "error", message: "设备不在线" });
    await phone.invoke("phone_take_start", { publicKey: target.device.public_key });
    await phone.invoke("phone_take_start", { publicKey: target.device.public_key });
    expect(events.at(-1)).toEqual({ type: "error", message: "phone take: 已有一次录音在进行" });
    vi.advanceTimersByTime(MOCK_MIC_READY_MS);
    expect(phone.peek().phone_take?.state.state).toBe("listening");
    await phone.invoke("connectivity_check");
    await phone.invoke("connectivity_check");
    expect(events.at(-1)).toEqual({ type: "error", message: "connectivity: 自检正在进行" });
    phone.destroy();
  });
});

describe("MockBackend history queries (docs/dictation.md section 4.4)", () => {
  const T = 1_758_700_000_000;
  const row = (i: number): HistoryEntry => ({
    id: `id-${i}`,
    at_ms: T - i * 60_000,
    raw_text: `第${i}句`,
    text: `第${i}句。`,
    refined: true,
    asr_model: "m",
    duration_ms: 1000,
    asr_ms: 100,
    refine_ms: 50,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
  });

  it("keeps the whole history for the queries and tells the UI the newest entries and the total", async () => {
    const rows = Array.from({ length: 30 }, (_, i) => row(i));
    const backend = new MockBackend({ now: () => T, history: rows });
    expect(backend.peek().history_recent).toEqual(rows.slice(0, HISTORY_RECENT));
    expect(backend.peek().history_total).toBe(30);
    const tail = await backend.historyQuery({ offset: 25, limit: 10 });
    expect(tail.entries.map((e) => e.id)).toEqual(["id-25", "id-26", "id-27", "id-28", "id-29"]);
    expect([tail.matching, tail.total]).toEqual([30, 30]);
    expect(await backend.historyEntry("id-29")).toEqual(rows[29]);
    expect(await backend.historyEntry("gone")).toBeNull();
    expect((await backend.historyQuery({ query: "第29句", limit: 5 })).entries).toEqual([rows[29]]);
    await backend.invoke("history_star", { id: "id-29", starred: true });
    expect((await backend.historyQuery({ starred: true, limit: 5 })).entries[0]?.id).toBe("id-29");
    await backend.invoke("history_delete", { id: "id-0" });
    expect(backend.log.at(-1)).toMatchObject({ type: "history", total: 29 });
    expect(backend.peek().history_recent[0]?.id).toBe("id-1");
    const stats = await backend.historyStats([T - 3 * 60_000, T + 1]);
    // id-1 to id-3: id-0 was deleted.
    expect(stats.buckets[0]?.count).toBe(3);
    expect(stats.total.count).toBe(29);
    await expect(backend.historyQuery({ limit: 0 })).rejects.toThrow(/limit/);
    await expect(backend.historyStats([5])).rejects.toThrow(/boundaries/);
    await backend.invoke("history_clear");
    expect(backend.peek().history_total).toBe(0);
    expect((await backend.historyHits()).dictionary).toEqual({});
    backend.destroy();
  });

  // Changed by the user's request of 2026-09-30 (item 10, docs/dictation.md §20.7): the phone
  // recognises takes itself when no paired computer is online and keeps them in its own history,
  // so the queries read it as the desktop's do (they answered empty before).
  it("the phone's history holds the takes it recognised itself", async () => {
    const phone = new MockBackend({ role: "phone", history: [row(0)] });
    expect((await phone.historyQuery({ limit: 10 })).entries.map((e) => e.id)).toEqual(["id-0"]);
    expect((await phone.historyEntry("id-0"))?.id).toBe("id-0");
    expect((await phone.historyStats([0, 1, 2])).buckets).toHaveLength(2);
    expect(await phone.historyHits()).toEqual({ dictionary: {}, rules: {} });
    await expect(phone.historyQuery({ limit: 0 })).rejects.toThrow(/limit/);
    phone.destroy();
  });

  it("the phone copies instead of pasting and shares through its own sheet (§20.7)", async () => {
    const phone = new MockBackend({ role: "phone" });
    expect(await phone.pasteText("今天下午三点开会。")).toEqual({
      kind: "copied",
      reason: "clipboard_only",
    });
    expect(phone.phoneClipboard).toBe("今天下午三点开会。");
    expect(await phone.pasteText("  ")).toEqual({ kind: "failed", reason: "invalid" });
    await phone.invoke("phone_share_text", { text: "今天下午三点开会。" });
    expect(phone.shared).toEqual(["今天下午三点开会。"]);
    await expect(phone.invoke("phone_share_text", { text: " " })).rejects.toThrow(/share/);
    phone.destroy();
    const desktop = new MockBackend();
    await expect(desktop.invoke("phone_share_text", { text: "x" })).rejects.toThrow(
      SHARE_UNAVAILABLE,
    );
    desktop.destroy();
  });
});
