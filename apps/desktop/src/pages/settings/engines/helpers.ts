import {
  DEFAULT_LOCALE,
  type EngineSettings,
  type EngineStatus,
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

/** The two views of the 语音模型 group: the providers (with the local models) and the options
 *  that apply whatever the provider. The LLM providers are their own group, AI 模型. */
export type SpeechTab = "asr" | "options";
export const SPEECH_TABS: readonly SpeechTab[] = ["asr", "options"];

export {
  LANGUAGE_CODES,
  type ProviderDraft,
  applyProviderDraft,
  checkProviderDraft,
  languageOptions,
  modelChoices,
  probeText,
  providerDraft,
  providersFor,
  serviceTarget,
  withProvider,
} from "@voltip/shared";

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
    case "import_incomplete":
      return { tone: "warn", text: t("model.state.import_incomplete") };
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
    // The manual download stays open below with what is missing; downloading still works.
    case "import_incomplete":
      return "download";
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
export type LivePreviewState = "ready" | "cloud" | "missing" | "off";

export function livePreviewState(
  settings: Pick<EngineSettings, "live_preview">,
  status: Pick<EngineStatus, "live_preview_ready" | "live_source">,
): LivePreviewState {
  if (!settings.live_preview) return "off";
  if (!status.live_preview_ready) return "missing";
  // docs/dictation.md §11.8: the built-in service previews itself, no model needed.
  return status.live_source === "cloud" ? "cloud" : "ready";
}

/** `Settings.engines` after picking a local model: on-device recognition with that model. */
export function activateLocalModel(current: EngineSettings, id: string): EngineSettings {
  return { ...current, asr_provider: "local", local_model: id };
}
