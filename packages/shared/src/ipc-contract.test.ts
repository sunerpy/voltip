// Cross-language IPC contract: the JSON under fixtures/ipc/ is written by the Rust side
// (crates/voltip-tauri-bridge/tests/contract.rs, serde output of UiState / UiEvent / UiCommand).
// This side proves the zod schemas accept exactly that output and that TauriBackend sends the
// command payloads the Rust side parses.
import { readFileSync } from "node:fs";
import { z } from "zod";

import {
  ACTIVATIONS,
  type CommandArgs,
  EDGE_SOURCES,
  type MutationCommand,
  OUTPUT_MODES,
  PROVIDER_IDS,
  TAKE_KINDS,
  type UiEventType,
  activationSchema,
  builtinSceneSchema,
  dictionaryDraftSchema,
  engineSettingsSchema,
  hexKeySchema,
  historyEntrySchema,
  historyHitsSchema,
  historyPageSchema,
  historyStatsBucketSchema,
  historyStatsSchema,
  importModeSchema,
  ruleDraftSchema,
  localeSettingSchema,
  overlayPlacementSchema,
  presetDraftSchema,
  presetIdSchema,
  sceneDraftSchema,
  providerIdSchema,
  serviceKindSchema,
  settingsSchema,
  recordingSettingsSchema,
  soloKeySchema,
  PHONE_TEXT_SOURCES,
  themeIdSchema,
  uiEventSchema,
  uiStateSchema,
} from "./schema";
import { translate } from "./i18n";
import { PROVIDER_CATALOGUE, resolveEngineStatus } from "./providers";
import { TauriBackend } from "./tauri-backend";

const FIXTURES = new URL("./fixtures/ipc/", import.meta.url);

function loadFixture(name: string): unknown {
  return JSON.parse(readFileSync(new URL(name, FIXTURES), "utf8"));
}

/** Every event tag the Rust enum has (the `Record` makes a new `UiEventType` a compile error). */
const EVENT_TYPE_SET: Record<UiEventType, null> = {
  state: null,
  identity: null,
  settings: null,
  relay: null,
  pairing: null,
  devices: null,
  trusted: null,
  unpaired: null,
  identity_changed: null,
  message: null,
  error: null,
  hotkey: null,
  dictation: null,
  history: null,
  engines: null,
  update: null,
  models: null,
  dictionary: null,
  rules: null,
  scenes: null,
  presets: null,
  preset_try: null,
  provider_probe: null,
  phone_take: null,
  sent_texts: null,
  nearby: null,
  hardware: null,
  connectivity: null,
  paste_result: null,
};
const EVENT_TYPES = Object.keys(EVENT_TYPE_SET);

/** Every command the UI can dispatch (a new `CommandArgs` key is a compile error here). */
const MUTATION_COMMAND_SET: Record<MutationCommand, null> = {
  pairing_start: null,
  pairing_join_code: null,
  pairing_join_ticket: null,
  pairing_confirm: null,
  pairing_reject: null,
  pairing_cancel: null,
  pairing_reset: null,
  device_forget: null,
  device_rename: null,
  send_text: null,
  phone_take_start: null,
  phone_take_stop: null,
  phone_take_cancel: null,
  phone_text_send: null,
  sent_texts_clear: null,
  settings_set_lan_discovery: null,
  settings_set_pairing_always_on: null,
  pairing_join_nearby: null,
  settings_set_relay: null,
  settings_set_theme: null,
  settings_set_hotkey: null,
  settings_set_edit_hotkey: null,
  settings_set_solo_key: null,
  settings_set_microphone: null,
  settings_set_recording: null,
  hotkey_capture: null,
  devices_refresh: null,
  connectivity_check: null,
  dictation_start: null,
  dictation_stop: null,
  dictation_cancel: null,
  settings_set_engines: null,
  provider_key_set: null,
  provider_probe: null,
  history_delete: null,
  history_clear: null,
  history_star: null,
  settings_set_locale: null,
  settings_set_auto_update: null,
  settings_set_history: null,
  settings_set_overlay: null,
  hotkey_edge: null,
  settings_set_activation: null,
  update_check: null,
  update_install: null,
  model_download: null,
  model_cancel: null,
  model_remove: null,
  dictionary_add: null,
  dictionary_update: null,
  dictionary_remove: null,
  dictionary_reorder: null,
  rules_add: null,
  rules_update: null,
  rules_remove: null,
  rules_reorder: null,
  rules_import: null,
  scenes_add: null,
  scenes_update: null,
  scenes_remove: null,
  scenes_reorder: null,
  scenes_restore: null,
  presets_add: null,
  presets_update: null,
  presets_remove: null,
  presets_try: null,
  settings_set_context_sharing: null,
};
const MUTATION_COMMANDS = Object.keys(MUTATION_COMMAND_SET);

function isMutationCommand(name: string): name is MutationCommand {
  return Object.hasOwn(MUTATION_COMMAND_SET, name);
}

const commandEntrySchema = z.object({
  name: z.string().refine(isMutationCommand, "not a mutation command"),
  args: z.record(z.string(), z.unknown()).nullable(),
});
const commandsFileSchema = z.array(commandEntrySchema);

/** Argument shapes as `CommandArgs` declares them (camelCase, the Tauri wire form). */
const argSchemas = {
  pairing_join_code: z.object({ code: z.string() }),
  pairing_join_ticket: z.object({ uri: z.string() }),
  device_forget: z.object({ publicKey: hexKeySchema }),
  device_rename: z.object({ name: z.string() }),
  send_text: z.object({ publicKey: hexKeySchema, body: z.string() }),
  phone_take_start: z.object({ publicKey: hexKeySchema }).strict(),
  phone_text_send: z
    .object({ publicKey: hexKeySchema, body: z.string(), source: z.enum(PHONE_TEXT_SOURCES) })
    .strict(),
  settings_set_lan_discovery: z.object({ enabled: z.boolean() }).strict(),
  settings_set_pairing_always_on: z.object({ enabled: z.boolean() }).strict(),
  pairing_join_nearby: z.object({ fingerprint: z.string() }).strict(),
  settings_set_relay: z.object({ url: z.string().nullable(), enabled: z.boolean() }),
  settings_set_theme: z.object({ theme: themeIdSchema, followSystem: z.boolean() }),
  settings_set_hotkey: z.object({ hotkey: z.string() }),
  settings_set_edit_hotkey: z.object({ hotkey: z.string().nullable() }).strict(),
  settings_set_solo_key: z.object({ key: soloKeySchema.nullable() }).strict(),
  settings_set_microphone: z.object({ device: z.string().nullable() }).strict(),
  settings_set_recording: z.object({ recording: recordingSettingsSchema.strict() }).strict(),
  hotkey_capture: z.object({ active: z.boolean() }),
  settings_set_engines: z.object({ engines: engineSettingsSchema }),
  provider_key_set: z
    .object({ provider: providerIdSchema, kind: serviceKindSchema, value: z.string().nullable() })
    .strict(),
  provider_probe: z
    .object({
      provider: providerIdSchema,
      kind: serviceKindSchema,
      baseUrl: z.string().nullable().optional(),
      key: z.string().nullable().optional(),
    })
    .strict(),
  history_delete: z.object({ id: z.string() }),
  history_star: z.object({ id: z.string(), starred: z.boolean() }),
  settings_set_locale: z.object({ locale: localeSettingSchema }),
  settings_set_auto_update: z.object({ enabled: z.boolean() }),
  settings_set_history: z.object({ enabled: z.boolean(), keep: z.number().int() }).strict(),
  settings_set_overlay: z.object({ placement: overlayPlacementSchema }).strict(),
  hotkey_edge: z.object({
    pressed: z.boolean(),
    atMs: z.number().int().nonnegative().optional(),
    source: z.enum(EDGE_SOURCES).optional(),
    purpose: z.enum(TAKE_KINDS).optional(),
    chorded: z.boolean().optional(),
  }),
  settings_set_activation: z.object({
    activation: activationSchema,
    holdThresholdMs: z.number().int().nonnegative(),
    extraRecordingMs: z.number().int().nonnegative(),
  }),
  model_download: z.object({ id: z.string() }),
  model_cancel: z.object({ id: z.string() }),
  model_remove: z.object({ id: z.string() }),
  dictionary_add: z
    .object({ entry: dictionaryDraftSchema, historyId: z.string().nullable().optional() })
    .strict(),
  dictionary_update: z.object({ id: z.string(), entry: dictionaryDraftSchema.strict() }).strict(),
  dictionary_remove: z.object({ id: z.string() }).strict(),
  dictionary_reorder: z.object({ ids: z.array(z.string()) }).strict(),
  rules_add: z.object({ rule: ruleDraftSchema.strict() }).strict(),
  rules_update: z.object({ id: z.string(), rule: ruleDraftSchema.strict() }).strict(),
  rules_remove: z.object({ id: z.string() }).strict(),
  rules_reorder: z.object({ ids: z.array(z.string()) }).strict(),
  rules_import: z.object({ toml: z.string(), mode: importModeSchema }).strict(),
  scenes_add: z.object({ scene: sceneDraftSchema.strict() }).strict(),
  scenes_update: z.object({ id: z.string(), scene: sceneDraftSchema.strict() }).strict(),
  scenes_remove: z.object({ id: z.string() }).strict(),
  scenes_reorder: z.object({ ids: z.array(z.string()) }).strict(),
  scenes_restore: z.object({ id: z.string() }).strict(),
  presets_add: z.object({ preset: presetDraftSchema.strict() }).strict(),
  presets_update: z.object({ id: z.string(), preset: presetDraftSchema.strict() }).strict(),
  presets_remove: z.object({ id: z.string() }).strict(),
  presets_try: z
    .object({
      id: z.number().int().nonnegative(),
      preset: presetIdSchema.nullable(),
      prompt: z.string().nullable(),
      text: z.string(),
    })
    .strict(),
  settings_set_context_sharing: z
    .object({ appName: z.boolean(), windowTitle: z.boolean() })
    .strict(),
} satisfies {
  [C in MutationCommand as CommandArgs[C] extends undefined ? never : C]: z.ZodType<CommandArgs[C]>;
};

/** Replay one fixture entry through the typed `Backend.invoke`, validating its args on the way. */
function replay(backend: TauriBackend, name: MutationCommand, args: unknown): Promise<void> {
  switch (name) {
    case "pairing_start":
    case "pairing_confirm":
    case "pairing_reject":
    case "pairing_cancel":
    case "pairing_reset":
    case "devices_refresh":
    case "connectivity_check":
    case "dictation_start":
    case "dictation_stop":
    case "dictation_cancel":
    case "history_clear":
    case "update_check":
    case "update_install":
      if (args !== null)
        throw new Error(`${name} takes no args, fixture has ${JSON.stringify(args)}`);
      return backend.invoke(name);
    case "pairing_join_code":
      return backend.invoke(name, argSchemas.pairing_join_code.parse(args));
    case "pairing_join_ticket":
      return backend.invoke(name, argSchemas.pairing_join_ticket.parse(args));
    case "device_forget":
      return backend.invoke(name, argSchemas.device_forget.parse(args));
    case "device_rename":
      return backend.invoke(name, argSchemas.device_rename.parse(args));
    case "send_text":
      return backend.invoke(name, argSchemas.send_text.parse(args));
    case "phone_take_start":
      return backend.invoke(name, argSchemas.phone_take_start.parse(args));
    case "phone_text_send":
      return backend.invoke(name, argSchemas.phone_text_send.parse(args));
    case "settings_set_lan_discovery":
      return backend.invoke(name, argSchemas.settings_set_lan_discovery.parse(args));
    case "settings_set_pairing_always_on":
      return backend.invoke(name, argSchemas.settings_set_pairing_always_on.parse(args));
    case "pairing_join_nearby":
      return backend.invoke(name, argSchemas.pairing_join_nearby.parse(args));
    case "phone_take_stop":
    case "phone_take_cancel":
    case "sent_texts_clear":
      return backend.invoke(name);
    case "settings_set_relay":
      return backend.invoke(name, argSchemas.settings_set_relay.parse(args));
    case "settings_set_theme":
      return backend.invoke(name, argSchemas.settings_set_theme.parse(args));
    case "settings_set_hotkey":
      return backend.invoke(name, argSchemas.settings_set_hotkey.parse(args));
    case "settings_set_edit_hotkey":
      return backend.invoke(name, argSchemas.settings_set_edit_hotkey.parse(args));
    case "settings_set_solo_key":
      return backend.invoke(name, argSchemas.settings_set_solo_key.parse(args));
    case "settings_set_microphone":
      return backend.invoke(name, argSchemas.settings_set_microphone.parse(args));
    case "settings_set_recording":
      return backend.invoke(name, argSchemas.settings_set_recording.parse(args));
    case "hotkey_capture":
      return backend.invoke(name, argSchemas.hotkey_capture.parse(args));
    case "settings_set_engines":
      return backend.invoke(name, argSchemas.settings_set_engines.parse(args));
    case "provider_key_set":
      return backend.invoke(name, argSchemas.provider_key_set.parse(args));
    case "provider_probe":
      return backend.invoke(name, argSchemas.provider_probe.parse(args));
    case "history_delete":
      return backend.invoke(name, argSchemas.history_delete.parse(args));
    case "history_star":
      return backend.invoke(name, argSchemas.history_star.parse(args));
    case "settings_set_locale":
      return backend.invoke(name, argSchemas.settings_set_locale.parse(args));
    case "settings_set_auto_update":
      return backend.invoke(name, argSchemas.settings_set_auto_update.parse(args));
    case "settings_set_history":
      return backend.invoke(name, argSchemas.settings_set_history.parse(args));
    case "settings_set_overlay":
      return backend.invoke(name, argSchemas.settings_set_overlay.parse(args));
    case "hotkey_edge":
      return backend.invoke(name, argSchemas.hotkey_edge.parse(args));
    case "settings_set_activation":
      return backend.invoke(name, argSchemas.settings_set_activation.parse(args));
    case "model_download":
      return backend.invoke(name, argSchemas.model_download.parse(args));
    case "model_cancel":
      return backend.invoke(name, argSchemas.model_cancel.parse(args));
    case "model_remove":
      return backend.invoke(name, argSchemas.model_remove.parse(args));
    case "dictionary_add":
      return backend.invoke(name, argSchemas.dictionary_add.parse(args));
    case "dictionary_update":
      return backend.invoke(name, argSchemas.dictionary_update.parse(args));
    case "dictionary_remove":
      return backend.invoke(name, argSchemas.dictionary_remove.parse(args));
    case "dictionary_reorder":
      return backend.invoke(name, argSchemas.dictionary_reorder.parse(args));
    case "rules_add":
      return backend.invoke(name, argSchemas.rules_add.parse(args));
    case "rules_update":
      return backend.invoke(name, argSchemas.rules_update.parse(args));
    case "rules_remove":
      return backend.invoke(name, argSchemas.rules_remove.parse(args));
    case "rules_reorder":
      return backend.invoke(name, argSchemas.rules_reorder.parse(args));
    case "rules_import":
      return backend.invoke(name, argSchemas.rules_import.parse(args));
    case "scenes_add":
      return backend.invoke(name, argSchemas.scenes_add.parse(args));
    case "scenes_update":
      return backend.invoke(name, argSchemas.scenes_update.parse(args));
    case "scenes_remove":
      return backend.invoke(name, argSchemas.scenes_remove.parse(args));
    case "scenes_reorder":
      return backend.invoke(name, argSchemas.scenes_reorder.parse(args));
    case "scenes_restore":
      return backend.invoke(name, argSchemas.scenes_restore.parse(args));
    case "presets_add":
      return backend.invoke(name, argSchemas.presets_add.parse(args));
    case "presets_update":
      return backend.invoke(name, argSchemas.presets_update.parse(args));
    case "presets_remove":
      return backend.invoke(name, argSchemas.presets_remove.parse(args));
    case "presets_try":
      return backend.invoke(name, argSchemas.presets_try.parse(args));
    case "settings_set_context_sharing":
      return backend.invoke(name, argSchemas.settings_set_context_sharing.parse(args));
  }
}

/** Deep copy with `path` replaced by `value` (or removed when `value` is `undefined`). */
function mutate(input: unknown, path: readonly string[], value: unknown): unknown {
  const copy: unknown = JSON.parse(JSON.stringify(input));
  let cursor: unknown = copy;
  for (const key of path.slice(0, -1)) {
    if (typeof cursor !== "object" || cursor === null) throw new Error(`no ${key} in fixture`);
    cursor = Reflect.get(cursor, key);
  }
  const last = path.at(-1);
  if (typeof cursor !== "object" || cursor === null || last === undefined) {
    throw new Error(`bad path ${path.join(".")}`);
  }
  if (value === undefined) Reflect.deleteProperty(cursor, last);
  else Reflect.set(cursor, last, value);
  return copy;
}

describe("IPC contract fixtures (written by the Rust side)", () => {
  const state = loadFixture("state.json");
  const events = z.array(z.unknown()).parse(loadFixture("events.json"));
  const commands = commandsFileSchema.parse(loadFixture("commands.json"));

  it("state.json is a UiState and every field the UI renders survives parsing", () => {
    const parsed = uiStateSchema.parse(state);
    expect(parsed.identity?.name).toBe("Surface-Laptop");
    expect(parsed.identity?.public_key).toMatch(/^[0-9a-f]{64}$/);
    const { engines, ...settings } = parsed.settings;
    expect(settings).toEqual({
      schema: 1,
      theme: "graphite",
      follow_system_theme: true,
      relay_url: "wss://relay.example.test/ws",
      relay_enabled: true,
      hotkey: "Ctrl+Alt+Space",
      locale: "zh-cn",
      auto_update: true,
      activation: "hold_or_toggle",
      hold_threshold_ms: 300,
      extra_recording_ms: 150,
      edit_hotkey: "Ctrl+Alt+E",
      solo_key: "right_ctrl",
      lan_discovery: true,
      pairing_always_on: true,
      context_sharing: { app_name: false, window_title: true },
      history: { enabled: true, keep: 200 },
      overlay: "top",
      microphone: "wasapi:{0.0.1.00000000}.{c2}",
      recording: {
        source: "mixed",
        output_device: "wasapi:{0.0.0.00000000}.{a1}",
        max_minutes: 60,
      },
    });
    expect(parsed.app_version).toBe("0.3.0");
    // docs/dictation.md §10.6: what the local models can run on, a discrete and an integrated GPU.
    expect(parsed.hardware.cpu_threads).toBe(16);
    expect(parsed.hardware.gpus.map((g) => [g.name, g.integrated])).toEqual([
      ["Vulkan0", false],
      ["Vulkan1", true],
    ]);
    expect(parsed.models.every((m) => m.repo.includes("/"))).toBe(true);
    // `Settings.engines` is `#[serde(default)]` on the Rust side: whatever the fixture carries must
    // be an EngineSettings, and a fixture without it parses to the defaults.
    expect(engineSettingsSchema.safeParse(engines).success).toBe(true);
    // docs/dictation.md §3 / §10: the provider fields ride along in settings and status.
    expect(PROVIDER_IDS).toContain(engines.asr_provider);
    expect(PROVIDER_IDS).toContain(parsed.engines.asr_provider);
    expect(typeof parsed.engines.local_ready).toBe("boolean");
    expect(Array.isArray(parsed.models)).toBe(true);
    // docs/dictation.md §12 / §13: the output mode rides in settings and status, the activation in
    // settings; Rust serialises all of them unconditionally (`#[serde(default)]` without a skip),
    // so the raw fixture carries the keys — the defaults in the schema are for older cores only.
    expect(OUTPUT_MODES).toContain(engines.output_mode);
    expect(typeof engines.vad_trim).toBe("boolean");
    expect(OUTPUT_MODES).toContain(parsed.engines.effective_output_mode);
    expect(ACTIVATIONS).toContain(parsed.settings.activation);
    const rawSettings = z
      .record(z.string(), z.unknown())
      .parse(Reflect.get(Object(state), "settings"));
    for (const key of ["activation", "hold_threshold_ms", "extra_recording_ms"])
      expect(Object.hasOwn(rawSettings, key)).toBe(true);
    const rawEngines = z.record(z.string(), z.unknown()).parse(rawSettings.engines);
    for (const key of ["output_mode", "vad_trim"])
      expect(Object.hasOwn(rawEngines, key)).toBe(true);
    const rawStatus = z
      .record(z.string(), z.unknown())
      .parse(Reflect.get(Object(state), "engines"));
    expect(Object.hasOwn(rawStatus, "effective_output_mode")).toBe(true);
    // Every history row names its output mode (§12); the fixture's are whole takes.
    expect(parsed.history_recent.map((e) => e.mode)).toEqual(
      parsed.history_recent.map(() => "whole_take"),
    );
    expect(parsed.dictation.phase.phase === "done" && parsed.dictation.phase.mode).toBe(
      "whole_take",
    );
    expect(parsed.relay).toEqual({
      endpoint: "wss://relay.example.test/ws",
      source: "user",
      state: "connected",
      attempts: 2,
    });
    expect(parsed.pairing.state).toEqual({ state: "awaiting_verification" });
    expect(parsed.pairing.safety_code?.words).toEqual(["amber", "canyon", "lantern", "orbit"]);
    expect(parsed.pairing.peer?.platform).toBe("android");
    expect(parsed.devices.map((d) => d.connection)).toEqual([
      { state: "online", via: "relay" },
      { state: "identity_changed", presented_fingerprint: "DE:B0:E3:8C · ED:1E:41:DE" },
    ]);
    expect(parsed.devices[0]?.device.last_connection).toBe("relay");
    expect(parsed.devices[1]?.device.last_seen).toBeUndefined();
  });

  it("state.json is not accepted once a field is wrong or missing", () => {
    const broken: [string, unknown][] = [
      ["bad key", mutate(state, ["identity", "public_key"], "not-hex")],
      ["unknown pairing phase", mutate(state, ["pairing", "state", "state"], "dancing")],
      ["future settings schema", mutate(state, ["settings", "schema"], 2)],
      ["unknown theme", mutate(state, ["settings", "theme"], "neon")],
      ["missing relay", mutate(state, ["relay"], undefined)],
      ["negative attempts", mutate(state, ["relay", "attempts"], -1)],
      ["devices not a list", mutate(state, ["devices"], "none")],
      [
        "safety code with three words",
        mutate(state, ["pairing", "safety_code", "words"], ["a", "b", "c"]),
      ],
    ];
    const verdicts = broken.map(([label, value]) => [
      label,
      uiStateSchema.safeParse(value).success,
    ]);
    expect(verdicts).toEqual(broken.map(([label]) => [label, false]));
  });

  it("events.json parses with the UiEvent schema and covers every variant", () => {
    const results = events.map((raw) => uiEventSchema.safeParse(raw));
    expect(results.map((r) => r.error?.issues ?? "ok")).toEqual(events.map(() => "ok"));
    const parsed = results.map((r) => r.data);
    const seen = new Set<string>(parsed.flatMap((e) => (e ? [e.type] : [])));
    expect(EVENT_TYPES.filter((type) => !seen.has(type))).toEqual([]);
    const failed = parsed.find((e) => e?.type === "pairing" && e.state.state === "failed");
    expect(
      failed && failed.type === "pairing" && failed.state.state === "failed" && failed.state.reason,
    ).toEqual({
      kind: "relay",
      code: "session_expired",
    });
    const devices = parsed.find((e) => e?.type === "devices");
    expect(devices?.type === "devices" && devices.devices.map((d) => d.connection.state)).toEqual([
      "offline",
      "connecting",
      "online",
      "identity_changed",
    ]);
  });

  it("regression: section 12 and 13 dictation fields survive parsing: locked, live.injected, finalizing, done.mode / segments / live_error, cancelled.injected_chars, history.mode, engines.effective_output_mode", () => {
    const parsed = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success ? [r.data] : [];
    });
    const dictation = parsed.flatMap((e) => (e.type === "dictation" ? [e.phase] : []));
    const listening = dictation.flatMap((p) => (p.phase === "listening" ? [p] : []));
    // `locked` is always on the wire: both values appear, none is defaulted.
    expect(listening.map((p) => p.locked)).toContain(true);
    expect(listening.map((p) => p.locked)).toContain(false);
    const rawListening = events.filter(
      (raw) =>
        z.object({ phase: z.object({ phase: z.string() }) }).safeParse(raw).data?.phase.phase ===
        "listening",
    );
    for (const raw of rawListening)
      expect(
        Object.hasOwn(
          z.object({ phase: z.record(z.string(), z.unknown()) }).parse(raw).phase,
          "locked",
        ),
      ).toBe(true);
    // `live.injected` counts pasted sentences (0 and 1 in the fixture).
    const injected = listening.flatMap((p) => (p.live === undefined ? [] : [p.live.injected]));
    expect(injected).toContain(0);
    expect(injected).toContain(1);
    // The streaming modes' `finalizing` stage carries the preview.
    const finalizing = dictation.find((p) => p.phase === "processing" && p.stage === "finalizing");
    expect(finalizing?.phase === "processing" && finalizing.preview).toBe(
      "把 fetchUser 改成 async，然后加上错误",
    );
    // `done` names its mode; a streaming take carries the sentences, a degraded one the reason.
    const done = dictation.flatMap((p) => (p.phase === "done" ? [p] : []));
    expect(new Set(done.map((p) => p.mode))).toEqual(new Set(["whole_take", "streaming_final"]));
    const streamed = done.find((p) => p.mode === "streaming_final");
    expect(streamed?.segments?.map((s) => s.text)).toEqual([
      "把 fetchUser 改成 async，",
      "然后加上错误处理",
    ]);
    expect(done.find((p) => p.live_error !== undefined)?.live_error).toBe(
      "open: asr: 实时识别模型未下载：实时预览",
    );
    // `cancelled.injected_chars`: 0 for every other mode, the pasted count under live_inject.
    const cancelled = dictation.flatMap((p) => (p.phase === "cancelled" ? [p.injected_chars] : []));
    expect(cancelled).toEqual([0, 21]);
    // History rows carry the same three fields.
    const rows = parsed.flatMap((e) => (e.type === "history" ? e.recent : []));
    const live = rows.find((r) => r.mode === "live_inject");
    expect(live?.segments).toHaveLength(2);
    expect(live?.live_error).toBe("live tap overrun: the decoder fell behind the microphone");
    expect(rows.every((r) => OUTPUT_MODES.includes(r.mode))).toBe(true);
    // `engines.effective_output_mode`: every value the core can report.
    const effective = parsed.flatMap((e) =>
      e.type === "engines" ? [e.effective_output_mode] : [],
    );
    expect(new Set(effective)).toEqual(new Set(["whole_take", "streaming_final", "live_inject"]));
    // Mutations of the new fields are refused.
    const cancelledRaw = events.find(
      (raw) =>
        uiEventSchema.safeParse(raw).data?.type === "dictation" &&
        JSON.stringify(raw).includes('"injected_chars":21'),
    );
    expect(
      uiEventSchema.safeParse(mutate(cancelledRaw, ["phase", "injected_chars"], -1)).success,
    ).toBe(false);
    const finalizingRaw = events.find((raw) => JSON.stringify(raw).includes('"finalizing"'));
    expect(
      uiEventSchema.safeParse(mutate(finalizingRaw, ["phase", "stage"], "thinking")).success,
    ).toBe(false);
    const enginesRaw = events.find((raw) => uiEventSchema.safeParse(raw).data?.type === "engines");
    expect(
      uiEventSchema.safeParse(mutate(enginesRaw, ["effective_output_mode"], "batch")).success,
    ).toBe(false);
    const settingsRaw = events.find(
      (raw) => uiEventSchema.safeParse(raw).data?.type === "settings",
    );
    expect(uiEventSchema.safeParse(mutate(settingsRaw, ["activation"], "press")).success).toBe(
      false,
    );
    expect(uiEventSchema.safeParse(mutate(settingsRaw, ["hold_threshold_ms"], -5)).success).toBe(
      false,
    );
  });

  it("regression: the dictionary and rules events and the history hits and chinese_script survive parsing from the Rust fixtures", () => {
    const parsed = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success ? [r.data] : [];
    });
    const dictionaries = parsed.flatMap((e) => (e.type === "dictionary" ? [e.entries] : []));
    expect(dictionaries.map((d) => d.length)).toEqual([2, 0]);
    const [manual, fromHistory] = dictionaries[0] ?? [];
    expect(manual?.source).toEqual({ kind: "manual" });
    expect(manual?.heard_as).toEqual(["fetch user", "费驰优瑟"]);
    expect(fromHistory?.source.kind === "history" && fromHistory.source.history_id).toBe(
      "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d",
    );
    expect(fromHistory?.enabled).toBe(false);
    const ruleLists = parsed.flatMap((e) => (e.type === "rules" ? [e.rules] : []));
    expect(ruleLists.map((r) => r.map((x) => `${x.kind}:${x.name}`))).toEqual([
      ["literal:git push", "regex:PR 编号"],
      [],
    ]);
    expect(ruleLists[0]?.[1]?.pattern).toBe("\\bpr (\\d+)");
    // History rows name what fired; a row where nothing fired has no key at all.
    const rows = parsed.flatMap((e) => (e.type === "history" ? e.recent : []));
    expect(rows.find((r) => r.vocabulary !== undefined)?.vocabulary).toEqual({
      corrections: [{ id: "3b241101-e2bb-4255-8caf-4136c566a962", count: 1 }],
      rules: [{ id: "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", count: 2 }],
    });
    const rawRows = events.flatMap((raw) => {
      const r = z
        .object({ type: z.literal("history"), recent: z.array(z.record(z.string(), z.unknown())) })
        .safeParse(raw);
      return r.success ? r.data.recent : [];
    });
    expect(rawRows.some((r) => !Object.hasOwn(r, "vocabulary"))).toBe(true);
    // `chinese_script` is always serialised in settings (§17).
    const settings = parsed.find((e) => e.type === "settings");
    expect(settings?.type === "settings" && settings.engines.chinese_script).toBe("simplified");
    const rawSettings = z
      .object({ engines: z.record(z.string(), z.unknown()) })
      .parse(events.find((raw) => uiEventSchema.safeParse(raw).data?.type === "settings"));
    expect(rawSettings.engines.chinese_script).toBe("simplified");
    const setEngines = commands.find((c) => c.name === "settings_set_engines");
    expect(argSchemas.settings_set_engines.parse(setEngines?.args).engines.chinese_script).toBe(
      "traditional",
    );
    const dictionaryRaw = events.find(
      (raw) => uiEventSchema.safeParse(raw).data?.type === "dictionary",
    );
    expect(
      uiEventSchema.safeParse(mutate(dictionaryRaw, ["entries", "0", "source", "kind"], "learned"))
        .success,
    ).toBe(false);
  });

  it("regression: the hotkey status carries what the session's hotkey can do", () => {
    const hotkeys = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success && r.data.type === "hotkey" ? [r.data] : [];
    });
    expect(
      hotkeys.map((h) => [
        h.backend.split(" · ").at(-1),
        h.capabilities?.global,
        h.capabilities?.everywhere,
        h.capabilities?.hold,
      ]),
    ).toEqual([
      ["RegisterHotKey", true, true, true],
      ["XWayland", true, false, true],
      ["Wayland", false, false, false],
    ]);
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.hotkey.capabilities?.toggle_command).toBe("voltip-desktop --toggle");
    expect(parsedState.hotkey.capabilities?.edit_toggle_command).toBe(
      "voltip-desktop --edit-toggle",
    );
    const raw = events.find((e) => uiEventSchema.safeParse(e).data?.type === "hotkey");
    expect(uiEventSchema.safeParse(mutate(raw, ["capabilities", "hold"], "yes")).success).toBe(
      false,
    );
  });

  it("regression: LAN discovery survives parsing: the nearby list, the switch, and a tap on a pairing desktop", () => {
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.nearby.map((d) => [d.name, d.pairing, d.trusted])).toEqual([
      ["Studio", true, false],
      ["MacBook Pro", false, true],
    ]);
    const lists = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success && r.data.type === "nearby" ? [r.data.devices.length] : [];
    });
    expect(lists).toEqual([0, 2]);
    // A state without it (an older core) lists nothing and has discovery on.
    const { nearby: _gone, ...older } = z.record(z.string(), z.unknown()).parse(state);
    expect(uiStateSchema.parse(older).nearby).toEqual([]);
    const join = commands.find((c) => c.name === "pairing_join_nearby");
    expect(argSchemas.pairing_join_nearby.parse(join?.args).fingerprint).toBe("A7C4198E3DF26109");
    const off = commands.find((c) => c.name === "settings_set_lan_discovery");
    expect(argSchemas.settings_set_lan_discovery.parse(off?.args).enabled).toBe(false);
  });

  it("regression: always-on pairing survives parsing, and an older settings file reads it off", () => {
    expect(uiStateSchema.parse(state).settings.pairing_always_on).toBe(true);
    const settings = z.record(z.string(), z.unknown()).parse(uiStateSchema.parse(state).settings);
    const { pairing_always_on: _gone, ...older } = settings;
    expect(settingsSchema.parse(older).pairing_always_on).toBe(false);
    const on = commands.find((c) => c.name === "settings_set_pairing_always_on");
    expect(argSchemas.settings_set_pairing_always_on.parse(on?.args).enabled).toBe(true);
  });

  it("regression: section 20.6 the phone's sent texts and a history entry's origin survive parsing", () => {
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.sent_texts.map((t) => [t.id, t.source, t.state.state])).toEqual([
      [5, "clipboard", "sending"],
      [4, "typed", "queued"],
      [3, "typed", "delivered"],
      [2, "clipboard", "delivered"],
      [1, "typed", "failed"],
    ]);
    const failed = parsedState.sent_texts.at(-1)?.state;
    expect(failed?.state === "failed" ? failed.code : undefined).toBe("no_answer");
    expect(parsedState.history_recent.at(-1)?.origin).toEqual({ device: "Pixel 8", kind: "typed" });
    expect(parsedState.history_recent[0]?.origin).toBeUndefined();
    const lists = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success && r.data.type === "sent_texts" ? [r.data.texts.length] : [];
    });
    expect(lists).toEqual([0, 5]);
    // A state without the list (the desktop, an older phone) reads as empty.
    const { sent_texts: _gone, ...older } = z.record(z.string(), z.unknown()).parse(state);
    expect(uiStateSchema.parse(older).sent_texts).toEqual([]);
    const raw = events.find(
      (e) =>
        uiEventSchema.safeParse(e).data?.type === "sent_texts" &&
        JSON.stringify(e).includes("no_answer"),
    );
    expect(uiEventSchema.safeParse(mutate(raw, ["texts", "0", "source"], "voice")).success).toBe(
      false,
    );
    const send = commands.find((c) => c.name === "phone_text_send");
    expect(argSchemas.phone_text_send.parse(send?.args).source).toBe("typed");
  });

  it("regression: section 13.1 the lone-key trigger survives parsing: the setting, what the hook watches and why not, and the session's keys", () => {
    const hotkeys = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success && r.data.type === "hotkey" ? [r.data] : [];
    });
    expect(
      hotkeys.map((h) => [
        h.solo_registered,
        h.solo_error !== undefined,
        h.solo_pressed,
        h.capabilities?.solo_keys.length,
      ]),
    ).toEqual([
      ["mouse_back", false, true, 7],
      ["right_alt", false, false, 7],
      [undefined, true, false, 0],
    ]);
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.settings.solo_key).toBe("right_ctrl");
    expect(parsedState.hotkey.solo_registered).toBe("right_ctrl");
    expect(parsedState.hotkey.capabilities?.solo_keys).not.toContain("fn");
    // An older core sends neither the setting nor the status fields: off, nothing watched.
    const { solo_key: _dropped, ...olderSettings } = z
      .record(z.string(), z.unknown())
      .parse(z.record(z.string(), z.unknown()).parse(state).settings);
    const older = uiStateSchema.parse({
      ...z.record(z.string(), z.unknown()).parse(state),
      settings: olderSettings,
    });
    expect(older.settings.solo_key).toBeNull();
    const raw = events.find((e) => uiEventSchema.safeParse(e).data?.type === "hotkey");
    expect(uiEventSchema.safeParse(mutate(raw, ["solo_registered"], "caps_lock")).success).toBe(
      false,
    );
    const setSolo = commands.find((c) => c.name === "settings_set_solo_key");
    expect(argSchemas.settings_set_solo_key.parse(setSolo?.args)).toEqual({ key: "mouse_back" });
    expect(argSchemas.hotkey_edge.parse({ pressed: false, chorded: true }).chorded).toBe(true);
  });

  it("regression: section 19 voice edit fields survive parsing: the edit hotkey and the take kind and the edit record and the four failure codes and the edit registration and the edge purpose", () => {
    const parsed = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success ? [r.data] : [];
    });
    // `edit_hotkey` is always serialised: the chord, and `null` once switched off.
    const editHotkeys = parsed.flatMap((e) => (e.type === "settings" ? [e.edit_hotkey] : []));
    expect(editHotkeys).toEqual(["Ctrl+Alt+E", null]);
    const rawSettings = events.filter(
      (raw) => uiEventSchema.safeParse(raw).data?.type === "settings",
    );
    for (const raw of rawSettings)
      expect(Object.hasOwn(z.record(z.string(), z.unknown()).parse(raw), "edit_hotkey")).toBe(true);
    // `kind` rides on every dictation event (never defaulted) and names both kinds.
    const kinds = parsed.flatMap((e) => (e.type === "dictation" ? [e.kind] : []));
    expect(new Set(kinds)).toEqual(new Set(["dictation", "edit"]));
    const rawDictation = events.filter(
      (raw) => uiEventSchema.safeParse(raw).data?.type === "dictation",
    );
    for (const raw of rawDictation)
      expect(Object.hasOwn(z.record(z.string(), z.unknown()).parse(raw), "kind")).toBe(true);
    const edits = parsed.flatMap((e) =>
      e.type === "dictation" && e.kind === "edit" ? [e.phase] : [],
    );
    expect(edits.map((p) => p.phase)).toEqual([
      "listening",
      "processing",
      "done",
      "failed",
      "failed",
      "failed",
      "failed",
      "failed",
    ]);
    expect(edits.flatMap((p) => (p.phase === "failed" ? [p.code] : []))).toEqual([
      "no_selection",
      "selection_too_long",
      "selection",
      "edit_unavailable",
      "edit_in_terminal",
    ]);
    // History: an edit row carries the instruction and the original; the other rows are dictations.
    const rows = parsed.flatMap((e) => (e.type === "history" ? e.recent : []));
    const edit = rows.find((r) => r.kind === "edit");
    expect(edit?.edit).toEqual({
      instruction: "改得更正式",
      selection: "大家好，会议改到周四十点哈",
    });
    // An edit names the app it ran in, never a scene (§19 with §18.6).
    expect([edit?.app, edit?.scene]).toEqual([{ id: "slack", name: "Slack" }, undefined]);
    expect(rows.filter((r) => r.kind === "dictation").every((r) => r.edit === undefined)).toBe(
      true,
    );
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.dictation.kind).toBe("dictation");
    expect(parsedState.hotkey.edit_registered).toBe("Ctrl+Alt+E");
    expect(parsedState.history_recent.map((r) => r.kind)).toEqual([
      "dictation",
      "dictation",
      "edit",
      "dictation",
    ]);
    const hotkeys = parsed.flatMap((e) => (e.type === "hotkey" ? [e] : []));
    expect(hotkeys.map((h) => h.edit_registered ?? h.edit_error)).toEqual([
      "Ctrl+Alt+E",
      "Ctrl+Alt+E 注册失败：already registered",
      undefined,
    ]);
    // Commands: the edit chord and the edit key's edge.
    const setEdit = commands.find((c) => c.name === "settings_set_edit_hotkey");
    expect(argSchemas.settings_set_edit_hotkey.parse(setEdit?.args)).toEqual({
      hotkey: "Ctrl+Alt+Shift+E",
    });
    const edge = commands.find((c) => c.name === "hotkey_edge");
    expect(argSchemas.hotkey_edge.parse(edge?.args).purpose).toBe("edit");
    // Mutations of the new fields are refused.
    const editRaw = events.find(
      (raw) =>
        uiEventSchema.safeParse(raw).data?.type === "dictation" &&
        JSON.stringify(raw).includes('"kind":"edit"'),
    );
    expect(uiEventSchema.safeParse(mutate(editRaw, ["kind"], "rewrite")).success).toBe(false);
    expect(uiEventSchema.safeParse(mutate(rawSettings[0], ["edit_hotkey"], 5)).success).toBe(false);
    const historyRaw = events.find((raw) => JSON.stringify(raw).includes('"kind":"edit","edit"'));
    expect(historyRaw).toBeDefined();
    expect(
      uiEventSchema.safeParse(mutate(historyRaw, ["recent", "2", "edit", "instruction"], undefined))
        .success,
    ).toBe(false);
    expect(argSchemas.hotkey_edge.safeParse({ pressed: true, purpose: "rewrite" }).success).toBe(
      false,
    );
  });

  it("regression: the scenes events, the take context, the history app and scene and the context switches survive parsing from the Rust fixtures", () => {
    const parsedState = uiStateSchema.parse(state);
    expect(parsedState.scenes.map((s) => s.name)).toEqual(["代码评审", "聊天", "legal"]);
    const [review, chat, legal] = parsedState.scenes;
    // docs/dictation.md §18.10: a built-in scene carries its category and may list no application.
    expect(legal?.builtin).toBe("legal");
    expect(legal?.match).toEqual({ apps: [], title_contains: [] });
    expect(legal?.overrides.refine_preset).toBe("proofread");
    expect(review?.builtin).toBeUndefined();
    expect(review?.match).toEqual({ apps: ["chrome", "code"], title_contains: ["Pull request"] });
    expect(review?.overrides).toEqual({
      refine_enabled: true,
      refine_preset: "formal",
      output_mode: "streaming_final",
      language: "en",
      chinese_script: "as_is",
      prompt: "这是代码评审意见：保留代码标识符原样。",
    });
    // Unset overrides are absent on the wire (the core skips them), not `null`.
    expect(chat?.overrides).toEqual({ refine_preset: "punctuation" });
    expect(chat?.enabled).toBe(false);
    expect(parsedState.settings.context_sharing).toEqual({ app_name: false, window_title: true });
    expect(parsedState.dictation.context).toEqual({
      app: { id: "code", name: "Code" },
      scene: { id: review?.id, name: "代码评审" },
    });
    expect(parsedState.history_recent[0]?.app).toEqual({ id: "code", name: "Code" });
    expect(parsedState.history_recent[0]?.scene?.name).toBe("代码评审");
    expect(parsedState.history_recent[1]?.app).toBeUndefined();
    const parsed = events.flatMap((raw) => {
      const r = uiEventSchema.safeParse(raw);
      return r.success ? [r.data] : [];
    });
    const lists = parsed.flatMap((e) => (e.type === "scenes" ? [e.scenes] : []));
    expect(lists.map((l) => l.length)).toEqual([3, 0]);
    const builtinRefs = parsed.flatMap((e) =>
      e.type === "history"
        ? e.recent.flatMap((h) => (h.scene?.builtin === undefined ? [] : [h.scene]))
        : [],
    );
    expect(builtinRefs).toContainEqual({ id: legal?.id, name: "legal", builtin: "legal" });
    const contexts = parsed.flatMap((e) =>
      e.type === "dictation" && e.context !== undefined ? [e.context] : [],
    );
    expect(contexts.map((c) => c.scene?.name ?? `(${c.app.id})`)).toEqual([
      "代码评审",
      "(winword)",
      "(slack)",
    ]);
    // The command samples parse with the draft schema and carry every override key.
    const add = commands.find((c) => c.name === "scenes_add");
    const overrideKeys = Object.keys(argSchemas.scenes_add.parse(add?.args).scene.overrides);
    overrideKeys.sort();
    expect(overrideKeys).toEqual([
      "chinese_script",
      "language",
      "output_mode",
      "prompt",
      "refine_enabled",
      "refine_preset",
    ]);
    const sharing = commands.find((c) => c.name === "settings_set_context_sharing");
    expect(argSchemas.settings_set_context_sharing.parse(sharing?.args)).toEqual({
      appName: true,
      windowTitle: false,
    });
    // Mutations of the new fields are refused.
    const scenesRaw = events.find((raw) => uiEventSchema.safeParse(raw).data?.type === "scenes");
    expect(
      uiEventSchema.safeParse(
        mutate(scenesRaw, ["scenes", "0", "overrides", "refine_preset"], "casual"),
      ).success,
    ).toBe(false);
    expect(
      uiEventSchema.safeParse(mutate(scenesRaw, ["scenes", "0", "match", "apps"], "chrome"))
        .success,
    ).toBe(false);
    const contextRaw = events.find((raw) => JSON.stringify(raw).includes('"context":{"app"'));
    expect(uiEventSchema.safeParse(mutate(contextRaw, ["context", "app"], undefined)).success).toBe(
      false,
    );
  });

  it("regression: the TypeScript provider catalogue matches the Rust one card for card", () => {
    const fixture = uiStateSchema.parse(loadFixture("state.json"));
    // The sample build compiles the built-in service in, so every provider is listed. The built-in
    // card's presets are the build's own model, not catalogue data, so they are left out.
    const services = (id: string, service?: { presets: string[]; default_base_url?: string }) =>
      service === undefined
        ? undefined
        : {
            presets: id === "builtin" ? [] : service.presets,
            base: service.default_base_url ?? "",
          };
    const fromRust = fixture.engines.providers.map((c) => ({
      id: c.id,
      key: c.key,
      onDevice: c.on_device,
      console: c.console,
      asr: services(c.id, c.asr),
      llm: services(c.id, c.llm),
    }));
    const preset = (p?: { baseUrl: string; models: readonly string[] }) =>
      p === undefined ? undefined : { presets: [...p.models], base: p.baseUrl };
    const fromTs = PROVIDER_CATALOGUE.map((p) => ({
      id: p.id,
      key: p.key,
      onDevice: p.onDevice,
      console: p.console,
      asr: preset(p.asr),
      llm: preset(p.llm),
    }));
    expect(fromRust).toEqual(fromTs);
  });

  it("regression: the preview's resolution reproduces the core's engine status from the same inputs", () => {
    // The inputs `contract.rs` resolved `state.engines` from: the sample settings, a Groq key, the
    // built-in service with both keys, the default local model installed, the live preview ready.
    const fixture = uiStateSchema.parse(loadFixture("state.json"));
    const local = fixture.models.find((m) => m.id === "qwen3-asr-0.6b");
    expect(local).toBeDefined();
    const resolved = resolveEngineStatus({
      settings: fixture.settings.engines,
      userKeys: new Set(["provider-key.groq"]),
      builtIn: {
        asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true },
        llm: { model: "qwen/qwen3.8-27b", key: true },
      },
      local: { id: "qwen3-asr-0.6b", name: local?.name ?? "", installed: true },
      liveReady: true,
    });
    expect(resolved).toEqual(fixture.engines);
  });

  it("events.json entries are rejected once mutated", () => {
    const retagged = events.map(
      (raw) => uiEventSchema.safeParse(mutate(raw, ["type"], "bogus")).success,
    );
    expect(retagged).toEqual(events.map(() => false));
    const devices = events.find((e) => uiEventSchema.safeParse(e).data?.type === "devices");
    expect(uiEventSchema.safeParse(mutate(devices, ["devices"], "not-a-list")).success).toBe(false);
    const message = events.find((e) => uiEventSchema.safeParse(e).data?.type === "message");
    expect(uiEventSchema.safeParse(mutate(message, ["body"], undefined)).success).toBe(false);
    const identity = events.find((e) => uiEventSchema.safeParse(e).data?.type === "identity");
    expect(uiEventSchema.safeParse(mutate(identity, ["platform"], "amiga")).success).toBe(false);
  });

  it("commands.json names exactly the mutation commands the UI can dispatch", () => {
    const names = commands.map((c) => c.name);
    expect(new Set(names)).toEqual(new Set(MUTATION_COMMANDS));
    expect(names).toHaveLength(MUTATION_COMMANDS.length);
  });

  it("TauriBackend sends each command with the name and args the Rust side parses", async () => {
    const recorded: { command: string; args: Record<string, unknown> | undefined }[] = [];
    const backend = new TauriBackend({
      invoke: (command, args) => {
        recorded.push({ command, args });
        return Promise.resolve(command === "core_state" ? state : null);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    for (const entry of commands) await replay(backend, entry.name, entry.args);
    expect(recorded).toStrictEqual(
      commands.map((entry) => ({ command: entry.name, args: entry.args ?? undefined })),
    );
    // core_state is the one non-mutation command: the fixture state is what it returns.
    const viaBackend = await backend.getState();
    expect(viaBackend).toEqual(uiStateSchema.parse(state));
    expect(recorded.at(-1)).toStrictEqual({ command: "core_state", args: undefined });
  });
});

describe("history queries (docs/dictation.md section 4.4)", () => {
  it("the answers the Rust side writes parse with the schemas", () => {
    const fixture = z.record(z.string(), z.unknown()).parse(loadFixture("history-queries.json"));
    const page = historyPageSchema.parse(fixture.page);
    expect([page.entries.length, page.matching, page.total]).toEqual([1, 12, 312]);
    expect(historyEntrySchema.parse(fixture.entry).id).toBe(page.entries[0]?.id);
    expect(historyEntrySchema.nullable().parse(fixture.missing)).toBeNull();
    const stats = historyStatsSchema.parse(fixture.stats);
    expect(stats.buckets.map((b) => b.count)).toEqual([0, 2]);
    expect(stats.total).toEqual({
      count: 6,
      raw_chars: 348,
      corrected_chars: 42,
      spoken_ms: 66_000,
      latency_ms: 7386,
    });
    const hits = historyHitsSchema.parse(fixture.hits);
    expect(Object.values(hits.dictionary)).toEqual([3]);
    expect(Object.values(hits.rules)).toEqual([1]);
    // A bucket without a field, or a negative count, is refused rather than drawn.
    expect(historyStatsBucketSchema.safeParse({ count: -1 }).success).toBe(false);
  });

  it("TauriBackend sends the four queries with the arguments the shell's commands take", async () => {
    const fixture = z.record(z.string(), z.unknown()).parse(loadFixture("history-queries.json"));
    const answers: Record<string, unknown> = {
      history_query: fixture.page,
      history_entry: fixture.missing,
      history_stats: fixture.stats,
      history_hits: fixture.hits,
    };
    const calls: { command: string; args: unknown }[] = [];
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answers[command]);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    expect(
      (await backend.historyQuery({ sinceMs: 5, failed: true, query: "会议", limit: 100 })).total,
    ).toBe(312);
    expect(await backend.historyEntry("9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d")).toBeNull();
    expect((await backend.historyStats([1, 2, 3])).buckets).toHaveLength(2);
    expect((await backend.historyHits()).rules).toEqual(hits(fixture.hits).rules);
    expect(calls).toStrictEqual([
      { command: "history_query", args: { sinceMs: 5, failed: true, query: "会议", limit: 100 } },
      { command: "history_entry", args: { id: "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d" } },
      { command: "history_stats", args: { boundaries: [1, 2, 3] } },
      { command: "history_hits", args: undefined },
    ]);
    function hits(raw: unknown) {
      return historyHitsSchema.parse(raw);
    }
  });

  it("the built-in scenes' names in both dictionaries are the ones the core's search finds", () => {
    const rows = z
      .array(
        z.object({
          id: builtinSceneSchema,
          names: z.object({ "zh-CN": z.string(), en: z.string() }),
        }),
      )
      .parse(loadFixture("scenes-builtin.json"));
    expect(rows).toHaveLength(7);
    for (const row of rows) {
      expect(translate("zh-CN", `builtinScenes.${row.id}.name`)).toBe(row.names["zh-CN"]);
      expect(translate("en", `builtinScenes.${row.id}.name`)).toBe(row.names.en);
    }
  });
});
