import { type TFunction, presetLabel, sceneLabel } from "@voltip/shared";
import { Icon, type IconName, useI18n, useUiState } from "@voltip/ui";
import type { ReactNode } from "react";
import { type Screen, useMobileShell } from "../app/shell";

/** `60` → 「1 小时」, `30` → 「30 分钟」 (as 设置 › 听写 on the desktop says it). */
export function lengthLabel(minutes: number, t: TFunction): string {
  return minutes >= 60 && minutes % 60 === 0
    ? t("settings.dictation.hours", { n: minutes / 60 })
    : t("settings.dictation.minutes", { n: minutes });
}

function Row({
  icon,
  title,
  detail,
  to,
}: {
  icon: IconName;
  title: string;
  detail?: ReactNode;
  to: Screen;
}) {
  const shell = useMobileShell();
  return (
    <li>
      <button
        type="button"
        data-testid={`settings-${to}`}
        onClick={() => {
          shell.go(to);
        }}
        className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-inset">
        <Icon name={icon} size={18} className="shrink-0 text-fg-muted" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-[14px] font-medium text-fg">{title}</span>
          {detail !== undefined && (
            <span className="truncate text-[12px] text-fg-muted">{detail}</span>
          )}
        </span>
        <Icon name="chevronRight" size={16} className="shrink-0 text-fg-subtle" />
      </button>
    </li>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2" aria-label={title}>
      <h2 className="px-1 text-[12px] font-medium text-fg-subtle">{title}</h2>
      <ul className="flex flex-col divide-y divide-border overflow-hidden rounded-10 bg-surface hairline">
        {children}
      </ul>
    </section>
  );
}

/** 设置 on the phone (user decision 2026-10-01): the phone's own recognition and clean-up, its
 *  appearance and recording length, this device and its pairings, and About. A take sent to a
 *  computer still follows the computer's settings. */
export function Settings() {
  const { t, locale } = useI18n();
  const state = useUiState();
  const { engines, settings, presets, identity, devices, app_version: version } = state;
  const pinned = state.scenes.find((s) => s.id === settings.pinned_scene);
  const speech = t("mobile.settings.serviceDetail", {
    provider: t(`engines.provider.${engines.asr_provider}`),
    model: engines.asr_model.length > 0 ? engines.asr_model : "—",
  });
  const ai = settings.engines.refine_enabled
    ? t("mobile.settings.aiDetail", {
        provider:
          engines.llm_provider === undefined ? "—" : t(`engines.provider.${engines.llm_provider}`),
        preset: presetLabel(settings.engines.refine_preset, presets, locale),
      })
    : t("mobile.settings.aiOff");
  const theme = settings.follow_system_theme
    ? t("theme.followSystem")
    : t(`theme.name.${settings.theme}`);
  return (
    <div className="flex flex-col gap-5 p-4" data-testid="phone-settings">
      <p className="px-1 text-[12px] leading-5 text-fg-muted">{t("mobile.settings.own")}</p>
      <Group title={t("mobile.settings.engines")}>
        <Row icon="wave" title={t("mobile.title.speech")} detail={speech} to="speech" />
        <Row icon="sparkles" title={t("mobile.title.ai")} detail={ai} to="ai" />
      </Group>
      <Group title={t("mobile.settings.vocabulary")}>
        <Row
          icon="book"
          title={t("mobile.title.dictionary")}
          detail={t("dictionary.enabledBadge", {
            n: state.dictionary.filter((e) => e.enabled).length,
          })}
          to="dictionary"
        />
        <Row
          icon="edit"
          title={t("mobile.title.rules")}
          detail={t("rules.enabledBadge", { n: state.rules.filter((r) => r.enabled).length })}
          to="rules"
        />
        <Row
          icon="grid"
          title={t("mobile.title.scenes")}
          detail={
            pinned === undefined
              ? t("mobile.settings.scenesDetail", { n: state.scenes.length })
              : t("mobile.settings.pinnedDetail", { name: sceneLabel(pinned, locale) })
          }
          to="scenes"
        />
      </Group>
      <Group title={t("mobile.settings.general")}>
        <Row
          icon="globe"
          title={t("mobile.title.appearance")}
          detail={`${t(`settings.general.locale.${settings.locale}`)} · ${theme}`}
          to="appearance"
        />
        <Row
          icon="mic"
          title={t("mobile.title.recording")}
          detail={t("mobile.settings.recordingDetail", {
            length: lengthLabel(settings.recording.max_minutes, t),
          })}
          to="recording"
        />
      </Group>
      <Group title={t("mobile.settings.computers")}>
        <Row icon="phone" title={t("mobile.settings.device")} detail={identity?.name} to="device" />
        {devices.length > 0 && (
          <Row
            icon="monitor"
            title={t("mobile.title.devices")}
            detail={t("mobile.settings.devicesDetail", { n: devices.length })}
            to="devices"
          />
        )}
      </Group>
      <Group title={t("mobile.settings.aboutGroup")}>
        <Row
          icon="info"
          title={t("mobile.title.about")}
          detail={t("mobile.settings.aboutDetail", { version })}
          to="about"
        />
      </Group>
    </div>
  );
}
