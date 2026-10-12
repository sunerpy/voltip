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

/** The suggested models of a service: the one in effect, the presets, then `probed`, without
 *  duplicates. The title bar's quick menus offer these; the card's select also offers every model
 *  the provider lists (`modelOptions`). */
export function modelChoices(service: ServiceStatus, probed: readonly string[] = []): string[] {
  const out: string[] = [];
  for (const m of [service.model, ...service.presets, ...probed])
    if (m.length > 0 && !out.includes(m)) out.push(m);
  return out;
}

/** One option of a card's model select; `group` heads the options of one kind. */
export interface ModelOption {
  value: string;
  label: string;
  group?: string;
}

/** The options of a card's model select (docs/dictation.md §3.3): the model in effect and the
 *  presets, then `other` (其他模型…, for a select that has it), then, once the provider has listed
 *  its models (`ServiceStatus.listed`, or what a 测试连接 in the form found, `probed`), those as a
 *  group headed with their count, the count 测试连接 reports; a suggested model the provider lists
 *  shows twice. A list only ever adds options after the ones shown: WebKit's native select reports
 *  a choice by its position, so a list arriving while it is open must not move what it shows. */
export function modelOptions(
  service: ServiceStatus,
  probed: readonly string[] = [],
  t: TFunction = zhT.t,
  other?: ModelOption,
): ModelOption[] {
  const suggested = uniqueIds([service.model, ...service.presets]).map((m) => modelOption(m));
  const offered = uniqueIds(probed.length > 0 ? probed : (service.listed ?? []));
  offered.sort();
  const group = t("engines.field.modelOffered", { n: offered.length });
  return [
    ...suggested,
    ...(other === undefined ? [] : [other]),
    ...offered.map((m) => modelOption(m, group)),
  ];
}

/** `ids` without blanks and repeats, in their order. */
function uniqueIds(ids: readonly string[]): string[] {
  return ids.filter((m, i) => m.length > 0 && ids.indexOf(m) === i);
}

function modelOption(model: string, group?: string): ModelOption {
  return group === undefined
    ? { value: model, label: model }
    : { value: model, label: model, group };
}

/** The line under a card's model select (docs/dictation.md §3.3): how many models the provider
 *  lists for this service (测试连接's count), or where the list's models come from. */
export function modelListHelp(
  provider: ProviderStatus,
  service: ServiceStatus,
  t: TFunction = zhT.t,
): string {
  const n = service.listed?.length ?? 0;
  if (n > 0) return t("engines.field.modelListed", { n });
  return provider.key === "required"
    ? t("engines.field.modelHelp")
    : t("engines.field.modelHelpKeyless");
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

/** `https://api.groq.com/openai/v1` → `api.groq.com`, `http://127.0.0.1:11434/v1` →
 *  `127.0.0.1:11434`: the host and port of an http(s) address, without a user name; `undefined`
 *  for anything else. Unlike `labels.ts`' `hostOf`, a user name and password in the address never
 *  come out (React Native's `URL` throws on `host`, and its fallback would keep them). */
export function endpointHost(url: string): string | undefined {
  return /^https?:\/\/(?:[^@/?#\s]*@)?([^/?#\s]+)/i.exec(url.trim())?.[1];
}

/** The line under a provider card's name: the host its requests go to, or what it is when it has
 *  no address to show — the built-in service (its host is never shown), the on-device models, and
 *  a custom endpoint not filled in yet. */
export function providerTagline(
  provider: ProviderStatus,
  kind: ServiceKind,
  t: TFunction = zhT.t,
): string {
  if (provider.id === "builtin") return t("engines.tagline.builtin");
  if (provider.id === "local") return t("engines.tagline.local");
  const url = provider[kind]?.base_url;
  return (url === undefined ? undefined : endpointHost(url)) ?? t("engines.tagline.custom");
}

export type ProviderBadgeTone = "accent" | "ok" | "warn";

/** What a provider card says of one service: 使用中 on the one in use, then why it cannot run, or
 *  就绪 on one that can and is not in use. */
export function providerBadges(
  service: ServiceStatus,
  t: TFunction = zhT.t,
): { tone: ProviderBadgeTone; text: string }[] {
  const out: { tone: ProviderBadgeTone; text: string }[] = [];
  if (service.active) out.push({ tone: "accent", text: t("engines.inUse") });
  if (service.issue !== undefined)
    out.push({ tone: "warn", text: t(`engines.issue.${service.issue}`) });
  else if (!service.active) out.push({ tone: "ok", text: t("engines.ready") });
  return out;
}

export const LANGUAGE_CODES = ["", "zh", "en", "yue", "ja", "ko"] as const;

/** Recognition language hints: the auto-detect label is localized, the language names are their own. */
export function languageOptions(t: TFunction = zhT.t): { value: string; label: string }[] {
  return LANGUAGE_CODES.map((value) => ({
    value,
    label: value === "" ? t("language.auto") : t(`language.${value}`),
  }));
}
