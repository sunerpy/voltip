import { type ProviderId, languageOptions, providersFor } from "@voltip/shared";
import {
  Card,
  ChineseScript,
  FallbackSection,
  ProviderCard,
  Select,
  SettingsSection,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { Lede, TOUCH_TOGGLE } from "../app/phone-ui";

/** Which provider cards are open: the one in use unless the user closed it, any other once the
 *  user opened it (as the desktop's engines pane). */
export function useOpenCards(active: ProviderId | undefined) {
  const [toggled, setToggled] = useState<ReadonlyMap<ProviderId, boolean>>(() => new Map());
  return {
    isOpen: (id: ProviderId) => toggled.get(id) ?? id === active,
    toggle: (id: ProviderId, open: boolean) => {
      setToggled((prev) => new Map(prev).set(id, open));
    },
  };
}

/** 语音模型 on the phone (user decision 2026-10-01): the recognition providers as the desktop's
 *  cards (`@voltip/ui`), without the on-device one — the phone has no local models — then the
 *  recognition language and the Chinese script. */
export function SpeechModels() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const providers = providersFor(state.engines, "asr").filter((p) => !p.on_device);
  const cards = useOpenCards(state.engines.asr_provider);
  return (
    <div className="flex flex-col gap-6 p-4" data-testid="phone-speech">
      <Lede>{t("mobile.settings.own")}</Lede>
      <SettingsSection
        title={t("engines.asrSection.title")}
        description={t("engines.asrSection.note")}>
        <div className="flex flex-col gap-3" role="list" aria-label={t("engines.asrSection.title")}>
          {providers.map((p) => (
            <div role="listitem" key={p.id}>
              <ProviderCard
                provider={p}
                kind="asr"
                open={cards.isOpen(p.id)}
                onToggle={(open) => {
                  cards.toggle(p.id, open);
                }}
              />
            </div>
          ))}
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
