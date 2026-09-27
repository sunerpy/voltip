import {
  DEFAULT_LOCALE,
  type EngineSettings,
  type EngineStatus,
  type ProbeReport,
  type ProviderId,
  type ProviderSettings,
  type ProviderStatus,
  type ServiceKind,
  type ServiceStatus,
  type Locale,
  MODEL_TIERS,
  type ModelInstallState,
  type ModelState,
  type ModelTier,
  type TFunction,
  isRecognitionModel,
  isStreamingModel,
  modelDescription as sharedModelDescription,
  zhT,
} from "@voltip/shared";
import type { BadgeTone } from "@voltip/ui";

export type EngineTab = "asr" | "llm" | "options";
export const ENGINE_TABS: readonly EngineTab[] = ["asr", "llm", "options"];

/** The providers that offer `kind` in this build, in display order. */
export function providersFor(status: EngineStatus, kind: ServiceKind): ProviderStatus[] {
  return status.providers.filter((p) => p[kind] !== undefined);
}

/** What one provider card edits for one service; `key` is never prefilled. */
export interface ProviderDraft {
  model: string;
  baseUrl: string;
  key: string;
}

/** The draft a card opens with: the user's saved choices (never the presets, which are the
 *  placeholders) and an empty key. */
export function providerDraft(
  settings: EngineSettings,
  provider: ProviderId,
  kind: ServiceKind,
): ProviderDraft {
  const saved = settings.providers?.[provider];
  return {
    model: (kind === "asr" ? saved?.asr_model : saved?.llm_model) ?? "",
    baseUrl: (kind === "asr" ? saved?.asr_url : saved?.llm_url) ?? "",
    key: "",
  };
}

/** Fold one card's draft into the full `EngineSettings` (`settings_set_engines` is all or nothing).
 *  Empty fields remove the override; a provider left without overrides leaves the map. */
export function applyProviderDraft(
  settings: EngineSettings,
  provider: ProviderId,
  kind: ServiceKind,
  draft: Pick<ProviderDraft, "model" | "baseUrl">,
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

export const LANGUAGE_CODES = ["", "zh", "en", "yue", "ja", "ko"] as const;

/** Recognition language hints: the auto-detect label is localized, the language names are their own. */
export function languageOptions(t: TFunction = zhT.t): { value: string; label: string }[] {
  return LANGUAGE_CODES.map((value) => ({
    value,
    label: value === "" ? t("language.auto") : t(`language.${value}`),
  }));
}

// ---- local models (docs/dictation.md §10) ----------------------------------------------------

/** `239549735` → `239.5 MB`; below a megabyte the kilobytes (`315.9 KB`). */
export function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return `${(bytes / 1_000).toFixed(1)} KB`;
}

/** Download progress as a fraction in 0..1 (`0` for an unknown total). */
export function downloadFraction(state: ModelInstallState): number {
  if (state.kind !== "downloading" || state.total <= 0) return 0;
  return Math.min(1, state.received / state.total);
}

export type StateTone = "ok" | "warn" | "idle" | "danger" | "accent";

/** Badge text and tone for one model's install state (the download percentage while streaming). */
export function modelStateCell(
  state: ModelInstallState,
  t: TFunction = zhT.t,
): { tone: StateTone; text: string } {
  switch (state.kind) {
    case "installed":
      return { tone: "ok", text: t("model.state.installed") };
    case "downloading":
      return {
        tone: "warn",
        text: t("model.state.downloading", {
          percent: Math.round(downloadFraction(state) * 100),
        }),
      };
    case "verifying":
      return { tone: "accent", text: t("model.state.verifying") };
    case "not_installed":
      return { tone: "idle", text: t("model.state.not_installed") };
    case "failed":
      return { tone: "danger", text: t("model.state.failed") };
  }
}

/** Lamp tone of `modelStateCell` → badge tone (idle has no lamp, so it is the neutral chip). */
export const STATE_BADGE_TONE: Readonly<Record<StateTone, BadgeTone>> = {
  ok: "ok",
  warn: "warn",
  idle: "neutral",
  danger: "danger",
  accent: "accent",
};

/** What the primary button on a model card does, per install state. `installed` is the streaming
 *  preview model once on disk: it is not a recognition model, so there is nothing to activate. */
export type ModelAction = "download" | "cancel" | "retry" | "use" | "current" | "installed";

export function modelAction(model: ModelState): ModelAction {
  switch (model.state.kind) {
    case "not_installed":
      return "download";
    case "downloading":
    case "verifying":
      return "cancel";
    case "failed":
      return "retry";
    case "installed":
      if (!isRecognitionModel(model)) return "installed";
      return model.active ? "current" : "use";
  }
}

/** The core's own catalogue text, localised by id when the dictionary knows the model. */
export function modelDescription(model: ModelState, locale: Locale = DEFAULT_LOCALE): string {
  return sharedModelDescription(model.id, model.description, locale);
}

/** Languages a model card shows before it folds the rest into「等」. */
export const CARD_LANGUAGES = 5;

/** The card's language badge: every language up to [`CARD_LANGUAGES`], else the first ones and
 *  「等」(`zh · en · ja · ko · yue 等`), with the rest in `title`. No count: the catalogue lists a
 *  model's main languages, not all of them (Qwen3-ASR lists 10 of its 30). A ten-language badge
 *  does not wrap and pushed the card out of its grid column (2026-09-27). */
export function languageSummary(
  languages: readonly string[],
  t: TFunction = zhT.t,
): { text: string | undefined; title: string | undefined } {
  if (languages.length <= CARD_LANGUAGES) return { text: languages.join(" · "), title: undefined };
  return {
    text: t("model.moreLanguages", { shown: languages.slice(0, CARD_LANGUAGES).join(" · ") }),
    title: t("model.allLanguages", { languages: languages.join(", ") }),
  };
}

/** `sense_voice` → `SenseVoice`, `transcribe_cpp` → `transcribe.cpp`. */
export function modelEngineLabel(engine: ModelState["engine"], t: TFunction = zhT.t): string {
  return t(`model.engine.${engine}`);
}

/** Display order of the product tiers (docs/dictation.md §10): 均衡 → 高精度 → 轻量; the streaming
 *  tier is shown in its own block, never among the recognition models. */
export const TIER_ORDER: readonly ModelTier[] = MODEL_TIERS;

/** The recognition models (`capabilities` includes `offline`) grouped by tier order, catalogue
 *  order within a tier (`轻量` before `轻量 · 中文`). */
export function offlineModelsByTier(models: readonly ModelState[]): ModelState[] {
  const offline = models.filter((m) => isRecognitionModel(m));
  return TIER_ORDER.flatMap((tier) => offline.filter((m) => m.tier === tier));
}

/** The live-preview model (`capabilities: ["streaming"]`); `undefined` on a core without one. */
export function streamingModel(models: readonly ModelState[]): ModelState | undefined {
  return models.find((m) => isStreamingModel(m));
}

/** What the 实时预览 block says: the switch is off, or the model is missing, or it is ready. */
export type LivePreviewState = "ready" | "missing" | "off";

export function livePreviewState(
  settings: Pick<EngineSettings, "live_preview">,
  status: Pick<EngineStatus, "live_preview_ready">,
): LivePreviewState {
  if (!settings.live_preview) return "off";
  return status.live_preview_ready ? "ready" : "missing";
}

/** `Settings.engines` after picking a local model: on-device recognition with that model. */
export function activateLocalModel(current: EngineSettings, id: string): EngineSettings {
  return { ...current, asr_provider: "local", local_model: id };
}
