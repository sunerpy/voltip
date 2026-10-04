import { providersFor } from "@voltip/shared";
import {
  Card,
  FallbackSection,
  PresetsSection,
  ProviderCard,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { Lede, TOUCH_TOGGLE } from "../app/phone-ui";
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
    <div className="flex flex-col gap-6 p-4" data-testid="phone-ai">
      <Lede>{t("mobile.settings.own")}</Lede>
      {/* The desktop writes the state beside the switch; a phone row has no room for it, and the
          switch says it. */}
      <Card padding="none" className="px-4">
        <StatusRow label={t("engines.refineToggle")} help={t("engines.refineToggleHelp")}>
          <Toggle
            checked={settings.refine_enabled}
            ariaLabel={t("engines.refineToggle")}
            className={TOUCH_TOGGLE}
            onChange={(refine_enabled) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...settings, refine_enabled },
              });
            }}
          />
        </StatusRow>
        {engines.refine_enabled && !engines.refine_ready && (
          <p className="pb-3 text-[12px] leading-5 text-warning" data-testid="refine-not-ready">
            {t("engines.refineNotReady", {
              issue: t(`engines.issue.${engines.refine_issue ?? "no_provider"}`),
            })}
          </p>
        )}
      </Card>
      <PresetsSection />
      {/* No description: the desktop's names 语音编辑, which the phone does not have. */}
      <SettingsSection title={t("engines.llmSection.title")}>
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
      </SettingsSection>
      <FallbackSection kind="llm" toggleClassName={TOUCH_TOGGLE} />
    </div>
  );
}
