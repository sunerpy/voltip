// The provider catalogue on the TypeScript side (`voltip_core::providers::PROVIDERS`) and the
// resolution the browser preview runs in place of the core (`ResolvedEngines::status`). The Rust
// catalogue is the source of truth: `ipc-contract.test.ts` compares this copy with the provider
// cards in the Rust-generated fixtures, so a drift fails the build.
import { fallbackSettingsOf } from "./fallback";
import type {
  EngineIssue,
  EngineSettings,
  EngineStatus,
  FallbackModelStatus,
  FallbackStatus,
  KeyPolicy,
  ProviderId,
  ProviderStatus,
  SecretState,
  ServiceKind,
  ServiceStatus,
  LiveSource,
} from "./schema";
import { PROVIDER_IDS } from "./schema";

/** A service a vendor offers: its public base URL and the models suggested first. */
export interface ServicePreset {
  baseUrl: string;
  models: readonly string[];
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
}

const NO_PRESET: ServicePreset = { baseUrl: "", models: [] };

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
      models: [
        "gemini-3.8-flash",
        "gemini-3.5-flash-lite",
        "gemini-3.1-flash-lite",
        "gemini-2.5-flash",
        "gemini-2.5-pro",
      ],
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
        "qwen-audio-3.1-asr-flash-message",
        "qwen3-asr-flash",
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

/** `AsrProtocol::of(url, model).streams()` (docs/dictation.md §3.4): a Model Studio realtime model,
 *  which recognises while the take is spoken. */
export function asrStreams(url: string, model: string): boolean {
  if (!isDashscope(url)) return false;
  const m = model.trim().toLowerCase();
  if (m.includes("filetrans") || m.startsWith("qwen3-asr")) return false;
  const realtime = m.includes("realtime");
  if (m.startsWith("qwen-audio") && m.includes("-asr"))
    return realtime || m.includes("-streaming") || m.includes("-message");
  return (m.startsWith("fun-asr") || m.startsWith("paraformer")) && realtime;
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
      return `provider-key.${provider}`;
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
  builtIn: { asr?: BuiltInService; llm?: BuiltInService };
  /** The local model the settings select, as the library sees it. */
  local: { id: string; name: string; installed: boolean };
  /** The switch is on and the library's streaming model is installed (the local source). */
  liveReady: boolean;
  /** The selected models that ran out of quota, by service, and when they are tried again (the
   *  mock's ledger: it only ever marks the selected model, docs/dictation.md §3.5). */
  quotaOut?: Partial<Record<ServiceKind, number>>;
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
  return {
    model: "ok" in resolved ? resolved.ok.model : resolved.model,
    presets: [...preset.models],
    ...(baseUrl === undefined ? {} : { base_url: baseUrl }),
    ...(defaultBase === undefined ? {} : { default_base_url: defaultBase }),
    key: userKey ? { set: true, source: "user" } : NO_KEY,
    ...("issue" in resolved ? { issue: resolved.issue } : {}),
    active,
  };
}

/** Mirrors `fallback_plan` + `FallbackPlan::status` without a ledger (docs/dictation.md §3.5):
 *  one row per settings entry, its problem or why it is skipped; `selected` is the provider and
 *  service in use when it is remote and ready. */
function fallbackStatus(
  kind: ServiceKind,
  input: EngineResolveInput,
  selected: { provider: ProviderId; target: Target } | undefined,
): { status: FallbackStatus; standIn: Target | undefined } {
  const config = fallbackSettingsOf(input.settings, kind);
  const keyOf = (provider: ProviderId, t: Target) => JSON.stringify([provider, t.model, t.url]);
  const selectedKey =
    selected === undefined ? undefined : keyOf(selected.provider, selected.target);
  const seen = new Set<string>();
  const ready: Target[] = [];
  const models = config.models.map((entry): FallbackModelStatus => {
    const asked = trimmed(entry.model);
    const r = target(entry.provider, kind, input, asked);
    if (!("ok" in r))
      return {
        provider: entry.provider,
        model: asked ?? r.model,
        issue: r.issue,
      };
    const key = keyOf(entry.provider, r.ok);
    const model = r.ok.model;
    if (key === selectedKey) return { provider: entry.provider, model, skip: "same_as_selected" };
    if (seen.has(key)) return { provider: entry.provider, model, skip: "duplicate" };
    seen.add(key);
    ready.push(r.ok);
    return { provider: entry.provider, model };
  });
  const inUse = config.enabled && selectedKey !== undefined;
  // `status_with(ledger)`: the selected model's retry time while the chain runs, and the model
  // standing in for it — the first fallback model, as the mock marks no other.
  const retry = inUse ? input.quotaOut?.[kind] : undefined;
  return {
    status: {
      enabled: config.enabled,
      in_use: inUse,
      ...(retry === undefined ? {} : { selected_retry_at_ms: retry }),
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
      on_device: spec.onDevice,
      console: spec.console,
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
      : asrTarget !== undefined && asrStreams(asrTarget.url, asrTarget.model)
        ? "stream"
        : input.liveReady
          ? "local"
          : undefined;
  const asrFallback = fallbackStatus(
    "asr",
    input,
    asrProvider !== "local" && asrTarget !== undefined
      ? { provider: asrProvider, target: asrTarget }
      : undefined,
  );
  // §3.5: while a model taking whole recordings only stands in for the realtime one, no take
  // previews or streams; the live source stays the realtime model.
  const standIn = asrFallback.standIn;
  const liveReady =
    liveSource !== undefined &&
    (liveSource !== "stream" || standIn === undefined || asrStreams(standIn.url, standIn.model));
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
        ? { provider: llmProvider, target: refineTarget }
        : undefined,
    ).status,
  };
}
