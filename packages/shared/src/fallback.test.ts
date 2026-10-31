import { describe, expect, it } from "vitest";
import {
  fallbackAddProblem,
  fallbackOnOutage,
  fallbackRows,
  fallbackSettingsOf,
  fallbackStatusOf,
  localFallbackModels,
  moveFallback,
  withFallback,
} from "./fallback";
import { MockBackend } from "./mock-backend";
import { resolveEngineStatus } from "./providers";
import {
  defaultEngineSettings,
  emptyEngineStatus,
  engineSettingsSchema,
  engineStatusSchema,
  type EngineSettings,
  type EngineStatus,
  type FallbackModel,
  MAX_FALLBACK_MODELS,
  type ModelState,
} from "./schema";

const entry = (provider: FallbackModel["provider"], model: string): FallbackModel => ({
  provider,
  model,
});

/** Model Studio selected for recognition, with five fallback entries of every kind. */
function studio(): EngineSettings {
  return {
    ...defaultEngineSettings(),
    asr_provider: "aliyun",
    providers: { aliyun: { asr_model: "qwen-audio-3.1-asr-flash-streaming" } },
    asr_fallback: {
      enabled: true,
      models: [
        entry("aliyun", "qwen-audio-3.1-asr-flash-message"),
        entry("aliyun", " qwen-audio-3.1-asr-flash-streaming "),
        entry("openai", "whisper-1"),
        entry("aliyun", "qwen-audio-3.1-asr-flash-message"),
        entry("local", "qwen3-asr-0.6b"),
      ],
    },
  };
}

function resolve(
  settings: EngineSettings,
  keys: string[] = ["provider-key.aliyun"],
  quotaOut?: { asr?: number; llm?: number },
  library?: ModelState[],
): EngineStatus {
  return resolveEngineStatus({
    settings,
    userKeys: new Set(keys),
    builtIn: {
      asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true },
      llm: { model: "qwen/qwen3.8-27b", key: true },
    },
    local: { id: "qwen3-asr-0.6b", name: "Qwen3-ASR 0.6B", installed: true },
    liveReady: false,
    ...(quotaOut === undefined ? {} : { quotaOut }),
    ...(library === undefined ? {} : { library }),
  });
}

/** A library entry of the fake catalogue: `id`, installed or not, recognising whole takes or streaming. */
function libraryModel(
  id: string,
  installed: boolean,
  capability: "offline" | "streaming" = "offline",
): ModelState {
  return {
    id,
    name: id,
    engine: "paraformer",
    tier: "light",
    capabilities: [capability],
    languages: ["zh"],
    size_bytes: 1,
    description: "",
    recommended: false,
    repo: "",
    dir: "",
    files: [],
    active: false,
    state: installed
      ? { kind: "installed", path: `/models/${id}`, installed_at: 1 }
      : { kind: "not_installed" },
  };
}

describe("fallback models (docs/dictation.md §3.5)", () => {
  it("settings and status default to off with nothing listed", () => {
    const settings = engineSettingsSchema.parse({});
    expect(settings.asr_fallback).toBeUndefined();
    expect(fallbackSettingsOf(settings, "asr")).toEqual({
      enabled: false,
      models: [],
    });
    expect(fallbackStatusOf(emptyEngineStatus(), "llm")).toEqual({
      enabled: false,
      in_use: false,
      models: [],
    });
    const parsed = engineSettingsSchema.parse({
      llm_fallback: { enabled: true, models: [{ provider: "builtin" }] },
    });
    expect(fallbackSettingsOf(parsed, "llm")).toEqual({
      enabled: true,
      models: [{ provider: "builtin", model: "" }],
    });
    const tooMany = Array.from({ length: MAX_FALLBACK_MODELS + 1 }, (_, i) =>
      entry("groq", `m${i}`),
    );
    expect(engineSettingsSchema.safeParse({ asr_fallback: { models: tooMany } }).success).toBe(
      false,
    );
    const status = engineStatusSchema.parse({
      asr_fallback: {
        enabled: true,
        in_use: true,
        models: [{ provider: "groq", model: "m", retry_at_ms: 5 }],
      },
    });
    expect(status.asr_fallback?.models[0]?.retry_at_ms).toBe(5);
  });

  it("the preview resolves every entry as the core does: issues, skips and whether the chain runs", () => {
    const status = resolve(studio());
    const asr = fallbackStatusOf(status, "asr");
    expect(asr.enabled && asr.in_use).toBe(true);
    expect(asr.models).toEqual([
      { provider: "aliyun", model: "qwen-audio-3.1-asr-flash-message" },
      {
        provider: "aliyun",
        model: "qwen-audio-3.1-asr-flash-streaming",
        skip: "same_as_selected",
      },
      { provider: "openai", model: "whisper-1", issue: "key_missing" },
      {
        provider: "aliyun",
        model: "qwen-audio-3.1-asr-flash-message",
        skip: "duplicate",
      },
      { provider: "local", model: "qwen3-asr-0.6b", issue: "unavailable" },
    ]);
    // On-device recognition never runs out: the chain does not run; nor without the key.
    expect(fallbackStatusOf(resolve({ ...studio(), asr_provider: "local" }), "asr").in_use).toBe(
      false,
    );
    expect(fallbackStatusOf(resolve(studio(), []), "asr").in_use).toBe(false);
    const off = withFallback(studio(), "asr", {
      ...fallbackSettingsOf(studio(), "asr"),
      enabled: false,
    });
    expect(fallbackStatusOf(resolve(off), "asr")).toMatchObject({
      enabled: false,
      in_use: false,
    });
  });

  it("the rows: the selected model first, the first with quota left in use", () => {
    const status = resolve(studio());
    const { rows, active } = fallbackRows(status, "asr");
    expect(rows.map((r) => [r.index, r.state])).toEqual([
      ["selected", "active"],
      [0, "ready"],
      [1, "same"],
      [2, "issue"],
      [3, "duplicate"],
      [4, "issue"],
    ]);
    expect(active?.index).toBe("selected");
    // The selected model ran out: the next one with quota left takes over.
    const asr = fallbackStatusOf(status, "asr");
    const ranOut: EngineStatus = {
      ...status,
      asr_fallback: { ...asr, selected_retry_at_ms: 1_000 },
    };
    const after = fallbackRows(ranOut, "asr");
    expect(after.rows[0]).toMatchObject({
      state: "exhausted",
      retryAtMs: 1_000,
    });
    expect(after.active).toMatchObject({
      index: 0,
      model: "qwen-audio-3.1-asr-flash-message",
      state: "active",
    });
    // Every model ran out: nothing is in use (the core asks the selected one again).
    const models = asr.models.map((m, i) => (i === 0 ? { ...m, retry_at_ms: 2_000 } : m));
    const allOut: EngineStatus = {
      ...status,
      asr_fallback: { ...asr, selected_retry_at_ms: 1_000, models },
    };
    expect(fallbackRows(allOut, "asr").active).toBeUndefined();
    expect(fallbackRows(allOut, "asr").rows[1]).toMatchObject({
      state: "exhausted",
      retryAtMs: 2_000,
    });
    // The chain does not run: no row is marked in use, retry times are not shown.
    const idle: EngineStatus = {
      ...ranOut,
      asr_fallback: { ...asr, in_use: false, selected_retry_at_ms: 1_000 },
    };
    expect(fallbackRows(idle, "asr").active).toBeUndefined();
    expect(fallbackRows(idle, "asr").rows[0]?.state).toBe("ready");
    // No clean-up provider: no selected row.
    expect(fallbackRows({ ...emptyEngineStatus() }, "llm").rows).toEqual([]);
  });

  it("list edits: what may be added, and moves within the list", () => {
    const list = [entry("aliyun", "a"), entry("builtin", ""), entry("groq", "b")];
    expect(fallbackAddProblem(list, entry("aliyun", " a "))).toBe("listed");
    // The built-in service's models are told apart like any other's (user request 2026-10-08:
    // it may offer several); the empty one is the model its card uses.
    expect(fallbackAddProblem(list, entry("builtin", " "))).toBe("listed");
    expect(fallbackAddProblem(list, entry("builtin", "openai/gpt-oss-20b"))).toBeUndefined();
    expect(fallbackAddProblem(list, entry("groq", "  "))).toBe("blank");
    expect(fallbackAddProblem([], entry("builtin", ""))).toBeUndefined();
    expect(fallbackAddProblem(list, entry("aliyun", "c"))).toBeUndefined();
    const full = Array.from({ length: MAX_FALLBACK_MODELS }, (_, i) => entry("groq", `m${i}`));
    expect(fallbackAddProblem(full, entry("aliyun", "c"))).toBe("full");
    expect(moveFallback(list, 0, 1).map((m) => m.model)).toEqual(["", "a", "b"]);
    expect(moveFallback(list, 2, -1).map((m) => m.model)).toEqual(["a", "b", ""]);
    expect(moveFallback(list, 0, -1)).toEqual(list);
    expect(moveFallback(list, 2, 1)).toEqual(list);
    expect(moveFallback(list, 7, 1)).toEqual(list);
    const settings = withFallback(defaultEngineSettings(), "llm", {
      enabled: true,
      models: list,
    });
    expect(settings.llm_fallback?.models).toHaveLength(3);
    expect(settings.asr_fallback).toBeUndefined();
  });

  it("regression: the preview's live state and output mode follow the model standing in for a used-up realtime one", () => {
    // Goal review 2026-10-04, as `ResolvedEngines::status_with`: the selected model's retry time
    // while the chain runs; a stand-in taking whole recordings only leaves the takes whole.
    const shown = (s: EngineStatus) => [
      s.live_source,
      s.live_preview_ready,
      s.effective_output_mode,
      s.asr_fallback?.selected_retry_at_ms,
    ];
    const realtimeFirst = studio();
    expect(shown(resolve(realtimeFirst))).toEqual(["stream", true, "streaming_final", undefined]);
    expect(shown(resolve(realtimeFirst, undefined, { asr: 99 }))).toEqual([
      "stream",
      true,
      "streaming_final",
      99,
    ]);
    const wholeFirst: EngineSettings = {
      ...realtimeFirst,
      asr_fallback: { enabled: true, models: [entry("aliyun", "qwen-audio-3.1-asr-flash")] },
    };
    expect(shown(resolve(wholeFirst, undefined, { asr: 99 }))).toEqual([
      "stream",
      false,
      "whole_take",
      99,
    ]);
    const live: EngineSettings = { ...wholeFirst, output_mode: "live_inject" };
    expect(resolve(live, undefined, { asr: 99 }).effective_output_mode).toBe("whole_take");
    expect(resolve(live).effective_output_mode).toBe("live_inject");
    const off: EngineSettings = {
      ...wholeFirst,
      asr_fallback: { enabled: false, models: wholeFirst.asr_fallback?.models ?? [] },
    };
    expect(shown(resolve(off, undefined, { asr: 99 }))).toEqual([
      "stream",
      true,
      "streaming_final",
      undefined,
    ]);
  });

  it("models of this device join a recognition chain and a local primary never moves on to a service (§3.14)", () => {
    const library = [
      libraryModel("qwen3-asr-0.6b", true),
      libraryModel("paraformer-zh", true),
      libraryModel("sense-voice", false),
      libraryModel("zipformer-stream-zh-en", true, "streaming"),
    ];
    expect(localFallbackModels(library).map((m) => m.id)).toEqual([
      "qwen3-asr-0.6b",
      "paraformer-zh",
    ]);
    const listed = [
      entry("aliyun", "qwen-audio-3.1-asr-flash-message"),
      entry("local", "paraformer-zh"),
      entry("local", "sense-voice"),
      entry("local", "zipformer-stream-zh-en"),
    ];
    // A service selected: the installed model of this device is ready, the others say why not.
    const cloud = resolve(
      { ...studio(), asr_fallback: { enabled: true, models: listed } },
      undefined,
      undefined,
      library,
    );
    expect(fallbackStatusOf(cloud, "asr").models.map((m) => m.skip ?? m.issue ?? "ready")).toEqual([
      "ready",
      "ready",
      "model_not_installed",
      "unavailable",
    ]);
    expect(fallbackStatusOf(cloud, "asr").in_use).toBe(true);
    // Recognition on this device: every service is left out; the chain runs only with the switch.
    const onDevice = (on_outage: boolean): EngineSettings => ({
      ...studio(),
      asr_provider: "local",
      asr_fallback: { enabled: true, on_outage, models: listed },
    });
    const local = resolve(onDevice(true), undefined, undefined, library);
    const asr = fallbackStatusOf(local, "asr");
    expect(asr.models.map((m) => m.skip ?? m.issue ?? "ready")).toEqual([
      "not_for_local",
      "ready",
      "model_not_installed",
      "unavailable",
    ]);
    expect(asr.in_use).toBe(true);
    expect(fallbackRows(local, "asr").rows.map((r) => r.state)).toEqual([
      "active",
      "not_for_local",
      "ready",
      "issue",
      "issue",
    ]);
    expect(
      fallbackStatusOf(resolve(onDevice(false), undefined, undefined, library), "asr").in_use,
    ).toBe(false);
    expect([
      fallbackOnOutage(onDevice(true), "asr"),
      fallbackOnOutage(onDevice(false), "asr"),
      fallbackOnOutage(studio(), "llm"),
    ]).toEqual([true, false, false]);
  });

  it("a model that failed shows as skipped for now, apart from one out of quota (§3.14)", () => {
    const status = resolve(studio());
    const asr = fallbackStatusOf(status, "asr");
    const models = asr.models.map((m, i) =>
      i === 0 ? { ...m, retry_at_ms: 2_000, retry_reason: "quota" as const } : m,
    );
    const failed: EngineStatus = {
      ...status,
      asr_fallback: {
        ...asr,
        selected_retry_at_ms: 1_000,
        selected_retry_reason: "outage",
        models,
      },
    };
    const { rows, active } = fallbackRows(failed, "asr");
    expect(rows.slice(0, 2).map((r) => [r.state, r.retryAtMs])).toEqual([
      ["cooling", 1_000],
      ["exhausted", 2_000],
    ]);
    expect(active).toBeUndefined();
    // An older core says no reason: out of quota, as before.
    const older: EngineStatus = {
      ...status,
      asr_fallback: { ...asr, selected_retry_at_ms: 1_000 },
    };
    expect(fallbackRows(older, "asr").rows[0]?.state).toBe("exhausted");
  });

  it("the preview takes the lists and answers 重新检查 with the engines' status", async () => {
    const backend = new MockBackend();
    const seen: string[] = [];
    const stop = backend.on((event) => {
      if (event.type === "engines") seen.push(JSON.stringify(event.asr_fallback ?? null));
    });
    await backend.invoke("provider_key_set", {
      provider: "aliyun",
      kind: "asr",
      value: "sk-example",
    });
    await backend.invoke("settings_set_engines", { engines: studio() });
    await backend.invoke("engines_quota_reset", { kind: "asr" });
    expect(seen.length).toBeGreaterThanOrEqual(2);
    expect(seen.at(-1)).toContain("same_as_selected");
    stop();
  });
});
