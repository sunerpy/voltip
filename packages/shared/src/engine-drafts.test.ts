import {
  choosesInterface,
  applyProviderDraft,
  checkProviderDraft,
  endpointHost,
  languageOptions,
  modelChoices,
  modelListHelp,
  modelOptions,
  probeText,
  providerBadges,
  providerDraft,
  providerTagline,
  providersFor,
  serviceTarget,
  shortModel,
  withProvider,
} from "./engine-drafts";
import { defaultEngineSettings } from "./schema";
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

  it("docs/dictation.md §3.7: the custom clean-up card keeps its interface and the Responses effort", () => {
    const base = defaultEngineSettings();
    expect(providerDraft(base, "custom", "llm")).toEqual({
      model: "",
      baseUrl: "",
      key: "",
      api: "chat_completions",
    });
    expect(providerDraft(base, "custom", "asr").api).toBeUndefined();
    expect(providerDraft(base, "openai", "llm").api).toBeUndefined();
    const responses = applyProviderDraft(base, "custom", "llm", {
      model: "claude-opus-5-5",
      baseUrl: "http://127.0.0.1:8787/v1",
      api: "responses",
      reasoning: "high",
    });
    expect(responses.providers?.custom).toEqual({
      llm_model: "claude-opus-5-5",
      llm_url: "http://127.0.0.1:8787/v1",
      llm_api: "responses",
      llm_reasoning: "high",
    });
    expect(providerDraft(responses, "custom", "llm")).toMatchObject({
      api: "responses",
      reasoning: "high",
    });
    // Chat completions is not written, and takes no effort with it.
    const chat = applyProviderDraft(responses, "custom", "llm", {
      model: "m",
      baseUrl: "http://127.0.0.1:8787/v1",
      api: "chat_completions",
      reasoning: "high",
    });
    expect(chat.providers?.custom).toEqual({ llm_model: "m", llm_url: "http://127.0.0.1:8787/v1" });
    // Another card never writes the fields.
    const vendor = applyProviderDraft(base, "openai", "llm", {
      model: "gpt-5",
      baseUrl: "",
      api: "responses",
      reasoning: "low",
    });
    expect(vendor.providers?.openai).toEqual({ llm_model: "gpt-5" });
    expect(choosesInterface("custom", "llm")).toBe(true);
    expect(choosesInterface("custom", "asr")).toBe(false);
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
    // The quick menus keep to the suggested models: a provider's list can run to hundreds.
    expect(modelChoices(service({ model: "b", presets: ["a"], listed: ["z", "y"] }))).toEqual([
      "b",
      "a",
    ]);
  });

  it("regression: the select offers the provider's models under the count 测试连接 reports", () => {
    // User report 2026-10-12: the select had the few suggested models while 测试连接 counted many.
    const plain = (ids: string[]) => ids.map((m) => ({ value: m, label: m }));
    const other = { value: "__other__", label: "其他模型…" };
    const vendor = service({ model: "b", presets: ["a", "b"] });
    expect(modelOptions(vendor, [], t, other)).toEqual([...plain(["b", "a"]), other]);
    const listed = service({ model: "b", presets: ["a", "b"], listed: ["c", "a", "d"] });
    const offered = (m: string) => ({ value: m, label: m, group: "服务商提供（3）" });
    expect(modelOptions(listed, [], t, other)).toEqual([
      ...plain(["b", "a"]),
      other,
      offered("a"),
      offered("c"),
      offered("d"),
    ]);
    // What a 测试连接 in the form found stands in for the saved list.
    expect(modelOptions(listed, ["e", "e", ""], t)).toEqual([
      ...plain(["b", "a"]),
      { value: "e", label: "e", group: "服务商提供（1）" },
    ]);
    // A provider without suggestions (Ollama, a custom endpoint): the model in effect, if any.
    expect(modelOptions(service({ model: "x", listed: ["y", "x"] }), [], t)).toEqual([
      { value: "x", label: "x" },
      { value: "x", label: "x", group: "服务商提供（2）" },
      { value: "y", label: "y", group: "服务商提供（2）" },
    ]);
    expect(modelOptions(service())).toEqual([]);
    const en = createTranslator("en").t;
    expect(modelOptions(listed, [], en)[2]?.group).toBe("From the provider (3)");
  });

  it("regression: a list that arrives only adds options after the ones shown", () => {
    // CI 2026-10-12: the list arrived while a test was choosing 其他模型… and the options it had
    // were rebuilt under it. WebKit's native select also reports a choice by position, so the
    // options a select shows keep their places when the provider's list comes in.
    const other = { value: "__other__", label: "其他模型…" };
    for (const before of [
      service({ model: "b", presets: ["a", "b"] }),
      service({ model: "x" }),
      service(),
    ]) {
      const shown = modelOptions(before, [], t, other);
      const after = modelOptions({ ...before, listed: ["z", "a", "b", "x"] }, [], t, other);
      expect(after.slice(0, shown.length)).toEqual(shown);
      expect(after.length).toBeGreaterThan(shown.length);
    }
  });

  it("says how many models the provider lists, or where the list comes from", () => {
    const vendor = provider({ key: "required" });
    const keyless = provider({ id: "ollama", key: "none" });
    expect(modelListHelp(vendor, service(), t)).toBe(t("engines.field.modelHelp"));
    expect(modelListHelp(keyless, service(), t)).toBe(t("engines.field.modelHelpKeyless"));
    expect(modelListHelp(vendor, service({ listed: ["a", "b"] }), t)).toBe(
      "列表包含服务商提供的 2 个可用模型，每次打开列表时更新。",
    );
    const en = createTranslator("en").t;
    expect(modelListHelp(vendor, service({ listed: ["a"] }), en)).toBe(
      "The list includes the model the provider offers for this service, updated each time the list opens.",
    );
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

  it("takes the host of an http(s) address and nothing else", () => {
    expect(endpointHost("https://api.groq.com/openai/v1")).toBe("api.groq.com");
    expect(endpointHost(" http://127.0.0.1:11434/v1 ")).toBe("127.0.0.1:11434");
    expect(endpointHost("https://user:secret@asr.example.test/v1")).toBe("asr.example.test");
    expect(endpointHost("HTTPS://ASR.example.test")).toBe("ASR.example.test");
    expect(endpointHost("asr.example.test")).toBeUndefined();
    expect(endpointHost("ftp://asr.example.test")).toBeUndefined();
    expect(endpointHost("")).toBeUndefined();
  });

  it("puts the host under a card's name, or what the provider is when it has none to show", () => {
    const en = createTranslator("en").t;
    const groq = provider({ asr: service({ base_url: "https://api.groq.com/openai/v1" }) });
    expect(providerTagline(groq, "asr", t)).toBe("api.groq.com");
    // The service the card is for: Groq has no clean-up address here.
    expect(providerTagline(groq, "llm", t)).toBe(t("engines.tagline.custom"));
    // The built-in service's host is never shown, even if a status carried one.
    const builtin = provider({
      id: "builtin",
      key: "none",
      asr: service({ base_url: "https://hidden.example.test/v1" }),
    });
    expect(providerTagline(builtin, "asr", t)).toBe(t("engines.tagline.builtin"));
    expect(providerTagline(provider({ id: "local", asr: service() }), "asr", en)).toBe(
      "Runs on this computer",
    );
    const custom = provider({ id: "custom", key: "optional", asr: service() });
    expect(providerTagline(custom, "asr", t)).toBe("OpenAI 兼容接口");
    const filled = provider({
      id: "custom",
      asr: service({ base_url: "http://192.168.1.20:8000/v1" }),
    });
    expect(providerTagline(filled, "asr")).toBe("192.168.1.20:8000");
  });

  it("says 使用中 on the provider in use, then why one cannot run, or 就绪", () => {
    expect(providerBadges(service({ active: true }), t)).toEqual([
      { tone: "accent", text: "使用中" },
    ]);
    expect(providerBadges(service({ active: true, issue: "key_missing" }), t)).toEqual([
      { tone: "accent", text: "使用中" },
      { tone: "warn", text: "缺少密钥" },
    ]);
    expect(providerBadges(service({ issue: "url_missing" }))).toEqual([
      { tone: "warn", text: "缺少接口地址" },
    ]);
    expect(providerBadges(service(), createTranslator("en").t)).toEqual([
      { tone: "ok", text: "Ready" },
    ]);
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
