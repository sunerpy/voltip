import { MAX_MINUTES_CHOICES } from "@voltip/shared";
import { Card, Select, StatusRow, useBackend, useI18n, useUiState } from "@voltip/ui";
import { lengthLabel } from "./Settings";

/** 录音 on the phone (user decision 2026-10-01): how long one take may run (docs/dictation.md §22,
 *  `settings_set_recording` with the rest of the recording settings unchanged). The phone records
 *  its microphone only. */
export function Recording() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { recording } = useUiState().settings;
  return (
    <div className="flex flex-col gap-6 p-4" data-testid="phone-recording">
      <Card padding="none" className="px-4">
        <StatusRow label={t("settings.dictation.maxLabel")} help={t("settings.dictation.maxHelp")}>
          <Select
            aria-label={t("settings.dictation.maxLabel")}
            value={String(recording.max_minutes)}
            options={MAX_MINUTES_CHOICES.map((minutes) => ({
              value: String(minutes),
              label: lengthLabel(minutes, t),
            }))}
            data-testid="recording-max-minutes"
            onChange={(value) => {
              void backend.invoke("settings_set_recording", {
                recording: { ...recording, max_minutes: Number(value) },
              });
            }}
          />
        </StatusRow>
      </Card>
    </div>
  );
}
