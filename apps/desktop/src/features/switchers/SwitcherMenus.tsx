import type { AudioDevice } from "@voltip/shared";
import { Icon, Lamp, type LampTone, Menu, useBackend, useI18n, useUiState } from "@voltip/ui";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { AI_ROUTE, SPEECH_ROUTE, useRouter } from "../../app/router";
import { engineReadout, shortModel } from "../../shell/page-meta";
import {
  microphoneReadoutValue,
  shortMicrophoneName,
  useMicrophoneReadout,
} from "../audio/mic-store";
import { useRecordingSource } from "../audio/RecordingSource";
import {
  engineSettingsFor,
  microphoneMenuSections,
  parseChoice,
  polishMenuSections,
  speechMenuSections,
} from "./switchers";

export interface SwitcherMenuProps {
  /** What the button shows; each menu has a title-bar default (the compact mono readout). */
  trigger?: ReactNode;
  triggerClassName?: string;
  align?: "start" | "end";
  title?: string;
  "data-testid"?: string;
}

/** The title bar's readout look, as a button: mono 11 px, subtle until hovered. */
export const BAR_TRIGGER =
  "inline-flex h-6 min-w-0 max-w-[220px] items-center gap-1.5 rounded-6 px-1.5 text-fg-subtle transition-colors hover:bg-inset hover:text-fg";

/** A readout's value with its lamp, tag and the menu chevron. */
export function ReadoutTrigger({
  value,
  lamp,
  badge,
  badgeTestId,
}: {
  value: string;
  lamp?: LampTone;
  badge?: string;
  badgeTestId?: string;
}) {
  return (
    <>
      {lamp !== undefined && <Lamp tone={lamp} size={6} />}
      <span className="truncate">{value}</span>
      {badge !== undefined && (
        <span
          data-testid={badgeTestId}
          className="shrink-0 rounded-6 bg-inset px-1 text-[10px] leading-4 text-fg-muted">
          {badge}
        </span>
      )}
      <Icon name="chevronDown" size={10} className="shrink-0 opacity-70" />
    </>
  );
}

/** The 语音模型 menu (user request 2026-09-30): the cloud providers' models and the installed
 *  local models, picked in place; 管理语音模型… opens the page. */
export function SpeechModelMenu({
  trigger,
  triggerClassName,
  align,
  title,
  "data-testid": testId,
}: SwitcherMenuProps) {
  const { backend } = useBackend();
  const i18n = useI18n();
  const { t, locale } = i18n;
  const state = useUiState();
  const { navigate } = useRouter();
  const readout = engineReadout(state.engines, i18n);
  return (
    <Menu
      trigger={
        trigger ?? (
          <ReadoutTrigger
            value={readout.value}
            lamp={readout.lamp}
            badge={readout.badge}
            badgeTestId="title-bar-readout-badge"
          />
        )
      }
      label={t("switchers.speech.label")}
      triggerLabel={t("switchers.speech.trigger", { name: readout.value })}
      title={title ?? readout.title}
      sections={speechMenuSections(state.engines, state.models, t, locale)}
      align={align}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate(SPEECH_ROUTE);
          return;
        }
        const engines =
          choice && engineSettingsFor(state.settings.engines, "asr", choice, state.engines);
        if (engines !== undefined) void backend.invoke("settings_set_engines", { engines });
      }}
    />
  );
}

/** The model the clean-up runs on, short (`qwen3.8-27b`), or why there is none. */
export function usePolishModelName(): string {
  const { t } = useI18n();
  const engines = useUiState().engines;
  return engines.refine_model.length > 0
    ? shortModel(engines.refine_model)
    : t(`engines.issue.${engines.refine_issue ?? "no_provider"}`);
}

/** The AI 润色模型 menu: the LLM providers' models, picked in place; 管理 AI 模型… opens the page. */
export function PolishModelMenu({
  trigger,
  triggerClassName,
  align,
  title,
  "data-testid": testId,
}: SwitcherMenuProps) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const { navigate } = useRouter();
  const name = usePolishModelName();
  return (
    <Menu
      trigger={trigger ?? <ReadoutTrigger value={name} />}
      label={t("switchers.polish.label")}
      triggerLabel={t("switchers.polish.trigger", { name })}
      title={title ?? (state.engines.refine_model || name)}
      sections={polishMenuSections(state.engines, t)}
      align={align}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate(AI_ROUTE);
          return;
        }
        const engines =
          choice && engineSettingsFor(state.settings.engines, "llm", choice, state.engines);
        if (engines !== undefined) void backend.invoke("settings_set_engines", { engines });
      }}
    />
  );
}

export interface MicrophoneMenuProps extends SwitcherMenuProps {
  /** Also offer what a take records (the title bar; the home card has its own switch). */
  withSource?: boolean;
}

/** The input device a take records from: the chosen one, else the system default; none until the
 *  list has answered, or while the chosen one is not connected. */
function recordingDevice(
  devices: readonly AudioDevice[],
  chosen: string | null,
): AudioDevice | undefined {
  return chosen === null ? devices.find((d) => d.is_default) : devices.find((d) => d.id === chosen);
}

/** That device by its short name; the metered device (or why there is none) until the list has
 *  answered. */
export function useMicrophoneName(devices: readonly AudioDevice[]): string {
  const { t } = useI18n();
  const chosen = useUiState().settings.microphone ?? null;
  const metered = useMicrophoneReadout();
  const device = recordingDevice(devices, chosen);
  return device !== undefined
    ? shortMicrophoneName(device.name)
    : microphoneReadoutValue(metered, t);
}

/** The 麦克风 menu: the input devices, read again each time it opens (a USB microphone may have
 *  just been plugged in), the system default first; 录音来源设置… opens the settings group. */
export function MicrophoneMenu({
  trigger,
  triggerClassName,
  align,
  title,
  withSource = false,
  "data-testid": testId,
}: MicrophoneMenuProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const { navigate } = useRouter();
  const source = useRecordingSource();
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const load = useCallback(() => {
    backend
      .audioDevices()
      .then(setDevices)
      .catch(() => {
        setDevices([]);
      });
  }, [backend]);
  useEffect(load, [load]);
  const chosen = state.settings.microphone ?? null;
  const name = useMicrophoneName(devices);
  return (
    <Menu
      trigger={trigger ?? <ReadoutTrigger value={name} />}
      label={t("switchers.microphone.label")}
      triggerLabel={t("switchers.microphone.trigger", { name })}
      // The whole name on hover: the short one can still be cut where the title bar is tight.
      title={title ?? recordingDevice(devices, chosen)?.name ?? name}
      sections={microphoneMenuSections(
        devices,
        chosen,
        t,
        withSource
          ? {
              current: source.recording.source,
              available: source.outputs !== undefined && source.unavailable === undefined,
            }
          : undefined,
        locale,
      )}
      align={align}
      onOpen={load}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate({ name: "settings", section: "microphone" });
          return;
        }
        if (choice?.kind === "source" && choice.source !== source.recording.source)
          source.setSource(choice.source);
        if (choice?.kind === "microphone" && choice.device !== chosen)
          void backend.invoke("settings_set_microphone", { device: choice.device });
      }}
    />
  );
}
