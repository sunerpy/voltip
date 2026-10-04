import {
  CardGrid,
  LampText,
  SettingsSection,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { livePreviewState, streamingModel } from "./helpers";
import { ModelCard } from "./LocalModels";

/** The 实时预览 block of the engines group (docs/dictation.md §11): the switch writes
 *  `EngineSettings.live_preview` through `settings_set_engines` (the whole block, like every other
 *  engine setting); the status line follows `state.engines.live_preview_ready` and `live_source` —
 *  ready through the built-in service (§11.8), the realtime model in use (§11.9) or the downloaded
 *  model, unavailable while a fallback model taking whole recordings stands in for the realtime
 *  one (§3.5), model not downloaded, or off. The streaming model's card underneath offers download /
 *  cancel / retry / delete only: it is not a recognition model, so there is no 使用此模型 and it is
 *  never `active`. */
export function LivePreview() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const model = streamingModel(state.models);
  const status = livePreviewState(settings, state.engines);
  const setLivePreview = (live_preview: boolean) => {
    void backend.invoke("settings_set_engines", { engines: { ...settings, live_preview } });
  };
  return (
    <SettingsSection
      title={t("engines.livePreview.title")}
      description={t("engines.livePreview.note")}
      data-testid="live-preview"
      data={{ "data-state": status }}
      aside={
        <LampText
          tone={
            status === "ready" || status === "cloud" || status === "stream"
              ? "ok"
              : status === "off"
                ? "idle"
                : "warn"
          }
          size="sm">
          <span data-testid="live-preview-state">{t(`engines.livePreview.state.${status}`)}</span>
        </LampText>
      }>
      <Toggle
        checked={settings.live_preview}
        onChange={setLivePreview}
        label={t("engines.livePreview.toggle")}
      />
      {model && (
        <CardGrid role="list" aria-label={t("engines.livePreview.title")}>
          <div role="listitem" data-tier={model.tier} className="flex min-w-0">
            <ModelCard model={model} />
          </div>
        </CardGrid>
      )}
    </SettingsSection>
  );
}
