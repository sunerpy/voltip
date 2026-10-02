import {
  CHINESE_SCRIPTS,
  DEFAULT_EDIT_HOTKEY,
  DEFAULT_HOLD_THRESHOLD_MS,
  DICTATION_FAILURE_CODES,
  MAX_EDIT_SELECTION_CHARS,
  TAKE_KINDS,
  hotkeyStatusSchema,
  NIL_ID,
  QUERY_COMMANDS,
  MAX_ACTIVATION_MS,
  MODEL_CAPABILITIES,
  MODEL_ENGINES,
  MODEL_TIERS,
  OUTPUT_MODES,
  applyEvent,
  phoneTakeFinal,
  sentTextFinal,
  defaultEngineSettings,
  defaultSettings,
  dictationPhaseSchema,
  isStreamingOutputMode,
  settingsSchema,
  emptyEngineStatus,
  emptyHotkeyStatus,
  engineReady,
  engineSettingsSchema,
  engineStatusSchema,
  historyEntrySchema,
  isRecognitionModel,
  isStreamingModel,
  liveTextSchema,
  modelStateSchema,
  idleDictation,
  idleSnapshot,
  idleUpdate,
  pairingStateSchema,
  BUILTIN_PRESETS,
  presetIdSchema,
  presetTryOutcomeSchema,
  dictationStatusSchema,
  sceneDraftSchema,
  sceneSchema,
  uiEventSchema,
  uiStateSchema,
  vocabularyPreviewSchema,
  APP_LICENSE,
} from "./schema";
import { MOCK_PUBLIC_KEYS, desktopIdentity, sampleDevices, sampleHistory } from "./mock-backend";
import type { EngineStatus, HistoryEntry, UiState } from "./schema";

function baseState(): UiState {
  return {
    sent_texts: [],
    nearby: [],
    mirrors: [],
    phone_outbox_too_large: [],
    identity: desktopIdentity(),
    settings: defaultSettings(),
    secret_backend: "memory",
    app_version: "0.3.0",
    relay: { state: "disconnected", attempts: 0, source: "none" },
    pairing: idleSnapshot(),
    devices: sampleDevices(1_700_000_000),
    hotkey: emptyHotkeyStatus(),
    dictation: idleDictation(),
    history_recent: [],
    history_total: 0,
    engines: emptyEngineStatus(),
    update: idleUpdate(),
    models: [],
    dictionary: [],
    rules: [],
    scenes: [],
    presets: [],
    hardware: { cpu_threads: 0, gpus: [] },
    connectivity: { running: false },
  };
}

const ENGINES: EngineStatus = {
  asr_provider: "builtin",
  asr_ready: true,
  asr_model: "Qwen/Qwen3-ASR-1.7B",
  asr_host: "",
  local_ready: false,
  live_preview_ready: false,
  effective_output_mode: "whole_take",
  refine_enabled: true,
  llm_provider: "groq",
  refine_ready: false,
  refine_issue: "key_missing",
  refine_model: "qwen/qwen3.8-27b",
  refine_host: "",
  inject: "paste",
  providers: [
    {
      id: "builtin",
      key: "builtin",
      on_device: false,
      console: false,
      asr: {
        model: "Qwen/Qwen3-ASR-1.7B",
        presets: ["Qwen/Qwen3-ASR-1.7B"],
        key: { set: true, source: "builtin" },
        active: true,
      },
    },
    {
      id: "groq",
      key: "required",
      on_device: false,
      console: true,
      llm: {
        model: "qwen/qwen3.8-27b",
        presets: ["qwen/qwen3.8-27b"],
        base_url: "https://api.groq.com/openai/v1",
        default_base_url: "https://api.groq.com/openai/v1",
        key: { set: false, source: "none" },
        issue: "key_missing",
        active: true,
      },
    },
  ],
};

describe("uiStateSchema", () => {
  it("accepts the Rust core's default state shape", () => {
    const parsed = uiStateSchema.parse(baseState());
    expect(parsed.identity?.public_key).toBe(MOCK_PUBLIC_KEYS.desktop);
    expect(parsed.pairing.state).toEqual({ state: "idle" });
  });

  it("rejects a public key that is not 64 lower-case hex chars", () => {
    const bad = { ...baseState(), identity: { ...desktopIdentity(), public_key: "ABC" } };
    expect(uiStateSchema.safeParse(bad).success).toBe(false);
  });

  it("rejects an unknown theme", () => {
    const bad = { ...baseState(), settings: { ...defaultSettings(), theme: "sepia" } };
    expect(uiStateSchema.safeParse(bad).success).toBe(false);
  });
});

describe("pairingStateSchema", () => {
  it("accepts every simple phase and the failed variant with relay code", () => {
    expect(pairingStateSchema.parse({ state: "awaiting_verification" })).toEqual({
      state: "awaiting_verification",
    });
    expect(
      pairingStateSchema.parse({
        state: "failed",
        reason: { kind: "relay", code: "invalid_code" },
      }),
    ).toEqual({ state: "failed", reason: { kind: "relay", code: "invalid_code" } });
    expect(pairingStateSchema.parse({ state: "failed", reason: { kind: "peer_left" } })).toEqual({
      state: "failed",
      reason: { kind: "peer_left" },
    });
  });

  it("rejects failed without a reason", () => {
    expect(pairingStateSchema.safeParse({ state: "failed" }).success).toBe(false);
  });
});

describe("uiEventSchema", () => {
  it("parses flattened struct variants and the wrapped devices array", () => {
    const state = uiEventSchema.parse({ type: "state", ...baseState() });
    expect(state.type).toBe("state");
    const devices = uiEventSchema.parse({ type: "devices", devices: sampleDevices(1) });
    expect(devices.type).toBe("devices");
    const relay = uiEventSchema.parse({ type: "relay", state: "reconnecting", attempts: 3 });
    expect(relay).toEqual({ type: "relay", state: "reconnecting", attempts: 3, source: "none" });
    const err = uiEventSchema.parse({ type: "error", message: "boom" });
    expect(err.type).toBe("error");
  });

  it("rejects a devices event whose payload is a bare array", () => {
    expect(uiEventSchema.safeParse({ type: "devices", 0: {} }).success).toBe(false);
    expect(uiEventSchema.safeParse({ type: "nope" }).success).toBe(false);
  });
});

describe("applyEvent", () => {
  const state = uiStateSchema.parse(baseState());

  it("replaces the whole state on `state`", () => {
    const next = applyEvent(state, { type: "state", ...baseState(), secret_backend: "keyring" });
    expect(next.secret_backend).toBe("keyring");
    expect("type" in next).toBe(false);
  });

  it("patches the matching slice for struct events", () => {
    const identity = { ...desktopIdentity(), name: "Studio" };
    expect(applyEvent(state, { type: "identity", ...identity }).identity?.name).toBe("Studio");
    expect(
      applyEvent(state, { type: "settings", ...defaultSettings(), theme: "dark" }).settings.theme,
    ).toBe("dark");
    expect(
      applyEvent(state, { type: "relay", state: "connected", attempts: 0, source: "builtin" }).relay
        .state,
    ).toBe("connected");
    expect(
      applyEvent(state, { type: "pairing", ...idleSnapshot(), state: { state: "expired" } }).pairing
        .state,
    ).toEqual({ state: "expired" });
    expect(applyEvent(state, { type: "devices", devices: [] }).devices).toEqual([]);
  });

  it("folds the hardware, connectivity, phone take, sent texts and nearby events; a null take clears it", () => {
    const hardware = applyEvent(state, { type: "hardware", cpu_threads: 16, gpus: [] });
    expect(hardware.hardware).toEqual({ cpu_threads: 16, gpus: [] });
    expect(applyEvent(state, { type: "connectivity", running: true }).connectivity.running).toBe(
      true,
    );
    const take = {
      device: "ab".repeat(32),
      take: 3,
      started_at: 1,
      state: { state: "listening" as const },
    };
    const taking = applyEvent(state, { type: "phone_take", take });
    expect(taking.phone_take?.take).toBe(3);
    expect(applyEvent(taking, { type: "phone_take", take: null }).phone_take).toBeUndefined();
    const text = {
      id: 7,
      device: "ab".repeat(32),
      device_name: "Studio",
      body: "x",
      source: "typed" as const,
      sent_at: 1,
      state: { state: "sending" as const },
    };
    expect(applyEvent(state, { type: "sent_texts", texts: [text] }).sent_texts).toEqual([text]);
    const nearby = {
      fingerprint: "AB",
      name: "Studio",
      platform: "macos" as const,
      pairing: true,
      trusted: false,
    };
    expect(applyEvent(state, { type: "nearby", devices: [nearby] }).nearby).toEqual([nearby]);
  });

  it("a sent text and a phone take know when no further answer comes", () => {
    expect(sentTextFinal({ state: "sending" })).toBe(false);
    expect(sentTextFinal({ state: "queued" })).toBe(false);
    expect(sentTextFinal({ state: "delivered", pasted: true })).toBe(true);
    expect(sentTextFinal({ state: "failed", code: "busy", message: "x" })).toBe(true);
    expect(phoneTakeFinal({ state: "starting" })).toBe(false);
    expect(phoneTakeFinal({ state: "done", text: "x", pasted: true })).toBe(true);
    expect(phoneTakeFinal({ state: "failed", code: "unavailable", message: "x" })).toBe(true);
    expect(phoneTakeFinal({ state: "cancelled" })).toBe(true);
  });

  it("leaves state untouched for notification-only events", () => {
    const trusted = sampleDevices(1)[0]?.device;
    if (!trusted) throw new Error("fixture");
    expect(applyEvent(state, { type: "trusted", ...trusted })).toBe(state);
    expect(
      applyEvent(state, {
        type: "identity_changed",
        previous: trusted,
        presented_fingerprint: "00:00",
      }),
    ).toBe(state);
    expect(applyEvent(state, { type: "message", from: "a", body: "b" })).toBe(state);
    expect(applyEvent(state, { type: "error", message: "x" })).toBe(state);
  });

  it("folds dictation, history and engines events into their slices", () => {
    const listening = {
      session: 3,
      phase: { phase: "listening" as const, started_at: 1000, ready: true, locked: false },
      kind: "dictation" as const,
    };
    const next = applyEvent(state, { type: "dictation", ...listening });
    expect(next.dictation).toEqual(listening);
    expect(next.history_recent).toBe(state.history_recent);
    const entries = sampleHistory(1_758_700_000_000);
    const folded = applyEvent(state, { type: "history", recent: entries, total: 312 });
    expect(folded.history_recent).toBe(entries);
    expect(folded.history_total).toBe(312);
    const engines = applyEvent(state, { type: "engines", ...ENGINES }).engines;
    expect(engines).toEqual(ENGINES);
    expect("type" in engines).toBe(false);
    // A probe answer is not folded into the state: the pane that asked shows it.
    expect(
      applyEvent(state, {
        type: "provider_probe",
        provider: "groq",
        kind: "llm",
        result: "ok",
        models: [],
      }),
    ).toBe(state);
  });

  it("regression: the shell's hotkey event replaces the hotkey slice and an old state without it parses with the empty status", () => {
    const before = baseState();
    const next = applyEvent(before, {
      type: "hotkey",
      registered: "Ctrl+Alt+Space",
      pressed: true,
      backend: "global-shortcut · Windows · RegisterHotKey",
    });
    expect(next.hotkey).toEqual({
      registered: "Ctrl+Alt+Space",
      pressed: true,
      backend: "global-shortcut · Windows · RegisterHotKey",
    });
    expect(next.settings).toBe(before.settings);
    const { hotkey: _omitted, ...legacy } = before;
    expect(uiStateSchema.parse(legacy).hotkey).toEqual(emptyHotkeyStatus());
    expect(uiEventSchema.safeParse({ type: "hotkey", pressed: "yes", backend: "x" }).success).toBe(
      false,
    );
  });
});

describe("dictation contract (docs/dictation.md)", () => {
  it("parses every DictationPhase variant and rejects unknown phases or stages", () => {
    const phases = [
      { phase: "idle" },
      { phase: "listening", started_at: 1 },
      { phase: "listening", started_at: 1, ready: false },
      { phase: "processing", stage: "transcribing", started_at: 1 },
      { phase: "processing", stage: "refining", started_at: 1 },
      { phase: "processing", stage: "inserting", started_at: 1 },
      {
        phase: "done",
        text: "a。",
        raw_text: "a",
        chars: 2,
        via: "paste",
        refined: true,
        duration_ms: 1200,
        asr_ms: 400,
        refine_ms: 300,
      },
      {
        phase: "done",
        text: "a",
        raw_text: "a",
        chars: 1,
        via: "clipboard",
        refined: false,
        duration_ms: 1200,
        asr_ms: 400,
        refine_error: "429 rate limited",
      },
      { phase: "failed", message: "没有听到声音" },
      { phase: "failed", message: "粘贴超时", text: "kept" },
      { phase: "cancelled" },
    ];
    for (const phase of phases) expect(dictationPhaseSchema.safeParse(phase).success).toBe(true);
    expect(dictationPhaseSchema.safeParse({ phase: "dreaming" }).success).toBe(false);
    expect(
      dictationPhaseSchema.safeParse({ phase: "processing", stage: "thinking", started_at: 1 })
        .success,
    ).toBe(false);
    expect(dictationPhaseSchema.safeParse({ phase: "listening" }).success).toBe(false);
  });

  it("regression: listening carries ready and the live preview, processing carries preview (docs/dictation.md §11)", () => {
    // `ready` is `#[serde(default)]`: an event without it reads as "no samples yet".
    expect(dictationPhaseSchema.parse({ phase: "listening", started_at: 1 })).toEqual({
      phase: "listening",
      started_at: 1,
      ready: false,
      locked: false,
    });
    const live = {
      committed: [{ text: "把 fetchUser 改成 async，", start_ms: 0, end_ms: 1480 }],
      current: "然后加上错误",
    };
    const listening = dictationPhaseSchema.parse({
      phase: "listening",
      started_at: 1_758_700_600_180,
      ready: true,
      live,
    });
    expect(listening).toEqual({
      phase: "listening",
      started_at: 1_758_700_600_180,
      ready: true,
      live: { ...live, injected: 0 },
      locked: false,
    });
    // `degraded` rides along; `live` without it has no key (skip-if-none), never `null`.
    const degraded = dictationPhaseSchema.parse({
      phase: "listening",
      started_at: 1,
      ready: true,
      live: { ...live, degraded: "live tap overrun" },
    });
    expect(degraded.phase === "listening" && degraded.live?.degraded).toBe("live tap overrun");
    expect(listening.phase === "listening" && "degraded" in (listening.live ?? {})).toBe(false);
    // Every LiveText field is `#[serde(default)]`.
    expect(liveTextSchema.parse({})).toEqual({ committed: [], current: "", injected: 0 });
    expect(liveTextSchema.safeParse({ committed: [{ text: "x" }] }).success).toBe(false);
    expect(
      dictationPhaseSchema.safeParse({ phase: "listening", started_at: 1, live: null }).success,
    ).toBe(false);
    expect(
      dictationPhaseSchema.parse({
        phase: "processing",
        stage: "transcribing",
        started_at: 9,
        preview: "把 fetchUser 改成 async，然后加上错误",
      }),
    ).toEqual({
      phase: "processing",
      stage: "transcribing",
      started_at: 9,
      preview: "把 fetchUser 改成 async，然后加上错误",
    });
    const plain = dictationPhaseSchema.parse({
      phase: "processing",
      stage: "refining",
      started_at: 9,
    });
    expect("preview" in plain).toBe(false);
    expect(
      dictationPhaseSchema.safeParse({
        phase: "processing",
        stage: "refining",
        started_at: 9,
        preview: 3,
      }).success,
    ).toBe(false);
  });

  it("parses HistoryEntry with each outcome and refuses an unknown one", () => {
    const base: HistoryEntry = {
      id: "1b4e28ba-2fa1-11d2-883f-0016d3cca427",
      at_ms: 1_758_700_000_000,
      raw_text: "raw",
      text: "text",
      refined: true,
      asr_model: "Qwen/Qwen3-ASR-1.7B",
      refine_model: "qwen/qwen3.8-27b",
      duration_ms: 6800,
      asr_ms: 400,
      refine_ms: 300,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
    };
    expect(historyEntrySchema.parse(base)).toEqual(base);
    expect(
      historyEntrySchema.safeParse({ ...base, outcome: { kind: "clipboard", reason: "焦点丢失" } })
        .success,
    ).toBe(true);
    expect(
      historyEntrySchema.safeParse({ ...base, outcome: { kind: "failed", reason: "ASR 500" } })
        .success,
    ).toBe(true);
    expect(historyEntrySchema.safeParse({ ...base, outcome: { kind: "lost" } }).success).toBe(
      false,
    );
    const { refine_model: _m, refine_ms: _ms, ...unrefined } = base;
    expect(historyEntrySchema.parse({ ...unrefined, refined: false }).refine_model).toBeUndefined();
  });

  it("parses EngineStatus and Settings.engines, and a state without them gets the defaults", () => {
    expect(engineStatusSchema.parse(ENGINES)).toEqual(ENGINES);
    expect(engineStatusSchema.safeParse({ ...ENGINES, inject: "type" }).success).toBe(false);
    expect(engineStatusSchema.safeParse({ ...ENGINES, asr_provider: "azure" }).success).toBe(false);
    expect(engineStatusSchema.safeParse({ ...ENGINES, refine_issue: "broken" }).success).toBe(
      false,
    );
    const {
      dictation: _d,
      history_recent: _h,
      history_total: _t,
      engines: _e,
      models: _m,
      ...partial
    } = baseState();
    const { engines: _se, ...partialSettings } = partial.settings;
    const parsed = uiStateSchema.parse({ ...partial, settings: partialSettings });
    expect(parsed.dictation).toEqual(idleDictation());
    expect(parsed.history_recent).toEqual([]);
    expect(parsed.history_total).toBe(0);
    expect(parsed.engines).toEqual(emptyEngineStatus());
    expect(parsed.models).toEqual([]);
    expect(parsed.settings.engines).toEqual(defaultEngineSettings());
    expect(defaultEngineSettings()).toEqual({
      asr_provider: "builtin",
      llm_provider: "builtin",
      refine_enabled: true,
      local_device: "auto",
      live_preview: true,
      output_mode: "whole_take",
      vad_trim: false,
      chinese_script: "simplified",
      inject: "paste",
      refine_preset: "proofread",
    });
    // `#[serde(default)]` everywhere: an empty block is the defaults, an empty status the empty one.
    expect(engineSettingsSchema.parse({})).toEqual(defaultEngineSettings());
    expect(engineStatusSchema.parse({})).toEqual(emptyEngineStatus());
    // Per-provider choices are a partial map keyed by provider id; unknown ids are refused.
    const custom = engineSettingsSchema.parse({
      asr_provider: "custom",
      providers: { custom: { asr_url: "http://10.0.0.2:8000/v1", asr_model: "whisper" } },
      local_threads: 6,
    });
    expect(custom.providers?.custom?.asr_model).toBe("whisper");
    expect(custom.providers?.openai).toBeUndefined();
    expect(engineSettingsSchema.safeParse({ providers: { azure: {} } }).success).toBe(false);
    expect(engineSettingsSchema.safeParse({ local_threads: 0 }).success).toBe(false);
    expect(engineSettingsSchema.safeParse({ local_device: "npu" }).success).toBe(false);
    // `live_preview` is `#[serde(default = true)]`: absent reads as on, false stays false.
    expect(engineSettingsSchema.parse({ live_preview: false }).live_preview).toBe(false);
    // Events carry the same shapes flattened next to `type`.
    expect(
      uiEventSchema.safeParse({
        type: "dictation",
        session: 1,
        phase: { phase: "cancelled", injected_chars: 0 },
      }).success,
    ).toBe(true);
    expect(uiEventSchema.safeParse({ type: "history", entries: "none" }).success).toBe(false);
    expect(uiEventSchema.safeParse({ type: "engines", ...ENGINES }).success).toBe(true);
  });
});

describe("local models (docs/dictation.md §10)", () => {
  const MODEL = {
    id: "sense-voice-small",
    name: "轻量",
    engine: "sense_voice" as const,
    tier: "light" as const,
    capabilities: ["offline" as const],
    languages: ["zh", "en", "ja", "ko", "yue"],
    size_bytes: 239_549_735,
    description: "SenseVoice Small，中英日韩粤，自带标点与数字规整（ITN）；240 MB",
    recommended: false,
    repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
    active: false,
    state: { kind: "not_installed" as const },
  };

  it("regression: ModelState carries the product tier, the capabilities and the four engines", () => {
    const qwen = {
      ...MODEL,
      id: "qwen3-asr-0.6b",
      name: "均衡",
      engine: "transcribe_cpp",
      tier: "balanced",
      recommended: true,
      size_bytes: 690_417_824,
    };
    expect(modelStateSchema.parse(qwen)).toEqual(qwen);
    const streaming = {
      ...MODEL,
      id: "zipformer-stream-zh-en",
      name: "实时预览",
      engine: "zipformer_streaming",
      tier: "streaming",
      capabilities: ["streaming"],
      size_bytes: 169_347_218,
    };
    const parsedStreaming = modelStateSchema.parse(streaming);
    expect(parsedStreaming).toEqual(streaming);
    expect(isStreamingModel(parsedStreaming)).toBe(true);
    expect(isRecognitionModel(parsedStreaming)).toBe(false);
    expect(isRecognitionModel(modelStateSchema.parse(qwen))).toBe(true);
    expect(isStreamingModel(modelStateSchema.parse(qwen))).toBe(false);
    expect(modelStateSchema.parse({ ...MODEL, engine: "paraformer" }).engine).toBe("paraformer");
    // `capabilities` is `#[serde(default)]`; `tier` is always written by the core.
    const { capabilities: _c, ...noCaps } = MODEL;
    expect(modelStateSchema.parse(noCaps).capabilities).toEqual([]);
    expect(modelStateSchema.safeParse({ ...MODEL, tier: "ultra" }).success).toBe(false);
    expect(modelStateSchema.safeParse({ ...MODEL, capabilities: ["gpu"] }).success).toBe(false);
    expect(modelStateSchema.safeParse({ ...MODEL, engine: "whisper" }).success).toBe(false);
  });

  it("regression: ModelState parses every install state tagged by kind and rejects unknown ones", () => {
    const states = [
      { kind: "not_installed" },
      { kind: "downloading", received: 1024, total: 239_549_735, file: "model.int8.onnx" },
      { kind: "verifying" },
      { kind: "installed", path: "/data/models/sense-voice-small", installed_at: 1_758_700_000 },
      { kind: "failed", message: "sha256 mismatch" },
    ];
    for (const state of states)
      expect(modelStateSchema.parse({ ...MODEL, state })).toEqual({ ...MODEL, state });
    expect(modelStateSchema.safeParse({ ...MODEL, state: { kind: "partial" } }).success).toBe(
      false,
    );
    expect(
      modelStateSchema.safeParse({ ...MODEL, state: { kind: "downloading", received: 1 } }).success,
    ).toBe(false);
    expect(modelStateSchema.safeParse({ ...MODEL, engine: "whisper" }).success).toBe(false);
    expect(modelStateSchema.safeParse({ ...MODEL, size_bytes: -1 }).success).toBe(false);
  });

  it("the models event replaces UiState.models; a state without the field parses to []", () => {
    const state = { ...baseState(), models: [MODEL] };
    expect(uiStateSchema.parse(state).models).toEqual([MODEL]);
    const active = { ...MODEL, active: true };
    const event = uiEventSchema.parse({ type: "models", models: [active] });
    expect(event.type).toBe("models");
    expect(applyEvent(state, event).models).toEqual([active]);
    expect(uiEventSchema.safeParse({ type: "models", models: "none" }).success).toBe(false);
    expect(uiEventSchema.safeParse({ type: "models" }).success).toBe(false);
  });

  it("regression: readiness is the core's asr_ready, whatever the provider", () => {
    expect(engineReady(ENGINES)).toBe(true);
    expect(engineReady({ ...ENGINES, asr_ready: false, asr_issue: "key_missing" })).toBe(false);
    expect(engineReady(emptyEngineStatus())).toBe(false);
    const local: EngineStatus = {
      ...ENGINES,
      asr_provider: "local",
      local_model: "sense-voice-small",
      asr_model: "轻量",
      local_ready: true,
    };
    expect(engineReady(local)).toBe(true);
    expect(engineStatusSchema.parse(local)).toEqual(local);
    expect(
      engineSettingsSchema.parse({ asr_provider: "local", local_model: "paraformer-zh" }),
    ).toEqual({ ...defaultEngineSettings(), asr_provider: "local", local_model: "paraformer-zh" });
    // Absent `local_model` stays absent (= the catalogue default), mirroring Rust's skip-if-none.
    expect(engineSettingsSchema.parse({ asr_provider: "local" }).local_model).toBeUndefined();
  });
});

describe("output modes and activation (docs/dictation.md §12–§13)", () => {
  it("regression: a core that always serialises the section 12 and 13 fields round-trips them, and a payload from before them parses to the Rust defaults", () => {
    // Rust: `#[serde(default)]` without `skip_serializing_if` — the keys are always on the wire
    // (the fixtures prove it in ipc-contract.test.ts); the TS defaults exist for older cores and for
    // hand-written test payloads, mirroring `Settings::default()` / `EngineSettings::default()`.
    const settings = settingsSchema.parse({ ...defaultSettings(), activation: undefined });
    expect(settings.activation).toBe("hold");
    const {
      activation: _a,
      hold_threshold_ms: _h,
      extra_recording_ms: _e,
      ...legacy
    } = defaultSettings();
    expect(settingsSchema.parse(legacy)).toEqual({
      ...legacy,
      activation: "hold",
      hold_threshold_ms: DEFAULT_HOLD_THRESHOLD_MS,
      extra_recording_ms: 0,
    });
    expect(DEFAULT_HOLD_THRESHOLD_MS).toBe(300);
    expect(MAX_ACTIVATION_MS).toBe(5000);
    const explicit = settingsSchema.parse({
      ...legacy,
      activation: "hold_or_toggle",
      hold_threshold_ms: 450,
      extra_recording_ms: 200,
    });
    expect(explicit).toMatchObject({
      activation: "hold_or_toggle",
      hold_threshold_ms: 450,
      extra_recording_ms: 200,
    });
    for (const bad of [
      { activation: "press" },
      { hold_threshold_ms: -1 },
      { hold_threshold_ms: 1.5 },
      { extra_recording_ms: "0" },
    ])
      expect(settingsSchema.safeParse({ ...legacy, ...bad }).success).toBe(false);
    // Engine settings and status.
    const { output_mode: _om, vad_trim: _vt, ...engines } = defaultEngineSettings();
    expect(engineSettingsSchema.parse(engines)).toEqual(defaultEngineSettings());
    expect(
      engineSettingsSchema.parse({ ...engines, output_mode: "live_inject", vad_trim: true }),
    ).toMatchObject({ output_mode: "live_inject", vad_trim: true });
    expect(engineSettingsSchema.safeParse({ ...engines, output_mode: "batch" }).success).toBe(
      false,
    );
    expect(engineStatusSchema.parse({ ...ENGINES, effective_output_mode: undefined })).toEqual(
      ENGINES,
    );
    expect(
      engineStatusSchema.parse({ ...ENGINES, effective_output_mode: "streaming_final" })
        .effective_output_mode,
    ).toBe("streaming_final");
    expect(emptyEngineStatus().effective_output_mode).toBe("whole_take");
    expect(OUTPUT_MODES).toEqual(["whole_take", "streaming_final", "live_inject"]);
    expect(OUTPUT_MODES.map(isStreamingOutputMode)).toEqual([false, true, true]);
  });

  it("regression: the new phase fields parse and default like serde: listening.locked, live.injected, the finalizing stage, done.mode / segments / live_error and cancelled.injected_chars; bad values are refused", () => {
    expect(
      dictationPhaseSchema.parse({ phase: "listening", started_at: 1, ready: true, locked: true }),
    ).toMatchObject({ locked: true });
    expect(dictationPhaseSchema.parse({ phase: "listening", started_at: 1 })).toMatchObject({
      locked: false,
    });
    expect(
      dictationPhaseSchema.safeParse({ phase: "listening", started_at: 1, locked: "yes" }).success,
    ).toBe(false);
    const live = {
      committed: [{ text: "把 fetchUser 改成 async，", start_ms: 0, end_ms: 1480 }],
      current: "然后加上错误",
    };
    expect(liveTextSchema.parse({ ...live, injected: 1 }).injected).toBe(1);
    expect(liveTextSchema.parse(live).injected).toBe(0);
    expect(liveTextSchema.safeParse({ ...live, injected: -1 }).success).toBe(false);
    expect(
      dictationPhaseSchema.parse({
        phase: "processing",
        stage: "finalizing",
        started_at: 9,
        preview: "把 fetchUser",
      }),
    ).toEqual({ phase: "processing", stage: "finalizing", started_at: 9, preview: "把 fetchUser" });
    const done = {
      phase: "done",
      text: "a。",
      raw_text: "a",
      chars: 2,
      via: "paste",
      refined: false,
      duration_ms: 1200,
      asr_ms: 45,
    };
    expect(dictationPhaseSchema.parse(done)).toEqual({ ...done, mode: "whole_take" });
    const streamed = dictationPhaseSchema.parse({
      ...done,
      mode: "streaming_final",
      segments: live.committed,
      live_error: "flush: asr: x",
    });
    expect(streamed).toEqual({
      ...done,
      mode: "streaming_final",
      segments: live.committed,
      live_error: "flush: asr: x",
    });
    expect(dictationPhaseSchema.safeParse({ ...done, mode: "batch" }).success).toBe(false);
    expect(dictationPhaseSchema.safeParse({ ...done, segments: [{ text: "x" }] }).success).toBe(
      false,
    );
    expect(dictationPhaseSchema.parse({ phase: "cancelled" })).toEqual({
      phase: "cancelled",
      injected_chars: 0,
    });
    expect(dictationPhaseSchema.parse({ phase: "cancelled", injected_chars: 21 })).toEqual({
      phase: "cancelled",
      injected_chars: 21,
    });
    expect(
      dictationPhaseSchema.safeParse({ phase: "cancelled", injected_chars: 2.5 }).success,
    ).toBe(false);
    // History rows: the same three fields, a row from before §12 reads as a whole take.
    const row = {
      id: "1b4e28ba-2fa1-11d2-883f-0016d3cca427",
      at_ms: 1,
      raw_text: "r",
      text: "t",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
    };
    expect(historyEntrySchema.parse(row).mode).toBe("whole_take");
    expect(
      historyEntrySchema.parse({ ...row, mode: "live_inject", segments: [], live_error: "e" }),
    ).toMatchObject({ mode: "live_inject", segments: [], live_error: "e" });
    expect(historyEntrySchema.safeParse({ ...row, mode: "nope" }).success).toBe(false);
  });

  it("regression: the auxiliary silero-vad catalogue row parses on the wire but is neither a recognition nor a streaming model, so no card would render it", () => {
    expect(MODEL_TIERS).toContain("auxiliary");
    expect(MODEL_ENGINES).toContain("silero_vad");
    expect(MODEL_CAPABILITIES).toContain("vad");
    const vad = modelStateSchema.parse({
      id: "silero-vad",
      name: "静音检测",
      engine: "silero_vad",
      tier: "auxiliary",
      capabilities: ["vad"],
      languages: [],
      size_bytes: 1_807_522,
      description: "Silero VAD v4",
      recommended: false,
      active: false,
      state: { kind: "not_installed" },
    });
    expect(isRecognitionModel(vad)).toBe(false);
    expect(isStreamingModel(vad)).toBe(false);
    // A models event carrying it is not dropped as a whole.
    expect(uiEventSchema.safeParse({ type: "models", models: [vad] }).success).toBe(true);
  });
});

describe("personal dictionary and replacement rules (docs/dictation.md section 16)", () => {
  const entry = {
    id: "3b241101-e2bb-4255-8caf-4136c566a962",
    term: "fetchUser",
    heard_as: ["fetch user"],
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: 1,
    updated_at_ms: 2,
  };
  const rule = {
    id: "8d7e6f5a-4b3c-4d2e-9f1a-0b9c8d7e6f5a",
    name: "git push",
    kind: "literal",
    pattern: "给他push",
    replacement: "git push",
    case_sensitive: true,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
  };

  it("regression: dictionary and rules parse as UiState lists and events and an old state without them reads as empty", () => {
    const { dictionary: _d, rules: _r, ...old } = baseState();
    const parsed = uiStateSchema.parse(old);
    expect(parsed.dictionary).toEqual([]);
    expect(parsed.rules).toEqual([]);
    const state = uiStateSchema.parse({ ...baseState(), dictionary: [entry], rules: [rule] });
    expect(state.dictionary[0]?.source).toEqual({ kind: "manual" });
    expect(state.rules[0]?.kind).toBe("literal");
    // `heard_as` and `replacement` are `#[serde(default)]` on the Rust side.
    const { heard_as: _h, ...bare } = entry;
    expect(
      uiStateSchema.parse({ ...baseState(), dictionary: [bare] }).dictionary[0]?.heard_as,
    ).toEqual([]);
    const { replacement: _x, ...noReplacement } = rule;
    expect(
      uiStateSchema.parse({ ...baseState(), rules: [noReplacement] }).rules[0]?.replacement,
    ).toBe("");
    const fromHistory = { ...entry, source: { kind: "history", history_id: "h1" } };
    const next = applyEvent(
      state,
      uiEventSchema.parse({ type: "dictionary", entries: [fromHistory] }),
    );
    expect(next.dictionary[0]?.source).toEqual({ kind: "history", history_id: "h1" });
    expect(next.rules).toEqual(state.rules);
    const cleared = applyEvent(next, uiEventSchema.parse({ type: "rules", rules: [] }));
    expect(cleared.rules).toEqual([]);
    expect(cleared.dictionary).toEqual(next.dictionary);
    for (const bad of [
      { type: "dictionary", entries: [{ ...entry, source: { kind: "learned" } }] },
      { type: "dictionary", entries: [{ ...entry, enabled: "yes" }] },
      { type: "rules", rules: [{ ...rule, kind: "glob" }] },
      { type: "rules", rules: [{ ...rule, pattern: undefined }] },
      { type: "rules", rules: "none" },
    ])
      expect(uiEventSchema.safeParse(bad).success).toBe(false);
  });

  it("regression: history rows carry the vocabulary hits and rows without them parse unchanged", () => {
    const row = {
      id: "h1",
      at_ms: 1,
      raw_text: "a",
      text: "b",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
    };
    expect(historyEntrySchema.parse(row).vocabulary).toBeUndefined();
    const hits = { corrections: [{ id: entry.id, count: 1 }], rules: [{ id: rule.id, count: 2 }] };
    expect(historyEntrySchema.parse({ ...row, vocabulary: hits }).vocabulary).toEqual(hits);
    expect(historyEntrySchema.parse({ ...row, vocabulary: {} }).vocabulary).toEqual({
      corrections: [],
      rules: [],
    });
    expect(
      historyEntrySchema.safeParse({
        ...row,
        vocabulary: { corrections: [{ id: "x", count: -1 }] },
      }).success,
    ).toBe(false);
  });

  it("regression: the preview answer parses with and without error and the queries are not mutations", () => {
    const answer = {
      corrected: "a",
      output: "b",
      corrections: [],
      rules: [{ id: NIL_ID, count: 1 }],
    };
    expect(vocabularyPreviewSchema.parse(answer).error).toBeUndefined();
    expect(vocabularyPreviewSchema.parse({ ...answer, error: "rules: x" }).error).toBe("rules: x");
    expect(vocabularyPreviewSchema.safeParse({ ...answer, output: 1 }).success).toBe(false);
    expect(QUERY_COMMANDS).toContain("rules_export");
    expect(QUERY_COMMANDS).toContain("vocabulary_preview");
  });

  it("regression: Settings engines chinese_script defaults to simplified and refuses other scripts", () => {
    expect(CHINESE_SCRIPTS).toEqual(["simplified", "traditional", "as_is"]);
    expect(defaultEngineSettings().chinese_script).toBe("simplified");
    const { chinese_script: _c, ...old } = defaultEngineSettings();
    expect(engineSettingsSchema.parse(old).chinese_script).toBe("simplified");
    for (const script of CHINESE_SCRIPTS)
      expect(engineSettingsSchema.parse({ ...old, chinese_script: script }).chinese_script).toBe(
        script,
      );
    expect(engineSettingsSchema.safeParse({ ...old, chinese_script: "hant" }).success).toBe(false);
  });
});

describe("scenes and context (docs/dictation.md section 18)", () => {
  const scene = {
    id: "5c0ffee0-1a2b-4c3d-8e4f-5a6b7c8d9e0f",
    name: "聊天",
    enabled: true,
    match: { apps: ["slack"], title_contains: [] },
    overrides: { refine_preset: "punctuation", prompt: "口语化" },
    created_at_ms: 1,
    updated_at_ms: 1,
  };

  it("regression: scenes parse as a UiState list and an event and an old state or settings without them reads with the defaults", () => {
    const { scenes: _s, ...old } = baseState();
    expect(uiStateSchema.parse(old).scenes).toEqual([]);
    const { context_sharing: _c, ...oldSettings } = defaultSettings();
    expect(settingsSchema.parse(oldSettings).context_sharing).toEqual({
      app_name: true,
      window_title: false,
    });
    expect(settingsSchema.parse({ ...oldSettings, context_sharing: {} }).context_sharing).toEqual({
      app_name: true,
      window_title: false,
    });
    const state = uiStateSchema.parse({ ...baseState(), scenes: [scene] });
    expect(state.scenes[0]?.match.apps).toEqual(["slack"]);
    const next = applyEvent(state, uiEventSchema.parse({ type: "scenes", scenes: [] }));
    expect(next.scenes).toEqual([]);
    expect(next.rules).toEqual(state.rules);
    // `title_contains` and `overrides` are `#[serde(default)]` on the Rust side.
    const { overrides: _o, ...bare } = scene;
    const minimal = sceneSchema.parse({ ...bare, match: { apps: ["x"] } });
    expect(minimal.overrides).toEqual({});
    expect(minimal.match.title_contains).toEqual([]);
    for (const bad of [
      { type: "scenes", scenes: [{ ...scene, overrides: { refine_preset: "casual" } }] },
      { type: "scenes", scenes: [{ ...scene, match: { apps: "slack" } }] },
      { type: "scenes", scenes: [{ ...scene, enabled: "yes" }] },
      { type: "scenes", scenes: "none" },
    ])
      expect(uiEventSchema.safeParse(bad).success).toBe(false);
  });

  it("regression: a draft may send null for the overrides that follow the global setting", () => {
    const draft = sceneDraftSchema.parse({
      name: "a",
      enabled: true,
      match: { apps: ["x"], title_contains: [] },
      overrides: { refine_enabled: null, output_mode: "live_inject", language: null },
    });
    expect(draft.overrides.output_mode).toBe("live_inject");
    expect(draft.overrides.refine_enabled).toBeNull();
  });

  it("regression: the take context rides on the status and the history rows name app and scene", () => {
    const context = { app: { id: "code", name: "Code" }, scene: { id: scene.id, name: "聊天" } };
    const status = dictationStatusSchema.parse({ session: 3, phase: { phase: "idle" }, context });
    expect(status.context).toEqual(context);
    expect(dictationStatusSchema.parse({ session: 3, phase: { phase: "idle" } }).context).toBe(
      undefined,
    );
    const withoutScene = dictationStatusSchema.parse({
      session: 3,
      phase: { phase: "idle" },
      context: { app: context.app },
    });
    expect(withoutScene.context?.scene).toBeUndefined();
    expect(
      dictationStatusSchema.safeParse({ session: 3, phase: { phase: "idle" }, context: {} })
        .success,
    ).toBe(false);
    const row = {
      id: "h1",
      at_ms: 1,
      raw_text: "a",
      text: "b",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
    };
    expect(historyEntrySchema.parse(row).app).toBeUndefined();
    const named = historyEntrySchema.parse({ ...row, app: context.app, scene: context.scene });
    expect([named.app?.name, named.scene?.name]).toEqual(["Code", "聊天"]);
    expect(QUERY_COMMANDS).toContain("recent_apps");
  });
});

describe("presets (docs/dictation.md section 21)", () => {
  const custom = {
    id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
    name: "周报",
    prompt: "整理成周报",
    created_at_ms: 1,
    updated_at_ms: 2,
  };

  it("names a preset by its built-in name or a custom preset's UUID, 校对 by default", () => {
    expect(BUILTIN_PRESETS).toEqual([
      "proofread",
      "prompt",
      "intent",
      "chat",
      "translate",
      "notes",
      "punctuation",
      "formal",
    ]);
    for (const id of [...BUILTIN_PRESETS, custom.id]) expect(presetIdSchema.parse(id)).toBe(id);
    // The core writes the canonical name only; `default` of the refine styles of old never
    // reaches the interface.
    for (const bad of ["default", "casual", "", "7e57ab1e"])
      expect(presetIdSchema.safeParse(bad).success).toBe(false);
    const { refine_preset: _p, ...old } = defaultEngineSettings();
    expect(engineSettingsSchema.parse(old).refine_preset).toBe("proofread");
  });

  it("carries the custom presets in the state and as an event, and 试一试 answers by id", () => {
    const { presets: _p, ...old } = baseState();
    expect(uiStateSchema.parse(old).presets).toEqual([]);
    const state = uiStateSchema.parse({ ...baseState(), presets: [custom] });
    const next = applyEvent(state, uiEventSchema.parse({ type: "presets", presets: [] }));
    expect(next.presets).toEqual([]);
    expect(next.scenes).toEqual(state.scenes);
    const answer = uiEventSchema.parse({
      type: "preset_try",
      id: 4,
      outcome: { status: "ok", text: "好", latency_ms: 10, model: "m" },
    });
    expect(applyEvent(state, answer)).toEqual(state);
    expect(
      presetTryOutcomeSchema.parse({
        status: "failed",
        reason: "尚未配置 AI 润色服务，无法试运行预设",
      }).status,
    ).toBe("failed");
    for (const bad of [
      { type: "presets", presets: [{ ...custom, prompt: 1 }] },
      { type: "preset_try", id: -1, outcome: { status: "failed", reason: "x" } },
      { type: "preset_try", id: 1, outcome: { status: "maybe" } },
    ])
      expect(uiEventSchema.safeParse(bad).success).toBe(false);
  });

  it("names the preset on the status while refining and on the history row", () => {
    const preset = { id: custom.id, name: "周报" };
    const status = dictationStatusSchema.parse({
      session: 1,
      phase: { phase: "idle" },
      preset,
    });
    expect(status.preset).toEqual(preset);
    const row = historyEntrySchema.parse({
      id: "h1",
      at_ms: 1,
      raw_text: "a",
      text: "b",
      refined: true,
      refine_model: "m",
      asr_model: "a",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      preset: { id: "formal", name: "书面语" },
    });
    expect(row.preset).toEqual({ id: "formal", name: "书面语" });
    const { preset: _p, ...older } = row;
    expect(historyEntrySchema.parse(older).preset).toBeUndefined();
  });
});

describe("voice edit (section 19)", () => {
  const row = {
    id: "h1",
    at_ms: 1,
    raw_text: "改的更正式",
    text: "各位同事：会议改至周四上午十点。",
    refined: true,
    asr_model: "m",
    duration_ms: 1,
    asr_ms: 1,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
  };

  it("regression: the take kind rides on the status and the history and an old payload reads as dictation and an edit row carries its record", () => {
    expect(TAKE_KINDS).toEqual(["dictation", "edit"]);
    expect(dictationStatusSchema.parse({ session: 1, phase: { phase: "idle" } }).kind).toBe(
      "dictation",
    );
    expect(idleDictation().kind).toBe("dictation");
    expect(
      dictationStatusSchema.parse({ session: 1, phase: { phase: "idle" }, kind: "edit" }).kind,
    ).toBe("edit");
    expect(
      dictationStatusSchema.safeParse({ session: 1, phase: { phase: "idle" }, kind: "rewrite" })
        .success,
    ).toBe(false);
    const legacy = historyEntrySchema.parse(row);
    expect([legacy.kind, legacy.edit]).toEqual(["dictation", undefined]);
    const edit = historyEntrySchema.parse({
      ...row,
      kind: "edit",
      edit: { instruction: "改得更正式", selection: "会议改到周四十点哈" },
    });
    expect(edit.edit).toEqual({ instruction: "改得更正式", selection: "会议改到周四十点哈" });
    expect(
      historyEntrySchema.safeParse({ ...row, kind: "edit", edit: { instruction: "x" } }).success,
    ).toBe(false);
    expect(MAX_EDIT_SELECTION_CHARS).toBe(2000);
  });

  it("regression: the edit hotkey defaults to Ctrl Alt E for an older settings file and keeps null as off and the four refusal codes and the edit registration parse", () => {
    const { edit_hotkey: _omitted, ...older } = defaultSettings();
    expect(settingsSchema.parse(older).edit_hotkey).toBe(DEFAULT_EDIT_HOTKEY);
    expect(DEFAULT_EDIT_HOTKEY).toBe("Ctrl+Alt+E");
    expect(defaultSettings().edit_hotkey).toBe("Ctrl+Alt+E");
    expect(settingsSchema.parse({ ...older, edit_hotkey: null }).edit_hotkey).toBeNull();
    expect(settingsSchema.safeParse({ ...older, edit_hotkey: 5 }).success).toBe(false);
    for (const code of [
      "no_selection",
      "selection_too_long",
      "selection",
      "edit_unavailable",
      "edit_in_terminal",
    ]) {
      expect(DICTATION_FAILURE_CODES).toContain(code);
      expect(dictationPhaseSchema.parse({ phase: "failed", message: "m", code })).toMatchObject({
        code,
      });
    }
    expect(
      hotkeyStatusSchema.parse({
        pressed: false,
        backend: "b",
        edit_registered: "Ctrl+Alt+E",
      }).edit_registered,
    ).toBe("Ctrl+Alt+E");
    const failed = hotkeyStatusSchema.parse({ pressed: false, backend: "b", edit_error: "e" });
    expect([failed.edit_registered, failed.edit_error]).toEqual([undefined, "e"]);
    expect(emptyHotkeyStatus().edit_registered).toBeUndefined();
  });
});

describe("release facts", () => {
  it("regression: the About pane's license is the one package.json and LICENSE publish", async () => {
    const { readFileSync } = await import("node:fs");
    const root = new URL("../../../", import.meta.url);
    const pkg: unknown = JSON.parse(readFileSync(new URL("package.json", root), "utf8"));
    const license = typeof pkg === "object" && pkg !== null && "license" in pkg ? pkg.license : "";
    expect(license).toBe(APP_LICENSE);
    // User decision 2026-10-01: AGPL-3.0-or-later after 0.0.20 (Apache-2.0 before).
    expect(readFileSync(new URL("LICENSE", root), "utf8")).toContain(
      "GNU AFFERO GENERAL PUBLIC LICENSE",
    );
  });
});
