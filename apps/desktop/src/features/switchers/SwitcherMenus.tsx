import { type AudioDevice, isBuiltinPreset, presetLabel } from "@voltip/shared";
import {
  Icon,
  Lamp,
  type LampTone,
  Menu,
  type MenuSection,
  cx,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { AI_ROUTE, SPEECH_ROUTE, useRouter } from "../../app/router";
import { engineReadout, shortModel } from "../../shell/page-meta";
import {
  microphoneReadoutValue,
  shortMicrophoneName,
  useMicrophoneReadout,
} from "../audio/mic-store";
import { useRecordingSource } from "../audio/RecordingSource";
import { MANAGE_PRESETS, presetMenuSections } from "../presets/presets";
import {
  engineSettingsFor,
  microphoneMenuSections,
  parseChoice,
  polishMenuSections,
  speechMenuSections,
} from "./switchers";

/** The sections of a list with a heading (its choices), without its closing command. */
function choices(sections: readonly MenuSection[]): MenuSection[] {
  return sections.filter((s) => s.label !== undefined);
}

export interface SwitcherMenuProps {
  /** What the button shows; each menu has a title-bar default (the compact mono readout). */
  trigger?: ReactNode;
  triggerClassName?: string;
  align?: "start" | "end";
  /** Spec sheet only: start open without taking the focus. */
  defaultOpen?: boolean;
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
}: {
  value: string;
  lamp?: LampTone;
  badge?: string;
}) {
  return (
    <>
      {lamp !== undefined && <Lamp tone={lamp} size={6} />}
      <span className="truncate">{value}</span>
      {badge !== undefined && (
        <span className="shrink-0 rounded-6 bg-inset px-1 text-[10px] leading-4 text-fg-muted">
          {badge}
        </span>
      )}
      <Icon name="chevronDown" size={10} className="shrink-0 opacity-70" />
    </>
  );
}

/** The 语音模型 menu (plan 2026-09-30): the cloud providers' models and the installed local
 *  models, picked in place; 管理语音模型… opens the page. */
export function SpeechModelMenu({
  trigger,
  triggerClassName,
  align,
  defaultOpen,
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
          <ReadoutTrigger value={readout.value} lamp={readout.lamp} badge={readout.badge} />
        )
      }
      label={t("switchers.speech.label")}
      triggerLabel={t("switchers.speech.trigger", { name: readout.value })}
      title={readout.title}
      sections={speechMenuSections(state.engines, state.models, t, locale)}
      align={align}
      defaultOpen={defaultOpen}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate(SPEECH_ROUTE);
          return;
        }
        const engines = choice && engineSettingsFor(state.settings.engines, "asr", choice);
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
  defaultOpen,
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
      title={state.engines.refine_model || undefined}
      sections={polishMenuSections(state.engines, t)}
      align={align}
      defaultOpen={defaultOpen}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate(AI_ROUTE);
          return;
        }
        const engines = choice && engineSettingsFor(state.settings.engines, "llm", choice);
        if (engines !== undefined) void backend.invoke("settings_set_engines", { engines });
      }}
    />
  );
}

/** Option B of the design: one menu for the preset and the model the clean-up runs with. */
export function PolishMenu({
  trigger,
  triggerClassName,
  align,
  defaultOpen,
  "data-testid": testId,
}: SwitcherMenuProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const { navigate } = useRouter();
  const engines = state.settings.engines;
  const preset = presetLabel(engines.refine_preset, state.presets, locale);
  const model = usePolishModelName();
  const presets = presetMenuSections(engines.refine_preset, state.presets, t);
  // The two lists, then their two commands together at the end.
  const sections: MenuSection[] = [
    ...choices(presets),
    ...choices(polishMenuSections(state.engines, t)),
    {
      items: [
        { kind: "action", id: MANAGE_PRESETS, label: t("presets.menu.manage") },
        { kind: "action", id: "manage:ai", label: t("switchers.polish.manage") },
      ],
    },
  ];
  return (
    <Menu
      trigger={
        trigger ?? (
          <>
            <span {...(isBuiltinPreset(engines.refine_preset) ? {} : { "data-user-text": "" })}>
              {preset}
            </span>
            <span aria-hidden>·</span>
            <span className="mono truncate text-[11px]">{model}</span>
            <Icon name="chevronDown" size={10} className="shrink-0 opacity-70" />
          </>
        )
      }
      label={t("switchers.polish.combinedLabel")}
      triggerLabel={t("switchers.polish.combinedTrigger", { preset, model })}
      sections={sections}
      align={align}
      defaultOpen={defaultOpen}
      triggerClassName={triggerClassName ?? BAR_TRIGGER}
      data-testid={testId}
      onSelect={(id) => {
        if (id === MANAGE_PRESETS) {
          navigate({ name: "ai", section: "presets" });
          return;
        }
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate(AI_ROUTE);
          return;
        }
        const next =
          choice === undefined
            ? id === engines.refine_preset
              ? undefined
              : { ...engines, refine_preset: id }
            : engineSettingsFor(engines, "llm", choice);
        if (next !== undefined) void backend.invoke("settings_set_engines", { engines: next });
      }}
    />
  );
}

export interface MicrophoneMenuProps extends SwitcherMenuProps {
  /** Also offer what a take records (the title bar; the home card has its own switch). */
  withSource?: boolean;
}

/** The 麦克风 menu: the input devices, read again each time it opens (a USB microphone may have
 *  just been plugged in), the system default first; 录音来源设置… opens the settings group. */
export function MicrophoneMenu({
  trigger,
  triggerClassName,
  align,
  defaultOpen,
  withSource = false,
  "data-testid": testId,
}: MicrophoneMenuProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const { navigate } = useRouter();
  const source = useRecordingSource();
  const metered = useMicrophoneReadout();
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
  const device =
    chosen === null ? devices.find((d) => d.is_default) : devices.find((d) => d.id === chosen);
  const name =
    device !== undefined ? shortMicrophoneName(device.name) : microphoneReadoutValue(metered, t);
  return (
    <Menu
      trigger={trigger ?? <ReadoutTrigger value={name} />}
      label={t("switchers.microphone.label")}
      triggerLabel={t("switchers.microphone.trigger", { name })}
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
      defaultOpen={defaultOpen}
      onOpen={load}
      triggerClassName={cx(triggerClassName ?? BAR_TRIGGER)}
      data-testid={testId}
      onSelect={(id) => {
        const choice = parseChoice(id);
        if (choice?.kind === "manage") {
          navigate({ name: "settings", section: "microphone" });
          return;
        }
        if (choice?.kind === "source") source.setSource(choice.source);
        if (choice?.kind === "microphone" && choice.device !== chosen)
          void backend.invoke("settings_set_microphone", { device: choice.device });
      }}
    />
  );
}
