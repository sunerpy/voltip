import { MAX_MINUTES_CHOICES, type TFunction } from "@voltip/shared";
import {
  Segmented,
  Select,
  SettingsPane,
  SettingsRows,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";

/** `60` → 「1 小时」, `30` → 「30 分钟」. */
function lengthLabel(minutes: number, t: TFunction): string {
  return minutes >= 60 && minutes % 60 === 0
    ? t("settings.dictation.hours", { n: minutes / 60 })
    : t("settings.dictation.minutes", { n: minutes });
}

/** 设置 › 听写 (user feedback 2026-09-29): how a take's text reaches the app in front — pasted at
 *  the cursor (the default) or only copied — written through `settings_set_engines { inject }` with
 *  the rest of the block unchanged. It used to sit among the 语音模型 page's recognition options;
 *  this is now the only place that sets it. Also the longest recording (docs/dictation.md §22,
 *  `settings_set_recording` with the rest of the recording settings unchanged). */
export function Dictation() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { engines, recording } = useUiState().settings;
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
        <StatusRow label={t("settings.dictation.maxLabel")} help={t("settings.dictation.maxHelp")}>
          <Select
            aria-label={t("settings.dictation.maxLabel")}
            size="sm"
            value={String(recording.max_minutes)}
            options={MAX_MINUTES_CHOICES.map((minutes) => ({
              value: String(minutes),
              label: lengthLabel(minutes, t),
            }))}
            data-testid="dictation-max-minutes"
            onChange={(value) => {
              void backend.invoke("settings_set_recording", {
                recording: { ...recording, max_minutes: Number(value) },
              });
            }}
          />
        </StatusRow>
      </SettingsRows>
    </SettingsPane>
  );
}
