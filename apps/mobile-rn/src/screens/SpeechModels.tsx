// 语音模型 (apps/mobile's SpeechModels): the recognition providers as cards, without the on-device
// one (a phone has no local models), the model in use and who gets the audio, the fallback models,
// the recognition language and the Chinese script.
import { languageOptions, providersFor } from "@voltip/shared";
import { View } from "react-native";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import {
  ChineseScript,
  CurrentService,
  ProviderCard,
  ServicePrivacy,
  useOpenCards,
} from "../features/engines";
import { FallbackSection } from "../features/Fallback";
import { Hint, Lede, Page, Section, SectionTitle } from "../ui/kit";
import { SelectRow } from "../ui/Select";

export function SpeechModels() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const providers = providersFor(state.engines, "asr").filter((p) => !p.on_device);
  const cards = useOpenCards(state.engines.asr_provider);
  return (
    <Page testID="phone-speech" gap={20}>
      <Lede>{t("mobile.settings.own")}</Lede>
      <View style={{ gap: 8 }}>
        <View style={{ gap: 4, paddingHorizontal: 4 }}>
          <SectionTitle>{t("engines.asrSection.title")}</SectionTitle>
          <Hint>{t("engines.asrSection.note")}</Hint>
        </View>
        <View style={{ gap: 12 }} accessibilityLabel={t("engines.asrSection.title")}>
          {providers.map((p) => (
            <ProviderCard
              key={p.id}
              provider={p}
              kind="asr"
              open={cards.isOpen(p.id)}
              onToggle={(open) => {
                cards.toggle(p.id, open);
              }}
            />
          ))}
        </View>
        <View style={{ gap: 4, paddingHorizontal: 4 }}>
          <CurrentService kind="asr" />
          <ServicePrivacy kind="asr" />
        </View>
      </View>
      <FallbackSection kind="asr" />
      <Section footer={t("engines.languageHelp")}>
        <SelectRow
          icon="translate"
          label={t("engines.languageLabel")}
          value={settings.language ?? ""}
          options={languageOptions(t)}
          onChange={(language) => {
            const { language: _old, ...rest } = settings;
            void backend.invoke("settings_set_engines", {
              engines: language.length > 0 ? { ...rest, language } : rest,
            });
          }}
        />
      </Section>
      <Section padded>
        <ChineseScript />
      </Section>
    </Page>
  );
}
