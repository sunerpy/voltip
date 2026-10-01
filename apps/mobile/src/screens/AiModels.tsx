import { providersFor } from "@voltip/shared";
import {
  Card,
  PresetsSection,
  ProviderCard,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useOpenCards } from "./SpeechModels";

/** AI 模型与预设 on the phone (user decision 2026-10-01): whether a take the phone recognises is
 *  cleaned up, the presets (the desktop's section, `@voltip/ui`) and the LLM providers. */
export function AiModels() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const engines = state.engines;
  const providers = providersFor(engines, "llm");
  const cards = useOpenCards(engines.llm_provider);
  return (
    <div className="flex flex-col gap-4 p-4" data-testid="phone-ai">
      <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.settings.own")}</p>
      <Card className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <span className="text-[14px] font-medium text-fg">{t("engines.refineToggle")}</span>
          <Toggle
            checked={settings.refine_enabled}
            onChange={(refine_enabled) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...settings, refine_enabled },
              });
            }}
            label={settings.refine_enabled ? t("engines.refineOn") : t("engines.refineOff")}
          />
        </div>
        <p className="text-[12px] leading-5 text-fg-muted">{t("engines.refineToggleHelp")}</p>
        {engines.refine_enabled && !engines.refine_ready && (
          <p className="text-[12px] text-warning" data-testid="refine-not-ready">
            {t("engines.refineNotReady", {
              issue: t(`engines.issue.${engines.refine_issue ?? "no_provider"}`),
            })}
          </p>
        )}
      </Card>
      <PresetsSection />
      <div className="flex flex-col gap-3" role="list" aria-label={t("engines.llmSection.title")}>
        {providers.map((p) => (
          <div role="listitem" key={p.id}>
            <ProviderCard
              provider={p}
              kind="llm"
              open={cards.isOpen(p.id)}
              onToggle={(open) => {
                cards.toggle(p.id, open);
              }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
