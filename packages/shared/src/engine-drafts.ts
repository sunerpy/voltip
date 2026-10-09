// The provider cards' editing logic, shared by the desktop's engines pane and the phone's settings
// (user decision 2026-10-01: the phone configures its own providers): what a card's draft holds,
// how it folds into `EngineSettings` (`settings_set_engines` is all or nothing), the model choices,
// the checks before a save and the probe's answer as a line. Moved here from
// `apps/desktop/src/pages/settings/engines/helpers.ts`, which re-exports it.
import { zhT, type TFunction } from "./i18n";
import type {
  EngineSettings,
  EngineStatus,
  LlmApi,
  ProbeReport,
  ProviderId,
  ProviderSettings,
  ProviderStatus,
  ReasoningEffort,
  ServiceKind,
  ServiceStatus,
} from "./schema";

/** The providers that offer `kind` in this build, in display order. */
export function providersFor(status: EngineStatus, kind: ServiceKind): ProviderStatus[] {
  return status.providers.filter((p) => p[kind] !== undefined);
}

/** What one provider card edits for one service; `key` is never prefilled. `api` and
 *  `reasoning` belong to the custom provider's clean-up card alone (docs/dictation.md §3.7). */
export interface ProviderDraft {
  model: string;
  baseUrl: string;
  key: string;
  api?: LlmApi;
  /** `undefined` sends no effort. */
  reasoning?: ReasoningEffort;
}

/** Whether a card chooses its interface: the custom provider's clean-up (§3.7). */
export function choosesInterface(provider: ProviderId, kind: ServiceKind): boolean {
  return provider === "custom" && kind === "llm";
}

/** The draft a card opens with: the user's saved choices (never the presets, which are the
 *  placeholders) and an empty key. */
export function providerDraft(
  settings: EngineSettings,
  provider: ProviderId,
  kind: ServiceKind,
): ProviderDraft {
  const saved = settings.providers?.[provider];
  const draft: ProviderDraft = {
    model: (kind === "asr" ? saved?.asr_model : saved?.llm_model) ?? "",
    baseUrl: (kind === "asr" ? saved?.asr_url : saved?.llm_url) ?? "",
    key: "",
  };
  if (choosesInterface(provider, kind)) {
    draft.api = saved?.llm_api ?? "chat_completions";
    if (saved?.llm_reasoning !== undefined) draft.reasoning = saved.llm_reasoning;
  }
  return draft;
}

/** Fold one card's draft into the full `EngineSettings` (`settings_set_engines` is all or nothing).
 *  Empty fields remove the override; a provider left without overrides leaves the map. */
export function applyProviderDraft(
  settings: EngineSettings,
  provider: ProviderId,
  kind: ServiceKind,
  draft: Pick<ProviderDraft, "model" | "baseUrl" | "api" | "reasoning">,
): EngineSettings {
  const current: ProviderSettings = { ...settings.providers?.[provider] };
  const model = draft.model.trim();
  const url = draft.baseUrl.trim();
  const modelKey = kind === "asr" ? "asr_model" : "llm_model";
  const urlKey = kind === "asr" ? "asr_url" : "llm_url";
  if (model.length > 0) current[modelKey] = model;
  else delete current[modelKey];
  if (url.length > 0) current[urlKey] = url;
  else delete current[urlKey];
  if (choosesInterface(provider, kind)) {
    // Chat completions is the default and is not written; the effort goes with Responses only.
    if (draft.api === "responses") current.llm_api = "responses";
    else delete current.llm_api;
    if (draft.api === "responses" && draft.reasoning !== undefined)
      current.llm_reasoning = draft.reasoning;
    else delete current.llm_reasoning;
  }
  const providers: Partial<Record<ProviderId, ProviderSettings>> = { ...settings.providers };
  if (Object.keys(current).length > 0) providers[provider] = current;
  else delete providers[provider];
  const next: EngineSettings = { ...settings };
  if (Object.keys(providers).length > 0) next.providers = providers;
  else delete next.providers;
  return next;
}

/** `settings` with `provider` serving `kind`. */
export function withProvider(
  settings: EngineSettings,
  kind: ServiceKind,
  provider: ProviderId,
): EngineSettings {
  return kind === "asr"
    ? { ...settings, asr_provider: provider }
    : { ...settings, llm_provider: provider };
}

/** The models a card's select offers: the one in effect, the presets, then what the provider
 *  listed (`probed`), without duplicates. */
export function modelChoices(service: ServiceStatus, probed: readonly string[] = []): string[] {
  const out: string[] = [];
  for (const m of [service.model, ...service.presets, ...probed])
    if (m.length > 0 && !out.includes(m)) out.push(m);
  return out;
}

/** Client-side checks before a save (the core validates again). */
export function checkProviderDraft(
  draft: ProviderDraft,
  provider: ProviderStatus,
  kind: ServiceKind,
  t: TFunction = zhT.t,
): string | undefined {
  const service = provider[kind];
  const url = draft.baseUrl.trim();
  if (url.length > 0 && !HTTP_URL_PATTERN.test(url)) return t("engines.check.badUrl");
  if (url.length === 0 && service?.default_base_url === undefined)
    return t("engines.check.missingUrl");
  if (provider.key === "required" && !service?.key.set && draft.key.trim().length === 0)
    return t("engines.check.missingKey");
  const model = draft.model.trim().length > 0 ? draft.model : (service?.model ?? "");
  if (model.trim().length === 0) return t("engines.check.missingModel");
  return undefined;
}

const HTTP_URL_PATTERN = /^https?:\/\/[^/\s]+/;

/** The probe answer as one line and a lamp tone. */
export function probeText(
  report: ProbeReport,
  t: TFunction = zhT.t,
): { text: string; ok: boolean } {
  if (report.result === "ok")
    return {
      ok: true,
      text: t("engines.probeResult.ok", {
        n: report.models?.length ?? 0,
        ms: report.latency_ms ?? 0,
      }),
    };
  const reason = report.reason ?? "unreachable";
  return {
    ok: false,
    text: t(`engines.probeResult.${reason}`, { status: report.status ?? 0 }),
  };
}

/** Where a service sends its data, for the privacy lines: nothing for on-device, the built-in
 *  service by name (its host is never shown), a user endpoint by host. */
export function serviceTarget(
  provider: ProviderId | undefined,
  host: string,
  t: TFunction = zhT.t,
): string | undefined {
  if (provider === undefined || provider === "local") return undefined;
  if (provider === "builtin") return t("engines.provider.builtin");
  if (provider === "ollama") return undefined;
  return host.length > 0 ? host : t(`engines.provider.${provider}`);
}

/** `Qwen/Qwen3-ASR-1.7B` → `Qwen3-ASR-1.7B`: the model id without its vendor prefix. */
export function shortModel(model: string): string {
  const tail = model.split("/").at(-1) ?? model;
  return tail.length > 0 ? tail : model;
}

export const LANGUAGE_CODES = ["", "zh", "en", "yue", "ja", "ko"] as const;

/** Recognition language hints: the auto-detect label is localized, the language names are their own. */
export function languageOptions(t: TFunction = zhT.t): { value: string; label: string }[] {
  return LANGUAGE_CODES.map((value) => ({
    value,
    label: value === "" ? t("language.auto") : t(`language.${value}`),
  }));
}
