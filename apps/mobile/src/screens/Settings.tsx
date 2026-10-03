import {
  type TFunction,
  formatCount,
  mirrorStateText,
  presetLabel,
  sceneLabel,
} from "@voltip/shared";
import { type IconName, SettingsSection, useI18n, useNow, useUiState } from "@voltip/ui";
import type { ReactNode } from "react";
import { Lede, NavRow, RowList } from "../app/phone-ui";
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
  param,
}: {
  icon: IconName;
  title: string;
  detail?: ReactNode;
  to: Screen;
  /** What `to` shows (`MobileShell.param`). */
  param?: string;
}) {
  const shell = useMobileShell();
  return (
    <NavRow
      icon={icon}
      title={title}
      detail={detail}
      data-testid={param === undefined ? `settings-${to}` : `settings-${to}-${param}`}
      onOpen={() => {
        shell.go(to, param);
      }}
    />
  );
}

/** A group of the list, as the desktop's settings sections: the eyebrow label, then the rows in
 *  one hairline card. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <SettingsSection title={title}>
      <RowList>{children}</RowList>
    </SettingsSection>
  );
}

/** 设置 on the phone (user decision 2026-10-01): the phone's own recognition and clean-up, its
 *  appearance and recording length, this device and its pairings, and About. A take sent to a
 *  computer still follows the computer's settings. */
export function Settings() {
  const { t, locale } = useI18n();
  const state = useUiState();
  const { engines, settings, presets, identity, devices, mirrors, app_version: version } = state;
  const now = useNow();
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
    <div className="flex flex-col gap-6 p-4" data-testid="phone-settings">
      <Lede>{t("mobile.settings.own")}</Lede>
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
          icon="history"
          title={t("mobile.title.historySettings")}
          detail={
            settings.history.enabled
              ? t("mobile.settings.historyDetail", { keep: formatCount(settings.history.keep) })
              : t("mobile.settings.historyOff")
          }
          to="historySettings"
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
        {/* docs/dictation.md §20.8: each computer's settings, read-only. */}
        {mirrors.map((m) => (
          <Row
            key={m.desktop}
            icon="settings"
            title={t("mirror.settingsRow", { name: m.name })}
            detail={mirrorStateText(m, now, locale)}
            to="computerSettings"
            param={m.desktop}
          />
        ))}
      </Group>
      <Group title={t("mobile.settings.aboutGroup")}>
        <Row
          icon="chat"
          title={t("mobile.title.feedback")}
          detail={t("mobile.settings.feedbackDetail")}
          to="feedback"
        />
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
