import {
  Segmented,
  SettingsPane,
  SettingsRows,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";

/** 设置 › 听写 (user feedback 2026-09-29): how a take's text reaches the app in front — pasted at
 *  the cursor (the default) or only copied — written through `settings_set_engines { inject }` with
 *  the rest of the block unchanged. It used to sit among the 语音模型 page's recognition options;
 *  this is now the only place that sets it. */
export function Dictation() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const engines = useUiState().settings.engines;
  return (
    <SettingsPane
      title={t("settings.dictation.title")}
      lede={t("settings.dictation.lede")}
      data-testid="dictation-pane">
      <SettingsRows>
        <StatusRow label={t("engines.injectLabel")} help={t("engines.injectHelp")}>
          <Segmented
            size="sm"
            label={t("engines.injectLabel")}
            value={engines.inject}
            onChange={(inject) => {
              void backend.invoke("settings_set_engines", { engines: { ...engines, inject } });
            }}
            options={[
              { value: "paste", label: t("engines.inject.paste") },
              { value: "clipboard_only", label: t("engines.inject.clipboard") },
            ]}
          />
        </StatusRow>
      </SettingsRows>
    </SettingsPane>
  );
}
