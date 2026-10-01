import {
  type AudioDevice,
  type AudioOutputs,
  RECORDING_SOURCES,
  type RecordingSettings,
  type RecordingSource,
  recordingSourceLabel,
  systemAudioNote,
} from "@voltip/shared";
import { Segmented, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useEffect, useState } from "react";

/** What a take records (docs/dictation.md §22): the settings, whether the computer's sound can be
 *  recorded here (and why not), the output device a take would record from, and setters that
 *  keep the rest of `settings.recording`. */
export interface RecordingSourceState {
  recording: RecordingSettings;
  /** `audio_outputs`; `undefined` until it answered. */
  outputs: AudioOutputs | undefined;
  /** Why the computer's sound cannot be recorded here; `undefined` when it can (or is not known
   *  yet). */
  unavailable: string | undefined;
  /** The output device a take records from: the chosen one, else the system default. */
  output: AudioDevice | undefined;
  /** The system's default output. */
  defaultOutput: AudioDevice | undefined;
  /** The chosen output device is not connected (a take records the default instead). */
  outputMissing: boolean;
  setSource: (source: RecordingSource) => void;
  setOutput: (device: string | null) => void;
  /** `mixed`: remove the microphone's echo of the computer's sound (docs/dictation.md §22.6). */
  setEchoCancel: (on: boolean) => void;
}

export function useRecordingSource(): RecordingSourceState {
  const { backend } = useBackend();
  const { locale } = useI18n();
  const { recording } = useUiState().settings;
  const [outputs, setOutputs] = useState<AudioOutputs | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    backend
      .audioOutputs()
      .then((answer) => {
        if (alive) setOutputs(answer);
      })
      .catch(() => {
        // No answer: the computer's sound is offered as not supported.
        if (alive) setOutputs({ system_audio: { state: "unsupported" }, devices: [] });
      });
    return () => {
      alive = false;
    };
  }, [backend]);
  const set = (next: Partial<RecordingSettings>) => {
    void backend.invoke("settings_set_recording", { recording: { ...recording, ...next } });
  };
  const devices = outputs?.devices ?? [];
  const defaultOutput = devices.find((d) => d.is_default) ?? devices[0];
  const chosen = recording.output_device ?? undefined;
  const found = chosen === undefined ? undefined : devices.find((d) => d.id === chosen);
  return {
    recording,
    outputs,
    unavailable: outputs === undefined ? undefined : systemAudioNote(outputs.system_audio, locale),
    output: found ?? defaultOutput,
    defaultOutput,
    outputMissing: chosen !== undefined && outputs !== undefined && found === undefined,
    setSource: (source) => {
      set({ source });
    },
    setOutput: (device) => {
      set({ output_device: device });
    },
    setEchoCancel: (on) => {
      set({ echo_cancel: on });
    },
  };
}

/** 麦克风 / 电脑声音 / 混合. The two that record the computer's sound are off, with the reason as
 *  their title, until `audio_outputs` says it works here; the setting stays as saved. */
export function SourceSwitch({ state, testId }: { state: RecordingSourceState; testId: string }) {
  const { t, locale } = useI18n();
  const ready = state.outputs !== undefined && state.unavailable === undefined;
  return (
    <span className="inline-flex" data-testid={testId}>
      <Segmented
        size="sm"
        label={t("settings.microphone.source")}
        value={state.recording.source}
        onChange={state.setSource}
        options={RECORDING_SOURCES.map((source) => ({
          value: source,
          label: recordingSourceLabel(source, locale),
          ...(source === "microphone" || ready
            ? {}
            : {
                disabled: true,
                ...(state.unavailable === undefined ? {} : { reason: state.unavailable }),
              }),
        }))}
      />
    </span>
  );
}
