import { languageOptions, providersFor } from "@voltip/shared";
import {
  Card,
  ChineseScript,
  CurrentService,
  FallbackSection,
  ProviderCards,
  Select,
  ServicePrivacy,
  SettingsSection,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { Lede, TOUCH_TOGGLE } from "../app/phone-ui";

/** 语音模型 on the phone (user decision 2026-10-01): the recognition providers as the desktop's
 *  cards (`@voltip/ui`), without the on-device one — the phone has no local models — with the
 *  desktop's 当前 and privacy lines under them, then the recognition language and the Chinese
 *  script. */
export function SpeechModels() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const providers = providersFor(state.engines, "asr").filter((p) => !p.on_device);
  return (
    <div className="flex flex-col gap-6 p-4" data-testid="phone-speech">
      <Lede>{t("mobile.settings.own")}</Lede>
      <SettingsSection
        title={t("engines.asrSection.title")}
        description={t("engines.asrSection.note")}>
        <ProviderCards kind="asr" providers={providers} label={t("engines.asrSection.title")} />
        {/* docs/dictation.md §3.5: the model in use (a fallback model says so) and who gets the audio. */}
        <div className="flex flex-col gap-1">
          <CurrentService kind="asr" />
          <ServicePrivacy kind="asr" />
        </div>
      </SettingsSection>
      {/* docs/dictation.md §3.5: the models to move on to when the selected one runs out. */}
      <FallbackSection kind="asr" toggleClassName={TOUCH_TOGGLE} />
      <Card padding="none" className="px-4">
        <StatusRow label={t("engines.languageLabel")} help={t("engines.languageHelp")}>
          <Select
            aria-label={t("engines.languageLabel")}
            value={settings.language ?? ""}
            onChange={(language) => {
              const { language: _old, ...rest } = settings;
              void backend.invoke("settings_set_engines", {
                engines: language.length > 0 ? { ...rest, language } : rest,
              });
            }}
            options={languageOptions(t)}
            data-endonyms=""
          />
        </StatusRow>
      </Card>
      {/* The desktop's 中文字形 section, in a card of its own as the rest of the page. */}
      <Card>
        <ChineseScript />
      </Card>
    </div>
  );
}
