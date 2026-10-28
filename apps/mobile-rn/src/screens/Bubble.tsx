// 设置 › 悬浮球 (docs/dictation.md §20.12; Penpot 「用量与额度」 boards 18a/18b): a talk button over
// other apps whose result goes onto the clipboard. Until the microphone, 显示在其他应用上层 and the
// notification are granted, the page lists them with the way to each; then how to talk (and how to
// tuck the bubble into the edge), whether the take is polished (the same switch as 说话 and AI 模型),
// whether the result shows beside the bubble and whether 不复制 shows with it. The page reads the
// state again whenever it comes back into view: a permission is granted in the system's settings,
// outside the app.
import { useFocusEffect } from "@react-navigation/native";
import { usageInline } from "@voltip/shared";
import { useCallback, useEffect, useState } from "react";
import { Alert, AppState, StyleSheet, View } from "react-native";
import { Icon, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import {
  type BubbleConfig,
  type BubbleControl,
  type BubbleMode,
  type BubbleStatus,
  bubbleStrings,
  grantedCount,
  useBubble,
} from "../backend/bubble";
import { useI18n } from "../backend/i18n";
import { useBuiltinInUse, useServiceUsage, useUsageNow } from "../features/Usage";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  Hint,
  Lede,
  Page,
  Rows,
  Section,
  Segmented,
  SwitchRow,
  rgba,
  useAppTheme,
} from "../ui/kit";

/** The bubble's state, read again whenever the page comes into view or the app to the front. */
function useBubbleStatus(control: BubbleControl): [BubbleStatus | undefined, () => void] {
  const [status, setStatus] = useState<BubbleStatus | undefined>(undefined);
  const read = useCallback(() => {
    control.status().then(setStatus, () => undefined);
  }, [control]);
  useFocusEffect(read);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (next) => {
      if (next === "active") read();
    });
    return () => {
      subscription.remove();
    };
  }, [read]);
  return [status, read];
}

type PermissionId = "microphone" | "overlay" | "notifications";

const PERMISSION_ICON: Readonly<Record<PermissionId, string>> = {
  microphone: "microphone-outline",
  overlay: "picture-in-picture-bottom-right",
  notifications: "message-text-outline",
};

/** One permission: what it is for, and 已允许 or the way to grant it. */
function PermissionRow({
  id,
  granted,
  onGrant,
}: {
  id: PermissionId;
  granted: boolean;
  onGrant: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  return (
    <View style={styles.permission} testID={`bubble-permission-${id}`}>
      <View
        style={[
          styles.permissionIcon,
          {
            backgroundColor: granted ? theme.voltip.okSoft : rgba(theme.colors.primary, 0.1),
          },
        ]}>
        <Icon
          source={granted ? "check" : PERMISSION_ICON[id]}
          size={18}
          color={granted ? theme.voltip.okText : theme.colors.primary}
        />
      </View>
      <View style={{ flex: 1, gap: 2 }}>
        <Text variant="titleSmall">{t(`mobile.bubble.${id}`)}</Text>
        <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
          {t(`mobile.bubble.${id}Body`)}
        </Text>
      </View>
      {granted ? (
        <Text
          testID={`bubble-permission-${id}-granted`}
          style={[styles.granted, { color: theme.voltip.okText }]}>
          {t("mobile.bubble.granted")}
        </Text>
      ) : (
        <Button
          compact
          mode={id === "overlay" ? "contained" : "outlined"}
          testID={`bubble-permission-${id}-grant`}
          onPress={onGrant}>
          {id === "overlay" ? t("mobile.bubble.openSettings") : t("mobile.bubble.allow")}
        </Button>
      )}
    </View>
  );
}

function BubblePage({ control }: { control: BubbleControl }) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const shell = useShell();
  const { settings } = useUiState();
  const [status, read] = useBubbleStatus(control);
  const usage = useServiceUsage("asr");
  const builtin = useBuiltinInUse("asr");
  const now = useUsageNow();
  if (status === undefined) return <Page testID="phone-bubble">{null}</Page>;
  const done = grantedCount(status.permissions);
  const ready = done === 3;
  const save = (change: Partial<Omit<BubbleConfig, "strings">>) => {
    return control
      .configure({
        enabled: status.enabled,
        mode: status.mode,
        showResult: status.showResult,
        showDiscard: status.showDiscard,
        pasteEnabled: status.pasteEnabled,
        autoStowSeconds: status.autoStowSeconds,
        ...change,
        strings: bubbleStrings(t),
      })
      .then(
        (next) => {
          if (change.enabled === true && !next.running) {
            shell.toast(t("mobile.bubble.notStarted"), "danger");
          }
          read();
          return true;
        },
        (e: unknown) => {
          shell.toast(
            t("mobile.bubble.failed", { message: e instanceof Error ? e.message : String(e) }),
            "danger",
          );
          read();
          return false;
        },
      );
  };
  const grant = (id: PermissionId) => () => {
    const ask =
      id === "microphone"
        ? control.requestMicrophone()
        : id === "notifications"
          ? control.requestNotifications()
          : control.openOverlaySettings().then(() => false);
    ask.then(read, read);
  };
  return (
    <Page testID="phone-bubble" gap={20}>
      <Lede>{t("mobile.bubble.lede")}</Lede>
      <Section>
        <SwitchRow
          title={t("mobile.bubble.show")}
          description={status.running ? t("mobile.bubble.on") : t("mobile.bubble.needs")}
          value={status.running}
          disabled={!ready}
          onValueChange={(on) => {
            save({ enabled: on });
          }}
          testID="bubble-toggle"
        />
        {!ready && (
          <View
            style={[styles.progress, { borderTopColor: theme.colors.outlineVariant }]}
            testID="bubble-progress">
            <Text style={[styles.progressText, { color: theme.voltip.warning }]}>
              {t("mobile.bubble.progress", { done, total: 3 })}
            </Text>
          </View>
        )}
      </Section>
      {!ready && (
        <Section title={t("mobile.bubble.permissions")}>
          <Rows>
            <PermissionRow
              id="microphone"
              granted={status.permissions.microphone}
              onGrant={grant("microphone")}
            />
            <PermissionRow
              id="overlay"
              granted={status.permissions.overlay}
              onGrant={grant("overlay")}
            />
            <PermissionRow
              id="notifications"
              granted={status.permissions.notifications}
              onGrant={grant("notifications")}
            />
          </Rows>
        </Section>
      )}
      {ready && (
        <>
          <Section
            title={t("mobile.bubble.mode")}
            padded
            footer={<Hint testID="bubble-stow-hint">{t("mobile.bubble.stowHint")}</Hint>}>
            <Segmented<BubbleMode>
              testID="bubble-mode"
              label={t("mobile.bubble.mode")}
              value={status.mode}
              onChange={(mode) => {
                save({ mode });
              }}
              options={[
                { value: "hold", label: t("mobile.bubble.modeHold"), testID: "bubble-mode-hold" },
                { value: "tap", label: t("mobile.bubble.modeTap"), testID: "bubble-mode-tap" },
              ]}
            />
          </Section>
          <Section title={t("mobile.bubble.gestures")}>
            <Rows>
              <SwitchRow
                title={t("mobile.bubble.pasteEnabled")}
                description={t("mobile.bubble.pasteEnabledBody")}
                value={status.pasteEnabled}
                onValueChange={(enabled) => {
                  if (!enabled) {
                    void save({ pasteEnabled: false });
                    return;
                  }
                  Alert.alert(t("mobile.bubble.pasteDisclosureTitle"), t("mobile.bubble.pasteDisclosure"), [
                    { text: t("mobile.bubble.pasteDecline"), style: "cancel" },
                    {
                      text: t("mobile.bubble.pasteAgree"),
                      onPress: () => {
                        void save({ pasteEnabled: true }).then((saved) => {
                          if (saved && !status.accessibilityEnabled) return control.openAccessibilitySettings();
                        }).catch(() => undefined);
                      },
                    },
                  ]);
                }}
                testID="bubble-paste-enabled"
              />
            </Rows>
            {status.pasteEnabled && !status.accessibilityEnabled && (
              <View style={{ paddingHorizontal: 16, paddingBottom: 12, gap: 8 }}>
                <Hint testID="bubble-paste-needs-accessibility">{t("mobile.bubble.pasteNeedsService")}</Hint>
                <Button compact testID="bubble-paste-open-settings" onPress={() => { void control.openAccessibilitySettings(); }}>
                  {t("mobile.bubble.openSettings")}
                </Button>
              </View>
            )}
          </Section>
          <Section title={t("mobile.bubble.autoStow")} padded>
            <Segmented<"0" | "5" | "10" | "30">
              label={t("mobile.bubble.autoStow")}
              testID="bubble-auto-stow"
              value={String(status.autoStowSeconds) as "0" | "5" | "10" | "30"}
              onChange={(seconds) => { void save({ autoStowSeconds: Number(seconds) as 0 | 5 | 10 | 30 }); }}
              options={[
                { value: "0", label: t("mobile.bubble.autoStowOff"), testID: "bubble-auto-stow-off" },
                { value: "5", label: "5s", testID: "bubble-auto-stow-5" },
                { value: "10", label: "10s", testID: "bubble-auto-stow-10" },
                { value: "30", label: "30s", testID: "bubble-auto-stow-30" },
              ]}
            />
          </Section>
          <Section title={t("mobile.bubble.result")}>
            <Rows>
              <SwitchRow
                title={t("mobile.bubble.refine")}
                description={t("mobile.bubble.refineBody")}
                value={settings.engines.refine_enabled}
                onValueChange={(refine_enabled) => {
                  void backend.invoke("settings_set_engines", {
                    engines: { ...settings.engines, refine_enabled },
                  });
                }}
                testID="bubble-refine"
              />
              <SwitchRow
                title={t("mobile.bubble.showResult")}
                description={t("mobile.bubble.showResultBody")}
                value={status.showResult}
                onValueChange={(showResult) => {
                  save({ showResult });
                }}
                testID="bubble-show-result"
              />
              <SwitchRow
                title={t("mobile.bubble.showDiscard")}
                description={
                  status.showResult
                    ? t("mobile.bubble.showDiscardBody")
                    : t("mobile.bubble.showDiscardNeeds")
                }
                value={status.showDiscard}
                disabled={!status.showResult}
                onValueChange={(showDiscard) => {
                  save({ showDiscard });
                }}
                testID="bubble-show-discard"
              />
            </Rows>
          </Section>
          {builtin && usage.tone !== "pending" && (
            <Hint testID="bubble-usage">
              {t("mobile.bubble.usage", { usage: usageInline(usage, now, locale) })}
            </Hint>
          )}
        </>
      )}
      <View style={{ gap: 4 }}>
        <Hint>{t("mobile.bubble.vendor")}</Hint>
        <Button
          compact
          style={{ alignSelf: "flex-start" }}
          testID="bubble-vendor"
          onPress={() => {
            backend.guideOpen("bubble", locale).catch(() => undefined);
          }}>
          {t("mobile.bubble.vendorLink")}
        </Button>
      </View>
    </Page>
  );
}

export function Bubble() {
  const control = useBubble();
  if (control === undefined) return null;
  return <BubblePage control={control} />;
}

const styles = StyleSheet.create({
  progress: {
    borderTopWidth: StyleSheet.hairlineWidth,
    marginHorizontal: 16,
    paddingVertical: 12,
  },
  progressText: { fontSize: 13, lineHeight: 18, fontWeight: "500" },
  permission: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
  },
  permissionIcon: {
    width: 36,
    height: 36,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  granted: { fontSize: 13, lineHeight: 18, fontWeight: "500" },
});
