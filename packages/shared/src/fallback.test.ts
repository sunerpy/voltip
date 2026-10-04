import { describe, expect, it } from "vitest";
import {
  fallbackAddProblem,
  fallbackRows,
  fallbackSettingsOf,
  fallbackStatusOf,
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
} from "./schema";

const entry = (
  provider: FallbackModel["provider"],
  model: string,
): FallbackModel => ({
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
  });
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
    expect(
      engineSettingsSchema.safeParse({ asr_fallback: { models: tooMany } })
        .success,
    ).toBe(false);
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
    expect(
      fallbackStatusOf(resolve({ ...studio(), asr_provider: "local" }), "asr")
        .in_use,
    ).toBe(false);
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
    const models = asr.models.map((m, i) =>
      i === 0 ? { ...m, retry_at_ms: 2_000 } : m,
    );
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
    const list = [
      entry("aliyun", "a"),
      entry("builtin", ""),
      entry("groq", "b"),
    ];
    expect(fallbackAddProblem(list, entry("aliyun", " a "))).toBe("listed");
    expect(fallbackAddProblem(list, entry("builtin", "x"))).toBe("listed");
    expect(fallbackAddProblem(list, entry("groq", "  "))).toBe("blank");
    expect(fallbackAddProblem([], entry("builtin", ""))).toBeUndefined();
    expect(fallbackAddProblem(list, entry("aliyun", "c"))).toBeUndefined();
    const full = Array.from({ length: MAX_FALLBACK_MODELS }, (_, i) =>
      entry("groq", `m${i}`),
    );
    expect(fallbackAddProblem(full, entry("aliyun", "c"))).toBe("full");
    expect(moveFallback(list, 0, 1).map((m) => m.model)).toEqual([
      "",
      "a",
      "b",
    ]);
    expect(moveFallback(list, 2, -1).map((m) => m.model)).toEqual([
      "a",
      "b",
      "",
    ]);
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

  it("the preview takes the lists and answers 重新检查 with the engines' status", async () => {
    const backend = new MockBackend();
    const seen: string[] = [];
    const stop = backend.on((event) => {
      if (event.type === "engines")
        seen.push(JSON.stringify(event.asr_fallback ?? null));
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
