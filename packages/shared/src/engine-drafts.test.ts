import {
  choosesInterface,
  applyProviderDraft,
  asrAbilities,
  asrAbilityHelp,
  asrCardAbilities,
  checkCredentialDraft,
  checkProbeDraft,
  checkProviderDraft,
  clearedSecret,
  credentialDraft,
  credentialFieldLabel,
  credentialHelp,
  credentialSetLabel,
  credentialValue,
  draftSecret,
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
  realtimeModel,
  serviceTarget,
  shortModel,
  siteOf,
  siteUrl,
  withProvider,
} from "./engine-drafts";
import { defaultEngineSettings } from "./schema";
import { PROVIDER_IDS } from "./schema";
import { keyEntry, providerSpec, resolveEngineStatus } from "./providers";
import { createTranslator, zhT } from "./i18n";
import type {
  EngineSettings,
  EngineStatus,
  ProbeReport,
  ProviderStatus,
  ServiceStatus,
} from "./schema";

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
  it("docs/dictation.md §3.2: the URL field names a vendor's site, and a site fills the field", () => {
    const kimi = service({
      sites: [
        { id: "cn", base_url: "https://api.moonshot.cn/v1" },
        { id: "intl", base_url: "https://api.moonshot.ai/v1" },
      ],
    });
    expect(siteOf(kimi, "")).toBe("cn");
    expect(siteOf(kimi, "  ")).toBe("cn");
    expect(siteOf(kimi, "https://api.moonshot.ai/v1")).toBe("intl");
    expect(siteOf(kimi, " HTTPS://API.moonshot.AI/v1/ ")).toBe("intl");
    expect(siteOf(kimi, "https://api.moonshot.cn/v1")).toBe("cn");
    expect(siteOf(kimi, "https://proxy.example.test/v1")).toBeUndefined();
    expect(siteOf(service(), "")).toBeUndefined();
    // The default site empties the field, so the catalogue's address follows it.
    expect(siteUrl(kimi, "cn")).toBe("");
    expect(siteUrl(kimi, "intl")).toBe("https://api.moonshot.ai/v1");
    expect(siteUrl(service(), "intl")).toBe("");
  });

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

  it("regression: section 3.12 a failed probe says what the service said", () => {
    // 2026-10-24: a refused 测试连接 said only 密钥被拒绝 or 服务返回错误（HTTP 404）.
    const refused = probeText(
      {
        provider: "openai",
        kind: "llm",
        result: "failed",
        reason: "unauthorized",
        detail: "Incorrect API key provided: sk-***. (invalid_api_key)",
      },
      t,
    );
    expect(refused).toEqual({
      ok: false,
      text: "密钥被拒绝：Incorrect API key provided: sk-***. (invalid_api_key)",
    });
    const gone = probeText(
      {
        provider: "google",
        kind: "llm",
        result: "failed",
        reason: "http_status",
        status: 404,
        detail: "Not Found",
      },
      createTranslator("en").t,
    );
    expect(gone.text).toBe("The service returned HTTP 404: Not Found");
    const blank = probeText(
      { provider: "groq", kind: "llm", result: "failed", reason: "timeout", detail: "  " },
      t,
    );
    expect(blank.text).toBe(t("engines.probeResult.timeout"));
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

describe("credentials of several fields (docs/dictation.md §3.15, the core's merge_credentials)", () => {
  /** A vendor's card as the engines status shows it, with its credential stored of `set`. */
  const card = (id: "tencent" | "volcengine_speech", set?: string) => {
    const entry = keyEntry(id, "asr") ?? "";
    const status = resolveEngineStatus({
      settings: defaultEngineSettings(),
      userKeys: new Set(set === undefined ? [] : [entry]),
      credentialSets: set === undefined ? {} : { [entry]: set },
      builtIn: {},
      local: { id: "", name: "", installed: false },
      localPreviewReady: false,
    });
    const found = status.providers.find((p) => p.id === id);
    if (found === undefined) throw new Error(id);
    return found;
  };
  const en = createTranslator("en").t;

  it("opens with the stored set, else the first, every field empty", () => {
    expect(credentialDraft(card("volcengine_speech"), card("volcengine_speech").asr)).toEqual({
      set: "api_key",
      fields: {},
    });
    const old = card("volcengine_speech", "app_token");
    expect(credentialDraft(old, old.asr)).toEqual({ set: "app_token", fields: {} });
    expect(credentialDraft(provider(), service())).toBeUndefined();
  });

  it("checks a draft by the core's rule, case by case", () => {
    const fresh = card("tencent");
    const stored = card("tencent", "default");
    const cases: [string, ProviderStatus, Record<string, string>, string | undefined][] = [
      [
        "nothing stored: every field",
        fresh,
        { app_id: "125", secret_id: "test-secret-id", secret_key: "sk" },
        undefined,
      ],
      ["nothing stored: a field missing", fresh, { app_id: "125" }, "请填写 SecretId、SecretKey"],
      [
        "nothing stored: blanks count as missing",
        fresh,
        { app_id: " ", secret_id: "test-secret-id", secret_key: "sk" },
        "请填写 App ID",
      ],
      ["the same set: blanks keep the stored values", stored, {}, undefined],
      ["the same set: one field replaced", stored, { secret_key: "new" }, undefined],
    ];
    expect(
      cases.map(([what, status, fields]) => [
        what,
        checkCredentialDraft({ set: "default", fields }, status, status.asr, t),
      ]),
    ).toEqual(cases.map(([what, , , want]) => [what, want]));
    // 豆包语音: another set than the stored one needs all of its fields.
    const old = card("volcengine_speech", "app_token");
    expect(checkCredentialDraft({ set: "api_key", fields: {} }, old, old.asr, t)).toBe(
      "请填写 API Key",
    );
    expect(
      checkCredentialDraft({ set: "api_key", fields: { api_key: "k" } }, old, old.asr, t),
    ).toBeUndefined();
    expect(checkCredentialDraft({ set: "app_token", fields: {} }, old, old.asr, t)).toBeUndefined();
    expect(checkCredentialDraft({ set: "other", fields: {} }, old, old.asr, t)).toBe(
      t("engines.check.missingKey"),
    );
    expect(checkCredentialDraft({ set: "default", fields: {} }, fresh, fresh.asr, en)).toBe(
      "Enter App ID, SecretId, SecretKey",
    );
  });

  it("sends the set and the fields typed, and nothing when none is", () => {
    expect(
      credentialValue({
        set: "default",
        fields: { app_id: " 125 ", secret_id: "", secret_key: "sk" },
      }),
    ).toBe('{"set":"default","app_id":"125","secret_key":"sk"}');
    expect(credentialValue({ set: "default", fields: { app_id: "  " } })).toBeUndefined();
    const draft = {
      model: "",
      baseUrl: "",
      key: "",
      credential: { set: "api_key", fields: { api_key: "k" } },
    };
    expect(draftSecret(draft)).toBe('{"set":"api_key","api_key":"k"}');
    expect(clearedSecret(draft)).toEqual({ ...draft, credential: { set: "api_key", fields: {} } });
    expect(draftSecret({ model: "", baseUrl: "", key: " gsk " })).toBe("gsk");
    expect(draftSecret({ model: "", baseUrl: "", key: " " })).toBeUndefined();
  });

  it("checks a card's draft with its credential instead of a key", () => {
    const fresh = card("tencent");
    const draft = { model: "", baseUrl: "", key: "" };
    expect(checkProviderDraft(draft, fresh, "asr", t)).toBe("请填写 App ID、SecretId、SecretKey");
    expect(
      checkProviderDraft(
        {
          ...draft,
          credential: { set: "default", fields: { app_id: "1", secret_id: "2", secret_key: "3" } },
        },
        fresh,
        "asr",
        t,
      ),
    ).toBeUndefined();
    const stored = card("tencent", "default");
    expect(checkProviderDraft(draft, stored, "asr", t)).toBeUndefined();
  });

  it("regression: 测试连接 names the missing fields of a credential of several fields, not an API key", () => {
    // 2026-11-04, the packaged app: 测试连接 with the old console's fields empty sent nothing, and the
    // core's answer read 请先填写 API 密钥, a field the form does not have.
    const fresh = card("volcengine_speech");
    const blank = { model: "", baseUrl: "", key: "" };
    const old = (fields: Record<string, string>) => ({
      ...blank,
      credential: { set: "app_token", fields },
    });
    expect(checkProbeDraft(old({}), fresh, "asr", t)).toBe("请填写 App ID、Access Token");
    expect(checkProbeDraft(old({ app_id: "1" }), fresh, "asr", en)).toBe("Enter Access Token");
    expect(
      checkProbeDraft(old({ app_id: "1", access_token: "2" }), fresh, "asr", t),
    ).toBeUndefined();
    // The stored set left blank is tested as saved; another set is filled in whole first.
    const stored = card("volcengine_speech", "app_token");
    expect(checkProbeDraft(old({}), stored, "asr", t)).toBeUndefined();
    expect(checkProbeDraft(blank, stored, "asr", t)).toBeUndefined();
    expect(
      checkProbeDraft({ ...blank, credential: { set: "api_key", fields: {} } }, stored, "asr", t),
    ).toBe("请填写 API Key");
    // A single key is the core's to check: left blank, the saved one is tested.
    expect(checkProbeDraft(blank, provider({ asr: service() }), "asr", t)).toBeUndefined();
  });

  it("names every field and set of the catalogue, and where each vendor's fields are found", () => {
    const sets = (id: (typeof PROVIDER_IDS)[number]) => providerSpec(id).credential ?? [];
    const vendors = PROVIDER_IDS.filter((id) => sets(id).length > 0);
    expect(vendors).toEqual(["volcengine_speech", "tencent", "iflytek", "iflytek_rtasr", "baidu"]);
    expect(PROVIDER_IDS.filter((id) => !vendors.includes(id) && credentialHelp(id, t))).toEqual([]);
    expect(vendors.filter((id) => !credentialHelp(id, t) || !credentialHelp(id, en))).toEqual([]);
    const fields = vendors.flatMap((id) => sets(id).flatMap((set) => set.fields.map((f) => f.id)));
    expect(
      fields.filter(
        (id) => credentialFieldLabel(id, t) === id || credentialFieldLabel(id, en) === id,
      ),
    ).toEqual([]);
    const named = vendors.flatMap((id) => (sets(id).length > 1 ? sets(id).map((s) => s.id) : []));
    expect(named).toEqual(["api_key", "app_token"]);
    expect(named.filter((id) => credentialSetLabel(id, t) === id)).toEqual([]);
    expect(credentialFieldLabel("token", t)).toBe("token");
    expect(credentialSetLabel("default", t)).toBe("default");
  });

  it("says a vendor without a model list checked the credential, and that its list is the presets", () => {
    const report: ProbeReport = {
      provider: "tencent",
      kind: "asr",
      result: "ok",
      models: ["16k_zh_en"],
      latency_ms: 40,
    };
    expect(probeText(report, t, true)).toEqual({ ok: true, text: "凭据有效 · 40 ms" });
    expect(probeText(report, t).text).toBe("连接正常 · 1 个模型 · 40 ms");
    const fresh = card("tencent");
    expect(modelListHelp(fresh, fresh.asr ?? service(), t)).toBe(
      t("engines.field.modelHelpPresets"),
    );
  });
});

describe("what a recognition model does with a take and the dictionary (docs/dictation.md §3.15)", () => {
  /** The recognition card of `id` as the engines status shows it, with nothing saved. */
  const asrCard = (id: (typeof PROVIDER_IDS)[number]) => {
    const status = resolveEngineStatus({
      settings: defaultEngineSettings(),
      userKeys: new Set(),
      credentialSets: {},
      builtIn: { asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true } },
      local: { id: "", name: "", installed: false },
      localPreviewReady: false,
    });
    const found = status.providers.find((p) => p.id === id);
    if (found === undefined) throw new Error(id);
    return found;
  };
  const en = createTranslator("en").t;

  it("names on each card whether a model recognises while you speak and takes hot words", () => {
    const named = (id: (typeof PROVIDER_IDS)[number], previews = false) =>
      asrCardAbilities(asrCard(id), previews, t);
    const realtime = t("engines.ability.realtime");
    const hotwords = t("engines.ability.hotwords");
    expect(named("volcengine_speech")).toEqual([realtime, hotwords]);
    expect(named("tencent")).toEqual([realtime, hotwords]);
    expect(named("aliyun")).toEqual([realtime, hotwords]);
    // StepFun: its stream model recognises while you speak, its whole-file model takes hot words.
    expect(named("stepfun")).toEqual([realtime, hotwords]);
    expect(named("iflytek")).toEqual([realtime]);
    expect(named("iflytek_rtasr")).toEqual([realtime]);
    expect(named("baidu")).toEqual([realtime]);
    expect(named("glm")).toEqual([hotwords]);
    expect(named("groq")).toEqual([hotwords]);
    expect(named("mimo")).toEqual([]);
    expect(named("local")).toEqual([]);
    // The built-in service previews where the shell shows its preview (the desktop).
    expect(named("builtin", true)).toEqual([t("engines.ability.preview"), hotwords]);
    expect(named("builtin")).toEqual([hotwords]);
    expect(asrCardAbilities(asrCard("baidu"), false, en)).toEqual(["Realtime"]);
  });

  it("follows the address and the model as the core's protocol does", () => {
    const dashscope = "https://dashscope.aliyuncs.com/compatible-mode/v1";
    expect(asrAbilities("aliyun", dashscope, "qwen-audio-3.1-asr-flash-streaming")).toEqual({
      live: "realtime",
      glossary: "hotwords",
    });
    expect(asrAbilities("aliyun", dashscope, "fun-asr-realtime")).toEqual({
      live: "realtime",
      glossary: "prompt",
    });
    expect(asrAbilities("custom", "https://example.test/v1", "whisper-1")).toEqual({
      live: "whole",
      glossary: "prompt",
    });
    // A vendor's protocol does not depend on the address: a proxy is still the vendor.
    expect(asrAbilities("baidu", "https://proxy.example.test", "15372")).toEqual({
      live: "realtime",
      glossary: "not_supported",
    });
    expect(asrAbilities("local", "", "qwen3-asr-0.6b")).toEqual({
      live: "whole",
      glossary: "not_supported",
    });
  });

  it("marks the realtime models in a recognition select, suggested and listed alike", () => {
    const stepfun = asrCard("stepfun").asr ?? service();
    const marked = modelOptions(
      { ...stepfun, listed: ["stepaudio-2.5-asr-stream", "stepaudio-2-asr"] },
      [],
      t,
      undefined,
      realtimeModel("stepfun", stepfun.base_url ?? ""),
    );
    expect(marked.map((o) => [o.value, o.label])).toEqual([
      ["stepaudio-2.5-asr", "stepaudio-2.5-asr"],
      [
        "stepaudio-2.5-asr-stream",
        t("engines.field.modelRealtime", { model: "stepaudio-2.5-asr-stream" }),
      ],
      ["stepaudio-2-asr", "stepaudio-2-asr"],
      [
        "stepaudio-2.5-asr-stream",
        t("engines.field.modelRealtime", { model: "stepaudio-2.5-asr-stream" }),
      ],
    ]);
    expect(t("engines.field.modelRealtime", { model: "m" })).toBe("m · 边说边识别");
    // Without the rule (clean-up, the switchers' other services) the labels are the ids.
    expect(modelOptions(stepfun, [], t).map((o) => o.label)).toEqual([
      "stepaudio-2.5-asr",
      "stepaudio-2.5-asr-stream",
    ]);
  });

  it("says under the model how it hears a take and where the terms go, for the shell", () => {
    const desktop = { previews: true, providerPreview: true, localPreview: "missing" as const };
    const phone = { previews: false, providerPreview: true, localPreview: "none" as const };
    const help = (
      id: (typeof PROVIDER_IDS)[number],
      model: string,
      shell: Parameters<typeof asrAbilityHelp>[3],
    ) => asrAbilityHelp(id, "", model, shell, t);
    expect(help("tencent", "16k_zh_en", desktop)).toEqual([
      t("engines.ability.realtimeHelp"),
      t("engines.ability.hotwordsHelp"),
    ]);
    // docs/dictation.md §11.10: with 服务商的实时文字 off a realtime model still recognises while you
    // speak; the pill shows none of its words. The phone shows none anyway.
    expect(help("tencent", "16k_zh_en", { ...desktop, providerPreview: false })[0]).toBe(
      t("engines.ability.realtimeHiddenHelp"),
    );
    expect(help("tencent", "16k_zh_en", { ...phone, providerPreview: false })[0]).toBe(
      t("engines.ability.realtimeHelp"),
    );
    // 本地模型预览 off: the help says where to turn it on.
    expect(help("mimo", "mimo-v2.5-asr", { ...desktop, localPreview: "off" })[0]).toBe(
      t("engines.ability.wholePreviewHelp"),
    );
    expect(help("mimo", "mimo-v2.5-asr", desktop)).toEqual([
      t("engines.ability.wholePreviewHelp"),
      t("engines.ability.noGlossaryHelp"),
    ]);
    expect(help("mimo", "mimo-v2.5-asr", { ...desktop, localPreview: "installed" })[0]).toBe(
      t("engines.ability.wholeLocalHelp"),
    );
    // The phone keeps no models and shows no preview.
    expect(help("mimo", "mimo-v2.5-asr", phone)[0]).toBe(t("engines.ability.wholeHelp"));
    expect(help("builtin", "Qwen/Qwen3-ASR-1.7B", desktop)).toEqual([
      t("engines.ability.previewHelp"),
      t("engines.ability.promptHelp"),
    ]);
    // The built-in service's words off: it sends no previews, and the local model may show its own.
    expect(help("builtin", "Qwen/Qwen3-ASR-1.7B", { ...desktop, providerPreview: false })[0]).toBe(
      t("engines.ability.wholePreviewHelp"),
    );
    expect(
      help("builtin", "Qwen/Qwen3-ASR-1.7B", {
        ...desktop,
        providerPreview: false,
        localPreview: "installed",
      })[0],
    ).toBe(t("engines.ability.wholeLocalHelp"));
    expect(t("engines.ability.realtimeHiddenHelp")).toBe(
      "边说边识别：说话时就开始识别，松开后只等最后一句；「服务商的实时文字」已关闭，悬浮窗不显示服务商的文字。",
    );
    expect(help("builtin", "Qwen/Qwen3-ASR-1.7B", phone)[0]).toBe(t("engines.ability.wholeHelp"));
    expect(help("stepfun", "stepaudio-2.5-asr-stream", phone)).toEqual([
      t("engines.ability.realtimeHelp"),
      t("engines.ability.promptHelp"),
    ]);
  });
});
