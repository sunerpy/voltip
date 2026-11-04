// The provider catalogue on the TypeScript side (`voltip_core::providers::PROVIDERS`) and the
// resolution the browser preview runs in place of the core (`ResolvedEngines::status`). The Rust
// catalogue is the source of truth: `ipc-contract.test.ts` compares this copy with the provider
// cards in the Rust-generated fixtures, so a drift fails the build.
import { fallbackSettingsOf } from "./fallback";
import type {
  CredentialSet,
  EngineIssue,
  EngineSettings,
  EngineStatus,
  FallbackModelStatus,
  FallbackStatus,
  GlossaryRoute,
  KeyPolicy,
  ModelState,
  ProviderId,
  ProviderStatus,
  SecretState,
  ServiceKind,
  ServiceStatus,
  SiteId,
  LiveSource,
} from "./schema";
import { PROVIDER_IDS } from "./schema";

/** A service a vendor offers: its public base URL, the models suggested first and, for a vendor
 *  with a Chinese and an international site, both sites (the default first, docs/dictation.md
 *  §3.2). */
export interface ServicePreset {
  baseUrl: string;
  models: readonly string[];
  sites?: readonly { id: SiteId; baseUrl: string }[];
}

export interface ProviderSpec {
  id: ProviderId;
  /** Recognition, if offered (an empty preset for the built-in, on-device and custom providers). */
  asr?: ServicePreset;
  /** Clean-up, if offered. */
  llm?: ServicePreset;
  key: KeyPolicy;
  onDevice: boolean;
  /** The shell can open the vendor's key page. */
  console: boolean;
  /** The shell can open the vendor's page of free quota and usage (Model Studio). */
  quota?: boolean;
  /** A credential of several fields: its sets, the default first (docs/dictation.md §3.15). */
  credential?: readonly CredentialSet[];
  /** `false`: the vendor lists no models (§3.15). */
  listsModels?: false;
}

const NO_PRESET: ServicePreset = { baseUrl: "", models: [] };

/** A required credential field. */
const field = (id: string, secret: boolean) => ({ id, secret, required: true });
/** A vendor with one credential set. */
const fields = (...list: ReturnType<typeof field>[]): readonly CredentialSet[] => [
  { id: "default", fields: list },
];

const SPECS: Readonly<Record<ProviderId, Omit<ProviderSpec, "id">>> = {
  builtin: {
    asr: NO_PRESET,
    llm: NO_PRESET,
    key: "builtin",
    onDevice: false,
    console: false,
  },
  local: { asr: NO_PRESET, key: "none", onDevice: true, console: false },
  openai: {
    asr: {
      baseUrl: "https://api.openai.com/v1",
      models: ["gpt-transcribe", "gpt-4o-mini-transcribe", "whisper-1"],
    },
    llm: {
      baseUrl: "https://api.openai.com/v1",
      models: ["gpt-6-luna", "gpt-6-sol"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  groq: {
    asr: {
      baseUrl: "https://api.groq.com/openai/v1",
      models: ["whisper-large-v3-turbo", "whisper-large-v3"],
    },
    llm: {
      baseUrl: "https://api.groq.com/openai/v1",
      models: ["qwen/qwen3.8-27b", "openai/gpt-oss-120b", "openai/gpt-oss-20b"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  google: {
    llm: {
      baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
      models: ["gemini-3.5-flash-lite", "gemini-3.8-flash", "gemini-3.1-flash-lite"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  siliconflow: {
    asr: {
      baseUrl: "https://api.siliconflow.cn/v1",
      models: ["FunAudioLLM/SenseVoiceSmall", "TeleAI/TeleSpeechASR"],
    },
    llm: {
      baseUrl: "https://api.siliconflow.cn/v1",
      models: ["Qwen/Qwen3-8B", "deepseek-ai/DeepSeek-V3"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  aliyun: {
    asr: {
      baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      models: [
        "qwen-audio-3.1-asr-flash-streaming",
        "qwen-audio-3.1-asr-flash",
        "fun-asr-realtime",
      ],
    },
    llm: {
      baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      models: ["qwen3.8-flash", "qwen3.8-max", "qwen3.7-flash"],
    },
    key: "required",
    onDevice: false,
    console: true,
    quota: true,
  },
  volcengine_speech: {
    asr: {
      baseUrl: "https://openspeech.bytedance.com",
      models: [
        "volc.seedasr.sauc.duration",
        "volc.bigasr.sauc.duration",
        "volc.seedasr.sauc.concurrent",
      ],
    },
    key: "required",
    onDevice: false,
    console: true,
    credential: [
      { id: "api_key", fields: [field("api_key", true)] },
      { id: "app_token", fields: [field("app_id", false), field("access_token", true)] },
    ],
    listsModels: false,
  },
  tencent: {
    asr: {
      baseUrl: "https://asr.cloud.tencent.com",
      models: ["16k_zh_en", "Hy-ASR-3.0-preview", "16k_zh"],
    },
    key: "required",
    onDevice: false,
    console: true,
    credential: fields(
      field("app_id", false),
      field("secret_id", false),
      field("secret_key", true),
    ),
    listsModels: false,
  },
  iflytek: {
    asr: { baseUrl: "https://iat-api.xfyun.cn", models: ["iat"] },
    key: "required",
    onDevice: false,
    console: true,
    credential: fields(field("app_id", false), field("api_key", true), field("api_secret", true)),
    listsModels: false,
  },
  iflytek_rtasr: {
    asr: { baseUrl: "https://rtasr.xfyun.cn", models: ["rtasr"] },
    key: "required",
    onDevice: false,
    console: true,
    credential: fields(field("app_id", false), field("api_key", true)),
    listsModels: false,
  },
  baidu: {
    asr: { baseUrl: "https://vop.baidu.com", models: ["15372", "1537", "15376"] },
    key: "required",
    onDevice: false,
    console: true,
    credential: fields(field("app_id", false), field("api_key", true)),
    listsModels: false,
  },
  stepfun: {
    asr: {
      baseUrl: "https://api.stepfun.com/v1",
      models: ["stepaudio-2.5-asr", "stepaudio-2.5-asr-stream"],
      sites: [
        { id: "cn", baseUrl: "https://api.stepfun.com/v1" },
        { id: "intl", baseUrl: "https://api.stepfun.ai/v1" },
      ],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  deepseek: {
    llm: {
      baseUrl: "https://api.deepseek.com",
      models: ["deepseek-flash", "deepseek-v4-pro"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  kimi: {
    llm: {
      baseUrl: "https://api.moonshot.cn/v1",
      models: ["kimi-k3.1-pro-highspeed", "kimi-k3.1-pro", "kimi-k3"],
      sites: [
        { id: "cn", baseUrl: "https://api.moonshot.cn/v1" },
        { id: "intl", baseUrl: "https://api.moonshot.ai/v1" },
      ],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  glm: {
    asr: {
      baseUrl: "https://open.bigmodel.cn/api/paas/v4",
      models: ["glm-asr-2512"],
      sites: [
        { id: "cn", baseUrl: "https://open.bigmodel.cn/api/paas/v4" },
        { id: "intl", baseUrl: "https://api.z.ai/api/paas/v4" },
      ],
    },
    llm: {
      baseUrl: "https://open.bigmodel.cn/api/paas/v4",
      models: ["glm-5.3-flash", "glm-4.7-flash", "glm-5.3"],
      sites: [
        { id: "cn", baseUrl: "https://open.bigmodel.cn/api/paas/v4" },
        { id: "intl", baseUrl: "https://api.z.ai/api/paas/v4" },
      ],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  volcengine: {
    llm: {
      baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
      models: [
        "doubao-seed-2-1-lite-260915",
        "doubao-seed-2-1-turbo-260628",
        "doubao-seed-2-0-mini-260428",
      ],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  hunyuan: {
    llm: { baseUrl: "https://tokenhub.tencentmaas.com/v1", models: ["hy3", "hy4-preview"] },
    key: "required",
    onDevice: false,
    console: true,
  },
  mimo: {
    asr: { baseUrl: "https://api.xiaomimimo.com/v1", models: ["mimo-v2.5-asr"] },
    llm: { baseUrl: "https://api.xiaomimimo.com/v1", models: ["mimo-v2.6-flash", "mimo-v2.6-pro"] },
    key: "required",
    onDevice: false,
    console: true,
  },
  openrouter: {
    llm: {
      baseUrl: "https://openrouter.ai/api/v1",
      models: ["google/gemini-3.5-flash-lite", "deepseek/deepseek-v4-flash", "openai/gpt-6-luna"],
    },
    key: "required",
    onDevice: false,
    console: true,
  },
  ollama: {
    llm: { baseUrl: "http://127.0.0.1:11434/v1", models: [] },
    key: "none",
    onDevice: true,
    console: false,
  },
  custom: {
    asr: NO_PRESET,
    llm: NO_PRESET,
    key: "optional",
    onDevice: false,
    console: false,
  },
};

export function providerSpec(id: ProviderId): ProviderSpec {
  return { id, ...SPECS[id] };
}

/** The catalogue in display order. */
export const PROVIDER_CATALOGUE: readonly ProviderSpec[] = PROVIDER_IDS.map(providerSpec);

export function offers(id: ProviderId, kind: ServiceKind): boolean {
  return providerSpec(id)[kind] !== undefined;
}

/** `voltip_core::providers::is_dashscope`: an Alibaba Cloud Model Studio address. */
export function isDashscope(url: string): boolean {
  try {
    return new URL(url.trim()).hostname.toLowerCase().endsWith(".aliyuncs.com");
  } catch {
    return false;
  }
}

/** The vendors whose provider picks the protocol, whatever the address (`AsrProtocol::resolve`,
 *  docs/dictation.md §3.15). */
const VENDOR_STREAMS: Partial<Record<ProviderId, (model: string) => boolean>> = {
  volcengine_speech: () => true,
  tencent: () => true,
  iflytek: () => true,
  iflytek_rtasr: () => true,
  baidu: () => true,
  stepfun: (model) => model.trim().toLowerCase().endsWith("-stream"),
  glm: () => false,
  mimo: () => false,
};

/** `AsrProtocol::resolve(provider, url, model).streams()` (docs/dictation.md §3.4, §3.15): a
 *  vendor's realtime protocol, or a Model Studio realtime model, which recognises while the take
 *  is spoken. */
export function asrStreams(provider: ProviderId, url: string, model: string): boolean {
  const vendor = VENDOR_STREAMS[provider];
  if (vendor !== undefined) return vendor(model);
  if (!isDashscope(url)) return false;
  const m = model.trim().toLowerCase();
  if (m.includes("filetrans") || m.startsWith("qwen3-asr")) return false;
  const realtime = m.includes("realtime");
  if (m.startsWith("qwen-audio") && m.includes("-asr"))
    return realtime || m.includes("-streaming") || m.includes("-message");
  return (m.startsWith("fun-asr") || m.startsWith("paraformer")) && realtime;
}

/** Where each vendor puts the glossary (`GlossaryRoute::for_service`, docs/dictation.md §3.15). */
const VENDOR_GLOSSARY: Partial<Record<ProviderId, (model: string) => GlossaryRoute>> = {
  volcengine_speech: () => "hotwords",
  tencent: () => "hotwords",
  iflytek: () => "not_supported",
  iflytek_rtasr: () => "not_supported",
  baidu: () => "not_supported",
  stepfun: (model) => (model.trim().toLowerCase().endsWith("-stream") ? "prompt" : "hotwords"),
  glm: () => "hotwords",
  mimo: () => "not_supported",
};

/** `GlossaryRoute::for_service(provider, url, model)` (docs/dictation.md §30.6): a vendor's own
 *  way (§3.15); else the `prompt` of an OpenAI-compatible endpoint, hot words for Model Studio's
 *  `qwen-audio` recognition models on its native and realtime protocols, nowhere for its other
 *  models. */
export function glossaryRoute(provider: ProviderId, url: string, model: string): GlossaryRoute {
  const vendor = VENDOR_GLOSSARY[provider];
  if (vendor !== undefined) return vendor(model);
  if (!isDashscope(url)) return "prompt";
  const m = model.trim().toLowerCase();
  if (m.includes("filetrans")) return "not_supported";
  if (m.startsWith("qwen-audio") && m.includes("-asr")) return "hotwords";
  // §3.15: Fun-ASR's context (`fun_asr_takes_context`).
  const context =
    m === "fun-asr-realtime" || (m.startsWith("fun-asr-flash") && !m.includes("realtime") && !m.includes("8k"));
  return context ? "prompt" : "not_supported";
}

/** The secret-store entry of the user's key (`voltip_core::providers::key_entry`): a vendor's two
 *  services share one, the custom endpoint keeps one per service, key-less providers have none. */
export function keyEntry(provider: ProviderId, kind: ServiceKind): string | undefined {
  switch (provider) {
    case "builtin":
    case "local":
    case "ollama":
      return undefined;
    case "custom":
      return `provider-key.custom-${kind}`;
    default:
      return `provider-key.${provider.replace("_", "-")}`;
  }
}

/** The built-in service a (preview) build carries: its model and whether a key is compiled in. */
export interface BuiltInService {
  model: string;
  /** Every model it offers, `model` (the default) first (`BuiltIn::models`); just `model` when
   *  absent. */
  models?: readonly string[];
  key: boolean;
  /** Recognition only: the built-in service previews while recording, the sentence decoded
   *  again as it grows (`BuiltIn::asr_live_preview`, docs/dictation.md §11.8). */
  preview?: boolean;
}

export interface EngineResolveInput {
  settings: EngineSettings;
  /** Secret-store entries holding a user key (`keyEntry`). */
  userKeys: ReadonlySet<string>;
  /** For a credential of several fields, the set each stored one is of, by entry
   *  (`SecretState.credential_set`, docs/dictation.md §3.15). */
  credentialSets?: Readonly<Record<string, string>>;
  builtIn: { asr?: BuiltInService; llm?: BuiltInService };
  /** The local model the settings select, as the library sees it. */
  local: { id: string; name: string; installed: boolean };
  /** The switch is on and the library's streaming model is installed (the local source). */
  liveReady: boolean;
  /** The model library as the core sees it: what a recognition chain's models of this device
   *  resolve against (docs/dictation.md §3.14). Absent: no library (a phone). */
  library?: readonly Pick<ModelState, "id" | "capabilities" | "state">[];
  /** The selected models that ran out of quota, by service, and when they are tried again (the
   *  mock's ledger: it only ever marks the selected model, docs/dictation.md §3.5). */
  quotaOut?: Partial<Record<ServiceKind, number>>;
  /** The models each provider listed, by `provider:kind`, with the endpoint they came from
   *  (docs/dictation.md §3.3); shown while that endpoint is the saved one. */
  listed?: Partial<Record<`${ProviderId}:${ServiceKind}`, ListedModels>>;
}

/** One provider service's list (`runtime/listing.rs`). */
export interface ListedModels {
  url: string;
  models: readonly string[];
}

function trimmed(value: string | null | undefined): string | undefined {
  const t = value?.trim();
  return t === undefined || t.length === 0 ? undefined : t;
}

/** `https://api.example.com/v1` → `api.example.com`; the input itself when it does not parse. */
function hostOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url.replace(/^[a-z]+:\/\//i, "").split(/[/?#]/)[0] ?? "";
  }
}

interface Target {
  url: string;
  model: string;
  key: boolean;
}

/** Mirrors `service_target_for`: the endpoint, model and key presence, or why it cannot run;
 *  `asked` is a fallback model's own model instead of the card's (docs/dictation.md §3.5). */
function target(
  provider: ProviderId,
  kind: ServiceKind,
  input: EngineResolveInput,
  asked?: string,
): { ok: Target } | { issue: EngineIssue; model: string } {
  const spec = providerSpec(provider);
  const preset = spec[kind];
  if (preset === undefined || provider === "local") return { issue: "unavailable", model: "" };
  const choice = input.settings.providers?.[provider];
  if (provider === "builtin") {
    const service = input.builtIn[kind];
    if (service === undefined) return { issue: "unavailable", model: "" };
    const chosen = asked ?? trimmed(kind === "asr" ? choice?.asr_model : choice?.llm_model);
    return { ok: { url: "", model: builtInModel(service, chosen), key: service.key } };
  }
  const url =
    trimmed(kind === "asr" ? choice?.asr_url : choice?.llm_url) ??
    (preset.baseUrl.length > 0 ? preset.baseUrl : undefined);
  const model =
    asked ?? trimmed(kind === "asr" ? choice?.asr_model : choice?.llm_model) ?? preset.models[0];
  const entry = keyEntry(provider, kind);
  const key = entry !== undefined && input.userKeys.has(entry);
  const shown = model ?? "";
  if (url === undefined) return { issue: "url_missing", model: shown };
  if (spec.key === "required" && !key) return { issue: "key_missing", model: shown };
  if (model === undefined) return { issue: "model_missing", model: shown };
  return { ok: { url, model, key } };
}

const NO_KEY: SecretState = { set: false, source: "none" };

/** A user key's state, with the set of a credential of several fields. */
function userSecret(set: string | undefined): SecretState {
  return set === undefined
    ? { set: true, source: "user" }
    : { set: true, source: "user", credential_set: set };
}

/** `BuiltIn::models`: the built-in service's models, the default first. */
function builtInModels(service: BuiltInService): readonly string[] {
  return service.models !== undefined && service.models.length > 0
    ? service.models
    : [service.model];
}

/** `BuiltIn::service`: `chosen` when the built-in service offers it, otherwise its default. */
function builtInModel(service: BuiltInService, chosen: string | undefined): string {
  const models = builtInModels(service);
  return chosen !== undefined && models.includes(chosen) ? chosen : (models[0] ?? service.model);
}

function serviceStatus(
  provider: ProviderId,
  kind: ServiceKind,
  input: EngineResolveInput,
  active: boolean,
): ServiceStatus | undefined {
  const spec = providerSpec(provider);
  const preset = spec[kind];
  if (preset === undefined) return undefined;
  if (provider === "builtin") {
    const service = input.builtIn[kind];
    if (service === undefined) return undefined;
    const choice = input.settings.providers?.builtin;
    return {
      model: builtInModel(service, trimmed(kind === "asr" ? choice?.asr_model : choice?.llm_model)),
      presets: [...builtInModels(service)],
      key: service.key ? { set: true, source: "builtin" } : NO_KEY,
      active,
    };
  }
  if (provider === "local") {
    return {
      model: input.local.id,
      presets: [],
      key: NO_KEY,
      ...(input.local.installed ? {} : { issue: "model_not_installed" as const }),
      active,
    };
  }
  const resolved = target(provider, kind, input);
  const choice = input.settings.providers?.[provider];
  const entry = keyEntry(provider, kind);
  const userKey = entry !== undefined && input.userKeys.has(entry);
  const defaultBase = preset.baseUrl.length > 0 ? preset.baseUrl : undefined;
  const baseUrl = trimmed(kind === "asr" ? choice?.asr_url : choice?.llm_url) ?? defaultBase;
  const list = input.listed?.[`${provider}:${kind}`];
  const listed = list !== undefined && list.url === baseUrl ? list.models : [];
  return {
    model: "ok" in resolved ? resolved.ok.model : resolved.model,
    presets: [...preset.models],
    ...(baseUrl === undefined ? {} : { base_url: baseUrl }),
    ...(defaultBase === undefined ? {} : { default_base_url: defaultBase }),
    key: userKey
      ? userSecret(entry === undefined ? undefined : input.credentialSets?.[entry])
      : NO_KEY,
    ...("issue" in resolved ? { issue: resolved.issue } : {}),
    active,
    ...(listed.length === 0 ? {} : { listed: [...listed] }),
    ...(preset.sites === undefined
      ? {}
      : { sites: preset.sites.map((site) => ({ id: site.id, base_url: site.baseUrl })) }),
  };
}

/** A ledger key as `fallbackStatus` compares them: a model of this device has no endpoint. */
function keyOf(provider: ProviderId, t: Target): string {
  return JSON.stringify([provider, t.model, t.url]);
}

function localKeyOf(id: string): string {
  return keyOf("local", { url: "", model: id, key: false });
}

/** Mirrors `local_target`: an installed whole-take model of the library, or why it cannot run. */
function localIssue(
  kind: ServiceKind,
  id: string,
  input: EngineResolveInput,
): EngineIssue | undefined {
  const model = (input.library ?? []).find(
    (m) => m.id === id && m.capabilities.includes("offline"),
  );
  if (model === undefined || kind !== "asr") return "unavailable";
  return model.state.kind === "installed" ? undefined : "model_not_installed";
}

/** The selected model a chain starts from (`Primary`): its ledger key, and whether it is a model of
 *  this device. */
interface Primary {
  key: string;
  local: boolean;
}

/** A fallback model ready to stand in: whether it streams, and what it does with the glossary
 *  (`FallbackTarget::glossary_route`). */
interface StandIn {
  streams: boolean;
  glossary: GlossaryRoute;
}

/** Mirrors `fallback_plan` + `FallbackPlan::status` without a ledger (docs/dictation.md §3.5,
 *  §3.14): one row per settings entry, its problem or why it is skipped; `selected` is the model in
 *  use when it is ready. With recognition on this device (`onDevice`) every service is left out,
 *  and the chain runs only with 服务出错时也改用候补 on and a model of this device ready. */
function fallbackStatus(
  kind: ServiceKind,
  input: EngineResolveInput,
  selected: Primary | undefined,
  onDevice: boolean,
): { status: FallbackStatus; standIn: StandIn | undefined } {
  const config = fallbackSettingsOf(input.settings, kind);
  const seen = new Set<string>();
  const ready: StandIn[] = [];
  let localReady = false;
  const place = (
    key: string,
    provider: ProviderId,
    model: string,
  ): FallbackModelStatus | undefined => {
    if (key === selected?.key) return { provider, model, skip: "same_as_selected" };
    if (seen.has(key)) return { provider, model, skip: "duplicate" };
    seen.add(key);
    return undefined;
  };
  const models = config.models.map((entry): FallbackModelStatus => {
    const asked = trimmed(entry.model);
    if (entry.provider === "local") {
      const id = asked ?? "";
      const issue = localIssue(kind, id, input);
      if (issue !== undefined) return { provider: "local", model: id, issue };
      const skipped = place(localKeyOf(id), "local", id);
      if (skipped !== undefined) return skipped;
      localReady = true;
      ready.push({ streams: false, glossary: "not_supported" });
      return { provider: "local", model: id };
    }
    const r = target(entry.provider, kind, input, asked);
    let row: FallbackModelStatus;
    if (!("ok" in r)) {
      row = { provider: entry.provider, model: asked ?? r.model, issue: r.issue };
    } else {
      const model = r.ok.model;
      row = place(keyOf(entry.provider, r.ok), entry.provider, model) ?? {
        provider: entry.provider,
        model,
      };
      if (row.skip === undefined && !onDevice)
        ready.push({
          streams: asrStreams(entry.provider, r.ok.url, model),
          glossary: glossaryRoute(entry.provider, r.ok.url, model),
        });
    }
    // §3.14: recognition on this device never moves on to a service.
    return onDevice ? { provider: row.provider, model: row.model, skip: "not_for_local" } : row;
  });
  const inUse =
    config.enabled &&
    selected !== undefined &&
    (!selected.local || (config.on_outage === true && localReady));
  // `status_with(ledger)`: the selected model's retry time while the chain runs, and the model
  // standing in for it — the first fallback model, as the mock marks no other (for its quota).
  const retry = inUse ? input.quotaOut?.[kind] : undefined;
  return {
    status: {
      enabled: config.enabled,
      in_use: inUse,
      ...(retry === undefined
        ? {}
        : { selected_retry_at_ms: retry, selected_retry_reason: "quota" as const }),
      models,
    },
    standIn: retry === undefined ? undefined : ready[0],
  };
}

/** `ResolvedEngines::resolve_with_models(..).status()` for the browser preview. */
export function resolveEngineStatus(input: EngineResolveInput): EngineStatus {
  const { settings } = input;
  const asrProvider: ProviderId =
    settings.asr_provider === "builtin" && input.builtIn.asr === undefined
      ? "local"
      : settings.asr_provider;
  const llmProvider: ProviderId | undefined =
    settings.llm_provider === "builtin" && input.builtIn.llm === undefined
      ? undefined
      : settings.llm_provider;
  const userHost = (provider: ProviderId | undefined, t: Target | undefined) =>
    provider === undefined || provider === "builtin" || provider === "local" || t === undefined
      ? ""
      : hostOf(t.url);

  let asrIssue: EngineIssue | undefined;
  let asrModel: string;
  let asrTarget: Target | undefined;
  if (asrProvider === "local") {
    asrIssue = input.local.installed ? undefined : "model_not_installed";
    asrModel = input.local.name;
  } else {
    const r = target(asrProvider, "asr", input);
    if ("ok" in r) {
      asrTarget = r.ok;
      asrModel = r.ok.model;
    } else {
      asrIssue = r.issue;
      asrModel = r.model;
    }
  }
  let refineIssue: EngineIssue | undefined;
  let refineModel = "";
  let refineTarget: Target | undefined;
  if (llmProvider === undefined) {
    refineIssue = "no_provider";
  } else {
    const r = target(llmProvider, "llm", input);
    if ("ok" in r) {
      refineTarget = r.ok;
      refineModel = r.ok.model;
    } else {
      refineIssue = r.issue;
      refineModel = r.model;
    }
  }
  const providers: ProviderStatus[] = [];
  for (const id of PROVIDER_IDS) {
    const spec = providerSpec(id);
    const asr = serviceStatus(id, "asr", input, asrProvider === id);
    const llm = serviceStatus(id, "llm", input, llmProvider === id);
    if (asr === undefined && llm === undefined) continue;
    providers.push({
      id,
      key: spec.key,
      ...(spec.credential === undefined
        ? {}
        : { credential: spec.credential.map((set) => ({ id: set.id, fields: [...set.fields] })) }),
      ...(spec.listsModels === false ? { lists_models: false } : {}),
      on_device: spec.onDevice,
      console: spec.console,
      ...(spec.quota === true ? { quota: true } : {}),
      ...(asr === undefined ? {} : { asr }),
      ...(llm === undefined ? {} : { llm }),
    });
  }
  // `ResolvedEngines::live_source`: the built-in service previews itself, a realtime model streams
  // itself, else the local model.
  const liveSource: LiveSource | undefined = !settings.live_preview
    ? undefined
    : asrProvider === "builtin" && asrTarget !== undefined && input.builtIn.asr?.preview === true
      ? "cloud"
      : asrTarget !== undefined && asrStreams(asrProvider, asrTarget.url, asrTarget.model)
        ? "stream"
        : input.liveReady
          ? "local"
          : undefined;
  const asrPrimary: Primary | undefined =
    asrProvider === "local"
      ? input.local.installed
        ? { key: localKeyOf(input.local.id), local: true }
        : undefined
      : asrTarget === undefined
        ? undefined
        : { key: keyOf(asrProvider, asrTarget), local: false };
  // A phone has no library: its rows read as before (nothing of the device recognises there).
  const onDevice = asrProvider === "local" && (input.library?.length ?? 0) > 0;
  const asrFallback = fallbackStatus("asr", input, asrPrimary, onDevice);
  // §3.5: while a model taking whole recordings only stands in for the realtime one, no take
  // previews or streams; the live source stays the realtime model.
  const standIn = asrFallback.standIn;
  const liveReady =
    liveSource !== undefined &&
    (liveSource !== "stream" || standIn === undefined || standIn.streams);
  // `effective_output_mode`: a realtime model's stream is the take's text (§11.9).
  const effective =
    settings.output_mode === "whole_take" && liveReady && liveSource === "stream"
      ? "streaming_final"
      : settings.output_mode !== "whole_take" && !liveReady
        ? "whole_take"
        : settings.output_mode;
  const language = trimmed(settings.language);
  return {
    asr_provider: asrProvider,
    asr_ready: asrIssue === undefined,
    ...(asrIssue === undefined ? {} : { asr_issue: asrIssue }),
    asr_model: asrModel,
    asr_host: userHost(asrProvider, asrTarget),
    ...(asrProvider === "local" ? { local_model: input.local.id } : {}),
    local_ready: asrProvider === "local" && input.local.installed,
    live_preview_ready: liveReady,
    ...(liveSource === undefined ? {} : { live_source: liveSource }),
    effective_output_mode: effective,
    ...(language === undefined ? {} : { language }),
    refine_enabled: settings.refine_enabled,
    ...(llmProvider === undefined ? {} : { llm_provider: llmProvider }),
    refine_ready: refineIssue === undefined,
    ...(refineIssue === undefined ? {} : { refine_issue: refineIssue }),
    refine_model: refineModel,
    refine_host: userHost(llmProvider, refineTarget),
    inject: settings.inject,
    providers,
    asr_fallback: asrFallback.status,
    llm_fallback: fallbackStatus(
      "llm",
      input,
      llmProvider !== undefined && refineTarget !== undefined
        ? { key: keyOf(llmProvider, refineTarget), local: false }
        : undefined,
      false,
    ).status,
    // `ResolvedEngines::status_with`: the route of the model standing in, else the selected one's
    // (`glossary_route`): nowhere on-device, unknown without a service that runs.
    ...(standIn !== undefined
      ? { glossary: standIn.glossary }
      : asrProvider === "local"
        ? { glossary: "not_supported" as const }
        : asrTarget === undefined
          ? {}
          : { glossary: glossaryRoute(asrProvider, asrTarget.url, asrTarget.model) }),
  };
}
