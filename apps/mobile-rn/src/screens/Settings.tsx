// 设置, the third tab (apps/mobile's Settings): the phone's own recognition and clean-up, the
// built-in service's usage and the floating bubble, its vocabulary, appearance, history and
// recording length, its network proxy, this device and its pairings, and About. A take sent to a
// computer still follows the computer's settings.
import { useFocusEffect } from "@react-navigation/native";
import {
  type TFunction,
  formatCount,
  mirrorStateText,
  presetLabel,
  sceneLabel,
  usageSummary,
} from "@voltip/shared";
import { useCallback, useState } from "react";

import { useUiState } from "../backend/BackendProvider";
import { type BubbleControl, useBubble } from "../backend/bubble";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { type RootParams, useRootNavigation } from "../routes";
import { Lede, NavRow, Page, Rows, Section, TabTitle } from "../ui/kit";

/** `60` → 「1 小时」, `30` → 「30 分钟」 (as 设置 › 听写 on the desktop says it). */
export function lengthLabel(minutes: number, t: TFunction): string {
  return minutes >= 60 && minutes % 60 === 0
    ? t("settings.dictation.hours", { n: minutes / 60 })
    : t("settings.dictation.minutes", { n: minutes });
}

type Plain = {
  [K in keyof RootParams]: RootParams[K] extends undefined ? K : never;
}[keyof RootParams];

/** Whether the bubble shows, read again whenever 设置 comes into view. */
function useBubbleRunning(control: BubbleControl | undefined): boolean {
  const [running, setRunning] = useState(false);
  useFocusEffect(
    useCallback(() => {
      if (control === undefined) return;
      let alive = true;
      control.status().then(
        (status) => {
          if (alive) setRunning(status.running);
        },
        () => undefined,
      );
      return () => {
        alive = false;
      };
    }, [control]),
  );
  return running;
}

export function Settings() {
  const { t, locale } = useI18n();
  const navigation = useRootNavigation();
  const state = useUiState();
  const { engines, settings, presets, identity, devices, mirrors, app_version: version } = state;
  const now = useNow();
  const bubble = useBubble();
  const bubbleRunning = useBubbleRunning(bubble);
  const open = (to: Plain) => () => {
    (navigation.navigate as (screen: Plain) => void)(to);
  };
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
    <Page testID="phone-settings" gap={20} title={<TabTitle title={t("mobile.tab.settings")} />}>
      <Lede>{t("mobile.settings.own")}</Lede>
      <Section title={t("mobile.settings.engines")}>
        <Rows>
          <NavRow
            icon="waveform"
            title={t("mobile.title.speech")}
            description={speech}
            onPress={open("Speech")}
            testID="settings-speech"
          />
          <NavRow
            icon="creation-outline"
            title={t("mobile.title.ai")}
            description={ai}
            onPress={open("Ai")}
            testID="settings-ai"
          />
          {/* docs/dictation.md §27: this phone's day of the built-in service (board 16a). */}
          {state.usage.builtin && (
            <NavRow
              icon="gauge"
              title={t("mobile.title.usage")}
              description={usageSummary(state.usage, now * 1000, locale)}
              onPress={open("Usage")}
              testID="settings-usage"
            />
          )}
          {/* docs/dictation.md §20.12: the talk button over other apps. */}
          {bubble !== undefined && (
            <NavRow
              icon="gesture-tap-button"
              title={t("mobile.title.bubble")}
              description={
                bubbleRunning ? t("mobile.settings.bubbleOn") : t("mobile.settings.bubbleOff")
              }
              onPress={open("Bubble")}
              testID="settings-bubble"
            />
          )}
        </Rows>
      </Section>
      <Section title={t("mobile.settings.vocabulary")}>
        <Rows>
          <NavRow
            icon="book-open-variant"
            title={t("mobile.title.dictionary")}
            description={t("dictionary.enabledBadge", {
              n: state.dictionary.filter((e) => e.enabled).length,
            })}
            onPress={open("Dictionary")}
            testID="settings-dictionary"
          />
          <NavRow
            icon="find-replace"
            title={t("mobile.title.rules")}
            description={t("rules.enabledBadge", {
              n: state.rules.filter((r) => r.enabled).length,
            })}
            onPress={open("Rules")}
            testID="settings-rules"
          />
          <NavRow
            icon="view-grid-outline"
            title={t("mobile.title.scenes")}
            description={
              pinned === undefined
                ? t("mobile.settings.scenesDetail", { n: state.scenes.length })
                : t("mobile.settings.pinnedDetail", { name: sceneLabel(pinned, locale) })
            }
            onPress={open("Scenes")}
            testID="settings-scenes"
          />
        </Rows>
      </Section>
      <Section title={t("mobile.settings.general")}>
        <Rows>
          <NavRow
            icon="palette-outline"
            title={t("mobile.title.appearance")}
            description={`${t(`settings.general.locale.${settings.locale}`)} · ${theme}`}
            onPress={open("Appearance")}
            testID="settings-appearance"
          />
          <NavRow
            icon="history"
            title={t("mobile.title.historySettings")}
            description={
              settings.history.enabled
                ? t("mobile.settings.historyDetail", { keep: formatCount(settings.history.keep) })
                : t("mobile.settings.historyOff")
            }
            onPress={open("HistorySettings")}
            testID="settings-historySettings"
          />
          <NavRow
            icon="microphone-outline"
            title={t("mobile.title.recording")}
            description={t("mobile.settings.recordingDetail", {
              length: lengthLabel(settings.recording.max_minutes, t),
            })}
            onPress={open("Recording")}
            testID="settings-recording"
          />
          <NavRow
            icon="web"
            title={t("mobile.title.network")}
            description={
              settings.proxy.mode === "manual"
                ? t("mobile.settings.networkManual", { server: settings.proxy.server ?? "—" })
                : settings.proxy.mode === "off"
                  ? t("mobile.settings.networkOff")
                  : t("mobile.settings.networkSystem")
            }
            onPress={open("Network")}
            testID="settings-network"
          />
        </Rows>
      </Section>
      <Section title={t("mobile.settings.computers")}>
        <Rows>
          <NavRow
            icon="cellphone"
            title={t("mobile.settings.device")}
            {...(identity === null ? {} : { description: identity.name })}
            onPress={open("ThisDevice")}
            testID="settings-device"
          />
          {devices.length > 0 && (
            <NavRow
              icon="monitor"
              title={t("mobile.title.devices")}
              description={t("mobile.settings.devicesDetail", { n: devices.length })}
              onPress={() => {
                navigation.navigate("Tabs", { screen: "Talk" });
              }}
              testID="settings-devices"
            />
          )}
          {/* docs/dictation.md §20.8: each computer's settings, read-only. */}
          {mirrors.map((m) => (
            <NavRow
              key={m.desktop}
              icon="cog-outline"
              title={t("mirror.settingsRow", { name: m.name })}
              description={mirrorStateText(m, now, locale)}
              onPress={() => {
                navigation.navigate("ComputerSettings", { desktop: m.desktop });
              }}
              testID={`settings-computerSettings-${m.desktop}`}
            />
          ))}
        </Rows>
      </Section>
      <Section title={t("mobile.settings.aboutGroup")}>
        <Rows>
          <NavRow
            icon="message-text-outline"
            title={t("mobile.title.feedback")}
            description={t("mobile.settings.feedbackDetail")}
            onPress={open("Feedback")}
            testID="settings-feedback"
          />
          <NavRow
            icon="information-outline"
            title={t("mobile.title.about")}
            description={t("mobile.settings.aboutDetail", { version })}
            onPress={open("About")}
            testID="settings-about"
          />
        </Rows>
      </Section>
    </Page>
  );
}
