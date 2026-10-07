// AI 模型与预设 (apps/mobile's AiModels): whether a take the phone recognises is cleaned up, the
// presets, the LLM providers, the model in use and who gets the text, and the fallback models.
import { providersFor } from "@voltip/shared";
import { View } from "react-native";
import { Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { CurrentService, ProviderCard, ServicePrivacy, useOpenCards } from "../features/engines";
import { FallbackSection } from "../features/Fallback";
import { PresetsSection } from "../features/Presets";
import { Lede, Page, Section, SectionTitle, SwitchRow, useAppTheme } from "../ui/kit";

export function AiModels() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const engines = state.engines;
  const providers = providersFor(engines, "llm");
  const cards = useOpenCards(engines.llm_provider);
  return (
    <Page testID="phone-ai" gap={20}>
      <Lede>{t("mobile.settings.own")}</Lede>
      <Section>
        <SwitchRow
          icon="creation-outline"
          title={t("engines.refineToggle")}
          description={t("engines.refineToggleHelp")}
          value={settings.refine_enabled}
          onValueChange={(refine_enabled) => {
            void backend.invoke("settings_set_engines", {
              engines: { ...settings, refine_enabled },
            });
          }}
          testID="refine-toggle"
        />
        {engines.refine_enabled && !engines.refine_ready && (
          <View style={{ paddingHorizontal: 16, paddingBottom: 12 }} testID="refine-not-ready">
            <Text variant="bodySmall" style={{ color: theme.voltip.warning }}>
              {t("engines.refineNotReady", {
                issue: t(`engines.issue.${engines.refine_issue ?? "no_provider"}`),
              })}
            </Text>
          </View>
        )}
      </Section>
      <PresetsSection />
      <View style={{ gap: 8 }}>
        <SectionTitle style={{ paddingHorizontal: 4 }}>
          {t("engines.llmSection.title")}
        </SectionTitle>
        <View style={{ gap: 12 }} accessibilityLabel={t("engines.llmSection.title")}>
          {providers.map((p) => (
            <ProviderCard
              key={p.id}
              provider={p}
              kind="llm"
              open={cards.isOpen(p.id)}
              onToggle={(open) => {
                cards.toggle(p.id, open);
              }}
            />
          ))}
        </View>
        <View style={{ gap: 4, paddingHorizontal: 4 }}>
          <CurrentService kind="llm" />
          <ServicePrivacy kind="llm" />
        </View>
      </View>
      <FallbackSection kind="llm" />
    </Page>
  );
}
