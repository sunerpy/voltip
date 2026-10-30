import { createTranslator, defaultEngineSettings } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { importOptions, polishOf, recognitionOf } from "./import-options";

const { t } = createTranslator("zh-CN");

describe("import options (docs/dictation.md §31)", () => {
  it("offers the user's own providers and the installed local models, never the built-in service", () => {
    const backend = new MockBackend({
      providerKeys: [{ provider: "groq", kind: "asr" }],
      models: { "sense-voice-small": { kind: "installed", path: "/m", installed_at: 1 } },
    });
    const { engines, models } = backend.peek();
    // The settings use the built-in service: it is what the menus check, and it is left out.
    expect(engines.asr_provider).toBe("builtin");
    const options = importOptions(engines, models, t, "zh-CN");
    const values = options.speech.map((o) => o.value);
    expect(values.some((v) => v.startsWith("remote:builtin:"))).toBe(false);
    expect(values).toContain("local:sense-voice-small");
    expect(values.filter((v) => v.startsWith("remote:groq:")).length).toBeGreaterThan(0);
    // Each row says whose model it is; the list starts on its first row.
    expect(options.speech[0]?.label.startsWith("Groq · ")).toBe(true);
    expect(options.speech.find((o) => o.value === "local:sense-voice-small")?.label).toMatch(
      /^本地模型 · /,
    );
    expect(options.speechDefault).toBe(values[0]);
    // The Groq key serves its AI models too; the polish list has no built-in row either.
    expect(options.polish.length).toBeGreaterThan(0);
    expect(options.polish.every((o) => o.value.startsWith("remote:groq:"))).toBe(true);
  });

  it("starts on the model in use when it may be used, and has nothing without a model", () => {
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const engines = {
      ...backend.peek().engines,
      asr_provider: "groq" as const,
      providers: backend
        .peek()
        .engines.providers.map((p) =>
          p.id === "groq" && p.asr !== undefined ? { ...p, asr: { ...p.asr, active: true } } : p,
        ),
    };
    const options = importOptions(engines, backend.peek().models, t, "zh-CN");
    const groq = engines.providers.find((p) => p.id === "groq")?.asr?.model;
    expect(options.speechDefault).toBe(`remote:groq:${groq}`);
    const none = importOptions(new MockBackend().peek().engines, [], t, "zh-CN");
    expect(none.speech).toEqual([]);
    expect(none.speechDefault).toBeUndefined();
    expect(none.polish).toEqual([]);
    expect(none.polishDefault).toBeUndefined();
  });

  it("offers a custom endpoint and Ollama the user set up without a key, a cloud provider only with one", () => {
    // docs/dictation.md §31.4 (user decision D20): a custom endpoint and Ollama are the user's own
    // as configured, key or not; a provider that needs a key is listed once the user entered it.
    const settings = {
      engines: {
        ...defaultEngineSettings(),
        providers: {
          custom: {
            asr_url: "https://asr.example.test/v1",
            asr_model: "my-asr",
            llm_url: "https://llm.example.test/v1",
            llm_model: "my-llm",
          },
          ollama: { llm_model: "qwen3:8b" },
          groq: { asr_model: "whisper-large-v3", llm_model: "qwen/qwen3.8-27b" },
        },
      },
    };
    const keyless = new MockBackend({ settings }).peek();
    const options = importOptions(keyless.engines, keyless.models, t, "zh-CN");
    const speech = options.speech.map((o) => o.value);
    const polish = options.polish.map((o) => o.value);
    expect(speech).toEqual(["remote:custom:my-asr"]);
    expect(polish).toEqual(["remote:ollama:qwen3:8b", "remote:custom:my-llm"]);
    // The custom endpoint is named as the menus name it.
    expect(options.speech[0]?.label).toBe("自定义接口 · my-asr");

    const keyed = new MockBackend({
      settings,
      providerKeys: [{ provider: "groq", kind: "asr" }],
    }).peek();
    const withGroq = importOptions(keyed.engines, keyed.models, t, "zh-CN");
    expect(withGroq.speech.map((o) => o.value)).toContain("remote:groq:whisper-large-v3");
    expect(withGroq.polish.map((o) => o.value)).toContain("remote:groq:qwen/qwen3.8-27b");
    expect(
      [...withGroq.speech, ...withGroq.polish].some((o) => o.value.startsWith("remote:builtin:")),
    ).toBe(false);
  });

  it("turns a row back into what import_add sends", () => {
    expect(recognitionOf("remote:groq:whisper-large-v3")).toEqual({
      kind: "remote",
      provider: "groq",
      model: "whisper-large-v3",
    });
    expect(recognitionOf("local:sense-voice-small")).toEqual({
      kind: "local",
      id: "sense-voice-small",
    });
    expect(recognitionOf("manage:speech")).toBeUndefined();
    expect(polishOf("remote:ollama:qwen3:8b")).toEqual({ provider: "ollama", model: "qwen3:8b" });
    expect(polishOf("local:sense-voice-small")).toBeUndefined();
    expect(polishOf("nonsense")).toBeUndefined();
  });
});
