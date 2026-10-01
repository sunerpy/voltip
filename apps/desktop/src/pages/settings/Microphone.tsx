import {
  Button,
  Select,
  SettingsPane,
  SettingsRows,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { MicrophoneStrength } from "../../features/audio/MicrophoneStrength";
import { SourceSwitch, useRecordingSource } from "../../features/audio/RecordingSource";
import { useAudioMeter } from "../../features/audio/useAudioMeter";
import { MIC_TEST_MS, useMicrophoneTest } from "../../features/audio/useMicrophoneTest";

/** The value the device menus use for "follow the system default" (`null` in the settings). */
const DEFAULT_CHOICE = "";

/** 设置 › 录音来源 (docs/dictation.md §22; was 设置 › 麦克风, user feedback 2026-09-28): what a take
 *  records — the microphone, the computer's sound or both (`settings_set_recording`) — the output
 *  device the computer's sound comes from, for a mixed take whether the speakers' echo is removed
 *  (§22.6), the input device (`settings_set_microphone`, `null` = the system default) and a
 *  测试麦克风 run with the strength bar. Nothing meters the microphone
 *  outside a test or a take. A chosen device that is unplugged stays chosen and is listed as not
 *  connected; takes use the default until it is back. */
export function Microphone() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { settings } = useUiState();
  const source = useRecordingSource();
  const usesMicrophone = source.recording.source !== "system";
  const usesOutput = source.recording.source !== "microphone";
  const chosen = settings.microphone ?? undefined;
  const test = useMicrophoneTest();
  const meter = useAudioMeter(test.testing, chosen);
  const devices = meter.devices ?? [];
  const systemDefault = devices.find((d) => d.is_default) ?? devices[0];
  const options = [
    {
      value: DEFAULT_CHOICE,
      label: systemDefault
        ? t("settings.microphone.followDefault", { name: systemDefault.name })
        : t("settings.microphone.followDefaultNone"),
    },
    ...devices.map((d) => ({ value: d.id, label: d.name })),
    ...(chosen !== undefined && meter.missing
      ? [{ value: chosen, label: t("settings.microphone.disconnected", { name: chosen }) }]
      : []),
  ];
  const facts = meter.device
    ? [
        meter.device.sample_rate_hz ? `${meter.device.sample_rate_hz / 1000} kHz` : "",
        meter.device.channels === 1
          ? t("settings.microphone.mono")
          : meter.device.channels
            ? t("settings.microphone.channels", { n: meter.device.channels })
            : "",
      ]
        .filter((part) => part.length > 0)
        .join(" · ")
    : undefined;
  const chosenOutput = source.recording.output_device ?? undefined;
  const outputOptions = [
    {
      value: DEFAULT_CHOICE,
      label: source.defaultOutput
        ? t("settings.microphone.outputDefault", { name: source.defaultOutput.name })
        : t("settings.microphone.outputDefaultNone"),
    },
    ...(source.outputs?.devices ?? []).map((d) => ({ value: d.id, label: d.name })),
    ...(chosenOutput !== undefined && source.outputMissing
      ? [
          {
            value: chosenOutput,
            label: t("settings.microphone.outputDisconnected", { name: chosenOutput }),
          },
        ]
      : []),
  ];
  const mixed = source.recording.source === "mixed";
  const sourceNote =
    source.unavailable ??
    (mixed
      ? t(
          source.recording.echo_cancel
            ? "settings.microphone.mixedEchoHint"
            : "settings.microphone.mixedHint",
        )
      : undefined);
  return (
    <SettingsPane
      title={t("settings.microphone.title")}
      lede={t("settings.microphone.lede")}
      data-testid="microphone-pane">
      <SettingsRows>
        <StatusRow
          label={t("settings.microphone.source")}
          help={t("settings.microphone.sourceHelp")}
          note={sourceNote}>
          <SourceSwitch state={source} testId="recording-source" />
        </StatusRow>
        {usesOutput && (
          <StatusRow
            label={t("settings.microphone.output")}
            help={t("settings.microphone.outputHelp")}
            note={source.outputMissing ? t("settings.microphone.outputMissingNote") : undefined}>
            <Select
              aria-label={t("settings.microphone.output")}
              size="sm"
              value={chosenOutput ?? DEFAULT_CHOICE}
              disabled={source.outputs === undefined || source.unavailable !== undefined}
              options={outputOptions}
              data-testid="recording-output"
              onChange={(value) => {
                source.setOutput(value === DEFAULT_CHOICE ? null : value);
              }}
            />
          </StatusRow>
        )}
        {mixed && (
          <StatusRow
            label={t("settings.microphone.echoCancel")}
            help={t("settings.microphone.echoCancelHelp")}>
            <Toggle
              checked={source.recording.echo_cancel}
              onChange={source.setEchoCancel}
              label={t("settings.microphone.echoCancel")}
            />
          </StatusRow>
        )}
        {usesMicrophone && (
          <StatusRow
            label={t("settings.microphone.device")}
            help={t("settings.microphone.deviceHelp")}
            note={
              meter.missing ? t("settings.microphone.missingNote") : (meter.error ?? undefined)
            }>
            <div className="flex flex-col items-end gap-1">
              <Select
                aria-label={t("settings.microphone.device")}
                size="sm"
                value={chosen ?? DEFAULT_CHOICE}
                disabled={meter.devices === undefined}
                options={options}
                data-testid="microphone-device"
                onChange={(value) => {
                  void backend.invoke("settings_set_microphone", {
                    device: value === DEFAULT_CHOICE ? null : value,
                  });
                }}
              />
              {facts !== undefined && facts.length > 0 && (
                <span className="mono text-[11px] text-fg-muted" data-testid="microphone-facts">
                  {facts}
                </span>
              )}
            </div>
          </StatusRow>
        )}
        {usesMicrophone && (
          <StatusRow
            label={t("settings.microphone.test")}
            help={t("settings.microphone.testHelp", { n: Math.round(MIC_TEST_MS / 1000) })}>
            <div className="flex items-center gap-4">
              <MicrophoneStrength
                frame={meter.frame}
                disabled={meter.error !== undefined}
                data-testid="microphone-strength"
              />
              <Button
                size="sm"
                variant={test.testing ? "outline" : "primary"}
                icon={test.testing ? "stop" : "mic"}
                disabled={meter.error !== undefined}
                data-testid="microphone-test"
                onClick={test.testing ? test.stop : test.start}>
                {test.testing
                  ? `${t("home.mic.stopTest")} · ${test.remaining}`
                  : t("settings.microphone.test")}
              </Button>
            </div>
          </StatusRow>
        )}
      </SettingsRows>
    </SettingsPane>
  );
}
