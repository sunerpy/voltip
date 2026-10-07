// 录音 (apps/mobile's Recording): how long one take may run (docs/dictation.md §22,
// `settings_set_recording` with the rest of the recording settings unchanged). The phone records
// its microphone only.
import { MAX_MINUTES_CHOICES } from "@voltip/shared";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { Page, Section } from "../ui/kit";
import { SelectRow } from "../ui/Select";
import { lengthLabel } from "./Settings";

export function Recording() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { recording } = useUiState().settings;
  return (
    <Page testID="phone-recording">
      <Section footer={t("settings.dictation.maxHelp")}>
        <SelectRow
          icon="timer-outline"
          label={t("settings.dictation.maxLabel")}
          value={String(recording.max_minutes)}
          options={MAX_MINUTES_CHOICES.map((minutes) => ({
            value: String(minutes),
            label: lengthLabel(minutes, t),
          }))}
          testID="recording-max-minutes"
          onChange={(value) => {
            void backend.invoke("settings_set_recording", {
              recording: { ...recording, max_minutes: Number(value) },
            });
          }}
        />
      </Section>
    </Page>
  );
}
