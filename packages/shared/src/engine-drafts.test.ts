import {
  applyProviderDraft,
  checkProviderDraft,
  languageOptions,
  modelChoices,
  probeText,
  providerDraft,
  providersFor,
  serviceTarget,
  shortModel,
  withProvider,
} from "./engine-drafts";
import { createTranslator, zhT } from "./i18n";
import type { EngineSettings, EngineStatus, ProviderStatus, ServiceStatus } from "./schema";

const t = zhT.t;

function service(overrides: Partial<ServiceStatus> = {}): ServiceStatus {
  return {
    model: "",
    presets: [],
    key: { set: false, source: "none" },
    active: false,
    ...overrides,
  };
}

function provider(overrides: Partial<ProviderStatus> = {}): ProviderStatus {
  return { id: "groq", key: "required", on_device: false, console: true, ...overrides };
}

const SETTINGS: EngineSettings = {
  asr_provider: "builtin",
  llm_provider: "builtin",
  providers: { groq: { asr_model: "whisper-large-v3", llm_url: "https://example.test/v1" } },
} as EngineSettings;

describe("engine drafts (provider cards, desktop and phone)", () => {
  it("lists the providers that offer a service", () => {
    const status = {
      providers: [
        provider({ id: "builtin", asr: service(), llm: service() }),
        provider({ id: "local", asr: service() }),
        provider({ id: "deepseek", llm: service() }),
      ],
    } as EngineStatus;
    expect(providersFor(status, "asr").map((p) => p.id)).toEqual(["builtin", "local"]);
    expect(providersFor(status, "llm").map((p) => p.id)).toEqual(["builtin", "deepseek"]);
  });

  it("opens a card with the saved choices of its service and never a key", () => {
    expect(providerDraft(SETTINGS, "groq", "asr")).toEqual({
      model: "whisper-large-v3",
      baseUrl: "",
      key: "",
    });
    expect(providerDraft(SETTINGS, "groq", "llm")).toEqual({
      model: "",
      baseUrl: "https://example.test/v1",
      key: "",
    });
    expect(providerDraft(SETTINGS, "openai", "asr")).toEqual({ model: "", baseUrl: "", key: "" });
  });

  it("folds a draft into the settings, and an empty draft removes the override", () => {
    const next = applyProviderDraft(SETTINGS, "groq", "asr", {
      model: " whisper-large-v3-turbo ",
      baseUrl: " https://asr.example.test/v1 ",
    });
    expect(next.providers?.groq).toEqual({
      asr_model: "whisper-large-v3-turbo",
      asr_url: "https://asr.example.test/v1",
      llm_url: "https://example.test/v1",
    });
    expect(SETTINGS.providers?.groq?.asr_model).toBe("whisper-large-v3");
    const llm = applyProviderDraft(SETTINGS, "groq", "llm", { model: "qwen", baseUrl: "" });
    expect(llm.providers?.groq).toEqual({ asr_model: "whisper-large-v3", llm_model: "qwen" });
    // The last override gone takes the provider, then the map, with it.
    const cleared = applyProviderDraft(
      applyProviderDraft(SETTINGS, "groq", "asr", { model: "", baseUrl: "" }),
      "groq",
      "llm",
      { model: "", baseUrl: "" },
    );
    expect(cleared.providers).toBeUndefined();
    const kept = applyProviderDraft(SETTINGS, "openai", "asr", { model: "", baseUrl: "" });
    expect(kept.providers).toEqual(SETTINGS.providers);
  });

  it("switches the provider of one service only", () => {
    expect(withProvider(SETTINGS, "asr", "groq")).toMatchObject({
      asr_provider: "groq",
      llm_provider: "builtin",
    });
    expect(withProvider(SETTINGS, "llm", "deepseek")).toMatchObject({
      asr_provider: "builtin",
      llm_provider: "deepseek",
    });
  });

  it("offers the model in effect, the presets and the listed ones, once each", () => {
    const s = service({ model: "b", presets: ["a", "b"] });
    expect(modelChoices(s)).toEqual(["b", "a"]);
    expect(modelChoices(s, ["c", "a", ""])).toEqual(["b", "a", "c"]);
    expect(modelChoices(service())).toEqual([]);
  });

  it("checks a draft before it is saved", () => {
    const vendor = provider({
      asr: service({ model: "whisper", default_base_url: "https://api.groq.com/openai/v1" }),
    });
    const draft = { model: "", baseUrl: "", key: "gsk" };
    expect(checkProviderDraft(draft, vendor, "asr", t)).toBeUndefined();
    expect(checkProviderDraft({ ...draft, baseUrl: "ftp://x" }, vendor, "asr", t)).toBe(
      t("engines.check.badUrl"),
    );
    expect(checkProviderDraft({ ...draft, key: " " }, vendor, "asr", t)).toBe(
      t("engines.check.missingKey"),
    );
    const keyed = provider({
      asr: service({
        model: "whisper",
        default_base_url: "https://x.test",
        key: { set: true, source: "user" },
      }),
    });
    expect(checkProviderDraft({ ...draft, key: "" }, keyed, "asr", t)).toBeUndefined();
    const custom = provider({ id: "custom", key: "optional", asr: service() });
    expect(checkProviderDraft({ model: "m", baseUrl: "", key: "" }, custom, "asr", t)).toBe(
      t("engines.check.missingUrl"),
    );
    expect(
      checkProviderDraft(
        { model: " ", baseUrl: "http://127.0.0.1:8000/v1", key: "" },
        custom,
        "asr",
        t,
      ),
    ).toBe(t("engines.check.missingModel"));
    expect(
      checkProviderDraft(
        { model: "m", baseUrl: "http://127.0.0.1:8000/v1", key: "" },
        custom,
        "asr",
      ),
    ).toBeUndefined();
  });

  it("says what a probe answered", () => {
    const ok = probeText(
      { provider: "groq", kind: "asr", result: "ok", models: ["a", "b"], latency_ms: 42 },
      t,
    );
    expect(ok).toEqual({ ok: true, text: t("engines.probeResult.ok", { n: 2, ms: 42 }) });
    expect(probeText({ provider: "groq", kind: "asr", result: "ok" }, t).text).toBe(
      t("engines.probeResult.ok", { n: 0, ms: 0 }),
    );
    const status = probeText(
      { provider: "groq", kind: "llm", result: "failed", reason: "http_status", status: 503 },
      t,
    );
    expect(status).toEqual({
      ok: false,
      text: t("engines.probeResult.http_status", { status: 503 }),
    });
    expect(probeText({ provider: "groq", kind: "llm", result: "failed" }).text).toBe(
      t("engines.probeResult.unreachable", { status: 0 }),
    );
  });

  it("names where a service sends its data", () => {
    expect(serviceTarget(undefined, "x", t)).toBeUndefined();
    expect(serviceTarget("local", "x", t)).toBeUndefined();
    expect(serviceTarget("ollama", "127.0.0.1", t)).toBeUndefined();
    expect(serviceTarget("builtin", "hidden.example.test", t)).toBe(t("engines.provider.builtin"));
    expect(serviceTarget("custom", "asr.example.test", t)).toBe("asr.example.test");
    expect(serviceTarget("groq", "")).toBe(t("engines.provider.groq"));
  });

  it("shortens a model id to its name without the vendor", () => {
    expect(shortModel("Qwen/Qwen3-ASR-1.7B")).toBe("Qwen3-ASR-1.7B");
    expect(shortModel("whisper-large-v3")).toBe("whisper-large-v3");
    expect(shortModel("vendor/")).toBe("vendor/");
  });

  it("offers the recognition languages with a localised auto entry", () => {
    expect(languageOptions(t)[0]).toEqual({ value: "", label: t("language.auto") });
    expect(languageOptions(createTranslator("en").t).map((o) => o.value)).toEqual([
      "",
      "zh",
      "en",
      "yue",
      "ja",
      "ko",
    ]);
    expect(languageOptions().length).toBe(6);
  });
});
