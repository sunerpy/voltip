import { type ProviderId, languageOptions, providersFor } from "@voltip/shared";
import {
  Card,
  ChineseScript,
  ProviderCard,
  Select,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";

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
    <div className="flex flex-col gap-4 p-4" data-testid="phone-speech">
      <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.settings.own")}</p>
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
      <Card className="flex flex-col gap-2">
        <Select
          label={t("engines.languageLabel")}
          size="sm"
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
        <p className="text-[12px] leading-5 text-fg-muted">{t("engines.languageHelp")}</p>
      </Card>
      <ChineseScript />
    </div>
  );
}
