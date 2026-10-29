import { MAX_MINUTES_CHOICES, type TFunction } from "@voltip/shared";
import {
  Segmented,
  Select,
  SettingsPane,
  SettingsRows,
  StatusRow,
  Toggle,
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
 *  this is now the only place that sets it. Next to it what the clipboard holds after a paste and
 *  what a take does when the window changed (docs/dictation.md §14.3, `clipboard_after` /
 *  `switched_window`, written the same way), and whether a paste is checked and a result card
 *  stays on screen (§28, `insert_check` / `result_cards_stay`, the same way). Also the longest
 *  recording (§22, `settings_set_recording` with the rest of the recording settings unchanged). */
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
        {/* docs/dictation.md §14.3: what the clipboard holds once the text is pasted, and what a
            take does when another window came to the front while it was processed. */}
        <StatusRow
          label={t("settings.dictation.clipboardAfterLabel")}
          help={t("settings.dictation.clipboardAfterHelp")}>
          <Segmented
            size="sm"
            label={t("settings.dictation.clipboardAfterLabel")}
            value={engines.clipboard_after}
            onChange={(clipboard_after) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...engines, clipboard_after },
              });
            }}
            options={[
              { value: "restore", label: t("settings.dictation.clipboardAfter.restore") },
              { value: "keep", label: t("settings.dictation.clipboardAfter.keep") },
            ]}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.dictation.switchedLabel")}
          help={t("settings.dictation.switchedHelp")}>
          <Segmented
            size="sm"
            label={t("settings.dictation.switchedLabel")}
            value={engines.switched_window}
            onChange={(switched_window) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...engines, switched_window },
              });
            }}
            options={[
              { value: "copy_only", label: t("settings.dictation.switched.copyOnly") },
              { value: "paste_here", label: t("settings.dictation.switched.pasteHere") },
            ]}
          />
        </StatusRow>
        {/* docs/dictation.md §28: the check of the field after a paste, and the result cards. */}
        <StatusRow
          label={t("insertCheck.settings.check")}
          help={t("insertCheck.settings.checkHelp")}>
          <Toggle
            checked={engines.insert_check}
            ariaLabel={t("insertCheck.settings.check")}
            onChange={(insert_check) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...engines, insert_check },
              });
            }}
          />
        </StatusRow>
        <StatusRow label={t("insertCheck.settings.stay")} help={t("insertCheck.settings.stayHelp")}>
          <Toggle
            checked={engines.result_cards_stay}
            ariaLabel={t("insertCheck.settings.stay")}
            onChange={(result_cards_stay) => {
              void backend.invoke("settings_set_engines", {
                engines: { ...engines, result_cards_stay },
              });
            }}
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
