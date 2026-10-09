import {
  type EngineSettings,
  type EngineStatus,
  type ProviderId,
  type TFunction,
  providerStatus,
} from "@voltip/shared";
import { Badge, Input, Select, Toggle, useI18n, useUiState } from "@voltip/ui";
import { shortModel } from "../shell/page-meta";
import {
  applyProviderDraft,
  modelChoices,
  providerDraft,
  providersFor,
} from "./settings/engines/helpers";
import { ModelCard } from "./settings/engines/LocalModels";

/** Step 3 choices: the build's own service, an on-device model, or another provider. */
export type EngineChoice = "builtin" | "local" | "provider";

/** What step 3 edits beyond the choice. The key is written through `provider_key_set` only. */
export interface EngineDraft {
  /** The vendor of the `provider` choice. */
  provider: ProviderId;
  model: string;
  baseUrl: string;
  key: string;
  /** Polish with an LLM too (when the choice has a polish provider). */
  refine: boolean;
}

/** The vendors step 3 offers under 其他服务商: every remote recognition provider but the built-in. */
export function vendorsFor(engines: EngineStatus): ProviderId[] {
  return providersFor(engines, "asr")
    .map((p) => p.id)
    .filter((id) => id !== "builtin" && id !== "local");
}

/** The choice step 3 opens with: what the settings already use. */
export function initialChoice(engines: EngineStatus): EngineChoice {
  if (engines.asr_provider === "local") return "local";
  if (engines.asr_provider === "builtin") return "builtin";
  return "provider";
}

/** The draft step 3 opens with: the provider the settings use (else the first vendor) with the
 *  model and endpoint saved for it, so saving again keeps them. */
export function initialDraft(settings: EngineSettings, engines: EngineStatus): EngineDraft {
  const provider =
    engines.asr_provider !== "builtin" && engines.asr_provider !== "local"
      ? engines.asr_provider
      : (vendorsFor(engines)[0] ?? "custom");
  return {
    provider,
    ...providerDraft(settings, provider, "asr"),
    refine: settings.refine_enabled,
  };
}

/** Who would polish the text for a choice: the vendor itself when it offers polish, else the
 *  built-in service when the build has it, else nobody. */
export function refineProviderFor(
  choice: EngineChoice,
  draft: EngineDraft,
  engines: EngineStatus,
): ProviderId | undefined {
  if (choice === "provider" && providerStatus(engines, draft.provider)?.llm !== undefined)
    return draft.provider;
  return providerStatus(engines, "builtin")?.llm !== undefined ? "builtin" : undefined;
}

/** The `Settings.engines` block a choice writes; everything else in the block is kept. */
export function engineSettingsFor(
  choice: EngineChoice,
  current: EngineSettings,
  draft: EngineDraft,
  engines: EngineStatus,
): EngineSettings {
  const refiner = refineProviderFor(choice, draft, engines);
  const withRefine = (next: EngineSettings): EngineSettings =>
    refiner === undefined
      ? { ...next, refine_enabled: false }
      : { ...next, llm_provider: refiner, refine_enabled: draft.refine };
  switch (choice) {
    case "builtin":
      return withRefine({ ...current, asr_provider: "builtin" });
    case "local":
      // No model named yet = the catalogue default, the recommended one.
      return withRefine({ ...current, asr_provider: "local" });
    case "provider": {
      const next = applyProviderDraft(current, draft.provider, "asr", draft);
      return withRefine({ ...next, asr_provider: draft.provider });
    }
  }
}

/** Why 保存并继续 is not possible yet, if it is not. */
export function engineDraftProblem(
  choice: EngineChoice,
  draft: EngineDraft,
  engines: EngineStatus,
  t: TFunction,
): string | undefined {
  if (choice !== "provider") return undefined;
  const provider = providerStatus(engines, draft.provider);
  const service = provider?.asr;
  if (provider === undefined || service === undefined) return undefined;
  const url = draft.baseUrl.trim();
  if (
    (url.length > 0 && !/^https?:\/\/[^/\s]+/.test(url)) ||
    (url.length === 0 && service.default_base_url === undefined)
  )
    return t("onboarding.engine.needUrl");
  if (provider.key === "required" && !service.key.set && draft.key.trim().length === 0)
    return t("onboarding.engine.needKey");
  // What the save leaves in effect: an empty field drops the saved override and falls back to the
  // provider's default model, which the custom endpoint does not have (the core would refuse every take).
  const model = draft.model.trim().length > 0 ? draft.model.trim() : (service.presets[0] ?? "");
  if (model.length === 0) return t("onboarding.engine.needModel");
  return undefined;
}

export interface OnboardingEngineStepProps {
  choice: EngineChoice;
  onChoice: (choice: EngineChoice) => void;
  draft: EngineDraft;
  onDraft: (draft: EngineDraft) => void;
}

/** Step 3 of the setup guide: three radio cards, the chosen one's details underneath (the recommended local
 *  model with its download button; the vendor, model, endpoint and key), and the polish switch. */
export function OnboardingEngineStep({
  choice,
  onChoice,
  draft,
  onDraft,
}: OnboardingEngineStepProps) {
  const { t, locale } = useI18n();
  const state = useUiState();
  const engines = state.engines;
  const builtin = providerStatus(engines, "builtin")?.asr;
  const recommended = state.models.find((m) => m.recommended);
  const options: { id: EngineChoice; title: string; subtitle: string; recommended: boolean }[] = [
    ...(builtin === undefined
      ? []
      : [
          {
            id: "builtin" as const,
            title: t("onboarding.engine.builtin"),
            subtitle: t("onboarding.engine.builtinSubtitle", { model: shortModel(builtin.model) }),
            recommended: true,
          },
        ]),
    {
      id: "local",
      title: t("onboarding.engine.local"),
      subtitle: t("onboarding.engine.localSubtitle"),
      recommended: builtin === undefined,
    },
    {
      id: "provider",
      title: t("onboarding.engine.provider"),
      subtitle: t("onboarding.engine.providerSubtitle"),
      recommended: false,
    },
  ];
  const vendor = providerStatus(engines, draft.provider);
  const vendorAsr = vendor?.asr;
  const refiner = refineProviderFor(choice, draft, engines);
  return (
    <div className="flex flex-col gap-3">
      <div
        role="radiogroup"
        aria-label={t("onboarding.engine.group")}
        className="flex flex-col gap-2">
        {options.map((opt) => {
          const selected = choice === opt.id;
          return (
            <button
              key={opt.id}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => {
                onChoice(opt.id);
              }}
              className={`flex items-start gap-3 rounded-10 bg-surface p-3 text-left ${selected ? "border-2 border-primary" : "hairline"}`}>
              <span
                className={`mt-1 inline-block h-3.5 w-3.5 rounded-full border ${selected ? "border-[4px] border-primary" : "border-border-strong"}`}
              />
              <span className="flex-1">
                <span className="flex items-center gap-2 text-[14px] font-semibold text-fg">
                  {opt.title}
                  {opt.recommended && <Badge tone="ok">{t("onboarding.engine.recommended")}</Badge>}
                </span>
                <span className="block text-[12px] text-fg-muted">{opt.subtitle}</span>
              </span>
            </button>
          );
        })}
      </div>
      {choice === "local" && recommended !== undefined && (
        <div className="flex flex-col gap-2" data-testid="onboarding-local">
          <ModelCard model={recommended} />
          {recommended.state.kind !== "installed" && (
            <p className="text-[11px] text-fg-subtle">{t("onboarding.engine.localDownload")}</p>
          )}
        </div>
      )}
      {choice === "provider" && vendorAsr !== undefined && (
        <div className="flex flex-col gap-2" data-testid="onboarding-provider">
          <div className="grid grid-cols-2 gap-2">
            <Select
              label={t("onboarding.engine.providerLabel")}
              size="sm"
              value={draft.provider}
              onChange={(provider) => {
                onDraft({
                  ...draft,
                  provider,
                  ...providerDraft(state.settings.engines, provider, "asr"),
                });
              }}
              options={vendorsFor(engines).map((id) => ({
                value: id,
                label: t(`engines.provider.${id}`),
              }))}
            />
            {/* A provider without presets (the custom endpoint) takes any model name. */}
            {vendorAsr.presets.length > 0 ? (
              <Select
                label={t("onboarding.engine.modelLabel")}
                size="sm"
                mono
                value={draft.model.length > 0 ? draft.model : vendorAsr.model}
                onChange={(model) => {
                  onDraft({ ...draft, model });
                }}
                options={modelChoices(vendorAsr).map((m) => ({ value: m, label: m }))}
              />
            ) : (
              <Input
                label={t("onboarding.engine.modelLabel")}
                mono
                size="sm"
                value={draft.model}
                onChange={(e) => {
                  onDraft({ ...draft, model: e.target.value });
                }}
              />
            )}
          </div>
          {vendorAsr.default_base_url === undefined && (
            <Input
              label={t("onboarding.engine.baseUrl")}
              mono
              size="sm"
              value={draft.baseUrl}
              placeholder="http://192.168.1.20:8000/v1"
              onChange={(e) => {
                onDraft({ ...draft, baseUrl: e.target.value });
              }}
            />
          )}
          {vendor?.key !== "none" && (
            <Input
              label={t("onboarding.engine.key")}
              mono
              size="sm"
              type="password"
              autoComplete="off"
              value={draft.key}
              placeholder={t("onboarding.engine.keyPlaceholder")}
              onChange={(e) => {
                onDraft({ ...draft, key: e.target.value });
              }}
            />
          )}
        </div>
      )}
      <div className="flex items-center justify-between gap-3 rounded-10 bg-inset px-3 py-2">
        <div className="min-w-0">
          <div className="text-[13px] text-fg">{t("onboarding.engine.refine")}</div>
          <div className="text-[11px] text-fg-muted">
            {refiner === undefined
              ? t("onboarding.engine.refineNone")
              : t("onboarding.engine.refineHelp", { provider: t(`engines.provider.${refiner}`) })}
          </div>
        </div>
        <Toggle
          checked={refiner !== undefined && draft.refine}
          disabled={refiner === undefined}
          onChange={(refine) => {
            onDraft({ ...draft, refine });
          }}
          label={t("onboarding.engine.refine")}
        />
      </div>
      <p className="text-[11px] text-fg-subtle" data-locale={locale}>
        {t("onboarding.engine.note")}
      </p>
    </div>
  );
}
