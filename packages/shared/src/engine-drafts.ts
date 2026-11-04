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
  SiteId,
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
  /** A vendor whose credential has several fields edits it here instead of `key`. */
  credential?: CredentialDraft;
}

/** A credential of several fields as a form edits it (docs/dictation.md §3.15): the set chosen and
 *  what was typed in its fields. Nothing is prefilled: a stored value never reaches the interface. */
export interface CredentialDraft {
  set: string;
  fields: Readonly<Record<string, string>>;
}

/** The credential draft a form opens with for `provider`: the set of the stored credential, else
 *  the default set, every field empty; `undefined` for a vendor with a plain key. */
export function credentialDraft(
  provider: ProviderStatus,
  service: ServiceStatus | undefined,
): CredentialDraft | undefined {
  const sets = provider.credential ?? [];
  const first = sets[0];
  if (first === undefined) return undefined;
  const stored = storedSet(service);
  return { set: sets.find((s) => s.id === stored)?.id ?? first.id, fields: {} };
}

/** The set of the credential the user saved for `service`, if one is saved. */
function storedSet(service: ServiceStatus | undefined): string | undefined {
  const key = service?.key;
  return key?.set === true && key.source === "user" ? key.credential_set : undefined;
}

/** Why a credential draft cannot be saved, by the core's rule (`merge_credentials`): with the
 *  stored set chosen a blank field keeps its stored value; with another set, or none stored, every
 *  required field of the chosen set must be filled in. */
export function checkCredentialDraft(
  draft: CredentialDraft,
  provider: ProviderStatus,
  service: ServiceStatus | undefined,
  t: TFunction = zhT.t,
): string | undefined {
  const set = provider.credential?.find((s) => s.id === draft.set);
  if (set === undefined) return t("engines.check.missingKey");
  if (storedSet(service) === draft.set) return undefined;
  const missing = set.fields.filter((f) => f.required && typed(draft, f.id) === undefined);
  if (missing.length === 0) return undefined;
  return t("engines.credential.missing", {
    fields: missing
      .map((f) => credentialFieldLabel(f.id, t))
      .join(t("engines.credential.listSeparator")),
  });
}

/** The `provider_key_set` value of a credential draft: its set and the fields filled in, trimmed;
 *  `undefined` when none is (the stored credential stays as it is). */
export function credentialValue(draft: CredentialDraft): string | undefined {
  const fields = Object.keys(draft.fields).flatMap((id) => {
    const value = typed(draft, id);
    return value === undefined ? [] : [[id, value] as const];
  });
  if (fields.length === 0) return undefined;
  return JSON.stringify({ set: draft.set, ...Object.fromEntries(fields) });
}

/** The catalogue's credential fields, sets and vendors with a note on where they are found (the
 *  keys of `engines.credential`). */
const CREDENTIAL_FIELD_IDS = [
  "app_id",
  "api_key",
  "api_secret",
  "access_token",
  "secret_id",
  "secret_key",
] as const;
const CREDENTIAL_SET_IDS = ["api_key", "app_token"] as const;
const CREDENTIAL_HELP_IDS = [
  "volcengine_speech",
  "tencent",
  "iflytek",
  "iflytek_rtasr",
  "baidu",
] as const;

function oneOf<T extends string>(ids: readonly T[], id: string): id is T {
  return (ids as readonly string[]).includes(id);
}

/** A credential field's label (`engines.credential.<id>`); the id itself for one without. */
export function credentialFieldLabel(id: string, t: TFunction = zhT.t): string {
  return oneOf(CREDENTIAL_FIELD_IDS, id) ? t(`engines.credential.${id}`) : id;
}

/** A credential set's name (`engines.credential.set.<id>`); the id itself for one without. */
export function credentialSetLabel(id: string, t: TFunction = zhT.t): string {
  return oneOf(CREDENTIAL_SET_IDS, id) ? t(`engines.credential.set.${id}`) : id;
}

/** Where a vendor's credential fields are found, for one with several. */
export function credentialHelp(provider: ProviderId, t: TFunction = zhT.t): string | undefined {
  return oneOf(CREDENTIAL_HELP_IDS, provider)
    ? t(`engines.credential.help.${provider}`)
    : undefined;
}

/** A draft's field, trimmed, when it holds something. */
function typed(draft: CredentialDraft, id: string): string | undefined {
  const value = draft.fields[id]?.trim() ?? "";
  return value.length > 0 ? value : undefined;
}

/** What a form's save and 测试连接 send as the provider's credential: the fields of a credential
 *  of several fields, else the key typed; `undefined` when nothing was typed. */
export function draftSecret(draft: ProviderDraft): string | undefined {
  if (draft.credential !== undefined) return credentialValue(draft.credential);
  const key = draft.key.trim();
  return key.length > 0 ? key : undefined;
}

/** `draft` after a save: what was typed is cleared, the set chosen stays. */
export function clearedSecret(draft: ProviderDraft): ProviderDraft {
  return draft.credential === undefined
    ? { ...draft, key: "" }
    : { ...draft, key: "", credential: { set: draft.credential.set, fields: {} } };
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
  if (provider.lists_models === false) return t("engines.field.modelHelpPresets");
  const n = service.listed?.length ?? 0;
  if (n > 0) return t("engines.field.modelListed", { n });
  return provider.key === "required"
    ? t("engines.field.modelHelp")
    : t("engines.field.modelHelpKeyless");
}

/** docs/dictation.md §3.2: the site a card's URL field points at — the default site while the
 *  field is empty, the site whose address it holds, or none for any other address (a proxy, a
 *  plan's own endpoint). `undefined` too for a vendor with one site. */
export function siteOf(service: ServiceStatus, baseUrl: string): SiteId | undefined {
  const sites = service.sites ?? [];
  const url = comparableUrl(baseUrl);
  if (sites.length === 0) return undefined;
  if (url.length === 0) return sites[0]?.id;
  return sites.find((site) => comparableUrl(site.base_url) === url)?.id;
}

/** What the URL field holds for `site`: nothing for the default site, whose address the catalogue
 *  keeps up to date, the site's address otherwise. */
export function siteUrl(service: ServiceStatus, site: SiteId): string {
  const sites = service.sites ?? [];
  const index = sites.findIndex((s) => s.id === site);
  return index <= 0 ? "" : (sites[index]?.base_url ?? "");
}

/** `https://API.example.com/v1/` and `https://api.example.com/v1` are one address. */
function comparableUrl(url: string): string {
  return url.trim().replace(/\/+$/, "").toLowerCase();
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
  const credential = draft.credential ?? credentialDraft(provider, service);
  if (credential !== undefined) {
    const issue = checkCredentialDraft(credential, provider, service, t);
    if (issue !== undefined) return issue;
  } else if (provider.key === "required" && !service?.key.set && draft.key.trim().length === 0)
    return t("engines.check.missingKey");
  const model = draft.model.trim().length > 0 ? draft.model : (service?.model ?? "");
  if (model.trim().length === 0) return t("engines.check.missingModel");
  return undefined;
}

/** What stops 测试连接 before the core is asked (docs/dictation.md §3.15): a credential of several
 *  fields missing a field of the set shown, by `checkCredentialDraft`'s rule. Left blank, the
 *  stored set is tested. A single key is not checked here: blank, the core tests the saved key or
 *  answers that none is saved. */
export function checkProbeDraft(
  draft: ProviderDraft,
  provider: ProviderStatus,
  kind: ServiceKind,
  t: TFunction = zhT.t,
): string | undefined {
  const service = provider[kind];
  const credential = draft.credential ?? credentialDraft(provider, service);
  return credential === undefined
    ? undefined
    : checkCredentialDraft(credential, provider, service, t);
}

const HTTP_URL_PATTERN = /^https?:\/\/[^/\s]+/;

/** The probe answer as one line and a lamp tone. For a vendor that lists no models (`checked`)
 *  测试连接 checked the credential, and says so instead of counting the presets it returns. */
export function probeText(
  report: ProbeReport,
  t: TFunction = zhT.t,
  checked = false,
): { text: string; ok: boolean } {
  if (report.result === "ok")
    return {
      ok: true,
      text: checked
        ? t("engines.probeResult.okChecked", { ms: report.latency_ms ?? 0 })
        : t("engines.probeResult.ok", {
            n: report.models?.length ?? 0,
            ms: report.latency_ms ?? 0,
          }),
    };
  const reason = report.reason ?? "unreachable";
  const result = t(`engines.probeResult.${reason}`, { status: report.status ?? 0 });
  // docs/dictation.md §3.12: what the service said, so the user knows what to fix.
  const detail = report.detail?.trim();
  return {
    ok: false,
    text:
      detail === undefined || detail.length === 0
        ? result
        : t("engines.probeResult.withDetail", { result, detail }),
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
