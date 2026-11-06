import {
  CardGrid,
  LampText,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { livePreviewState, livePreviewStateText, streamingModel } from "./helpers";
import { ModelCard } from "./LocalModels";

/** The 实时预览 block of the engines group (docs/dictation.md §11.10): two switches in one block,
 *  each written through `settings_set_engines` (the whole block, like every other engine setting)
 *  — 服务商的实时文字 (`provider_preview`: the built-in service's preview, §11.8, and a realtime
 *  model's words, §11.9) and 本地模型预览 (`local_preview`: the downloaded streaming model). The
 *  status line names whose words the pill shows (`state.engines.preview_source` and
 *  `preview_provider`), or why it shows none: a realtime model recognising unseen, a fallback model
 *  taking whole recordings standing in for the realtime one (§3.5), the model not downloaded, or
 *  off. The streaming model's card underneath offers download / cancel / retry / delete only: it
 *  is not a recognition model, so there is no 使用此模型 and it is never `active`. */
export function LivePreview() {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const model = streamingModel(state.models);
  const status = livePreviewState(settings, state.engines);
  const set = (change: { provider_preview: boolean } | { local_preview: boolean }) => {
    void backend.invoke("settings_set_engines", { engines: { ...settings, ...change } });
  };
  return (
    <SettingsSection
      title={t("engines.livePreview.title")}
      description={t("engines.livePreview.note")}
      data-testid="live-preview"
      data={{
        "data-state": status.kind,
        "data-source": status.kind === "showing" ? status.source : undefined,
      }}
      aside={
        <LampText
          tone={
            status.kind === "showing"
              ? "ok"
              : status.kind === "off" || status.kind === "hidden"
                ? "idle"
                : "warn"
          }
          size="sm">
          <span data-testid="live-preview-state">{livePreviewStateText(status, t, locale)}</span>
        </LampText>
      }>
      <SettingsRows>
        <StatusRow
          label={t("engines.livePreview.provider")}
          help={t("engines.livePreview.providerHelp")}
          data-testid="live-preview-provider">
          <Toggle
            checked={settings.provider_preview}
            ariaLabel={t("engines.livePreview.provider")}
            onChange={(provider_preview) => {
              set({ provider_preview });
            }}
          />
        </StatusRow>
        <StatusRow
          label={t("engines.livePreview.local")}
          help={t("engines.livePreview.localHelp")}
          data-testid="live-preview-local">
          <Toggle
            checked={settings.local_preview}
            ariaLabel={t("engines.livePreview.local")}
            onChange={(local_preview) => {
              set({ local_preview });
            }}
          />
        </StatusRow>
      </SettingsRows>
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
