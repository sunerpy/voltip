// What the phone recognised itself and the takes it sent to a computer (docs/dictation.md §20.7),
// newest first, each on a card of its own (Penpot 「澄空 Glass · Voltip」 board 03): the text on one
// line over when and where it went; the card opens its entry's page, the button at its end copies
// the result and says so for a moment. Sharing is on the entry's page. Nothing to show, no section.
import { type HistoryEntry, type TFunction, relativeTime } from "@voltip/shared";
import { useEffect, useState } from "react";
import { StyleSheet, View } from "react-native";
import { Icon, Text, TouchableRipple } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { cardShadow, useAppTheme } from "../ui/kit";

/** How many of the phone's newest results the list shows (board 03); 记录 has the rest. */
export const RECENT_SHOWN = 3;

/** How long a row says 已复制 after its button copied it. */
export const COPIED_MS = 2000;

/** Characters of a result the buttons' accessible names quote. */
const LABEL_CHARS = 24;

function quoted(text: string): string {
  const chars = Array.from(text.trim());
  return chars.length > LABEL_CHARS ? `${chars.slice(0, LABEL_CHARS).join("")}…` : chars.join("");
}

/** Where a result went: inserted on (or sent to) a computer, or recognised on the phone. */
export function resultPlace(entry: HistoryEntry, t: TFunction): string {
  if (entry.origin?.kind !== "sent") return t("mobile.recent.onPhone");
  return entry.outcome.kind === "inserted"
    ? t("mobile.recent.inserted", { name: entry.origin.device })
    : t("mobile.recent.sent", { name: entry.origin.device });
}

export function RecentResults() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const { history_recent: history } = useUiState();
  const now = useNow();
  const [copied, setCopied] = useState<string | undefined>(undefined);
  useEffect(() => {
    if (copied === undefined) return;
    const id = setTimeout(() => {
      setCopied(undefined);
    }, COPIED_MS);
    return () => {
      clearTimeout(id);
    };
  }, [copied]);
  const shown = history.filter((e) => e.text.trim().length > 0).slice(0, RECENT_SHOWN);
  if (shown.length === 0) return null;

  const copy = (entry: HistoryEntry) => {
    void backend.pasteText(entry.text).then(
      (outcome) => {
        if (outcome.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else setCopied(entry.id);
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };

  return (
    <View style={{ gap: 10 }} testID="phone-recent">
      <View style={styles.head}>
        <Text accessibilityRole="header" style={[styles.title, { color: theme.colors.onSurface }]}>
          {t("mobile.recent.title")}
        </Text>
        <TouchableRipple
          accessibilityRole="link"
          testID="phone-recent-all"
          borderless
          hitSlop={12}
          onPress={() => {
            navigation.navigate("Tabs", { screen: "History" });
          }}>
          <View style={styles.all}>
            <Text style={[styles.allText, { color: theme.colors.primary }]}>
              {t("mobile.recent.all")}
            </Text>
            <Icon source="chevron-right" size={16} color={theme.colors.primary} />
          </View>
        </TouchableRipple>
      </View>
      {shown.map((entry) => {
        const done = copied === entry.id;
        return (
          <TouchableRipple
            key={entry.id}
            testID="phone-recent-row"
            accessibilityRole="button"
            accessibilityLabel={t("mobile.recent.openLabel", { text: quoted(entry.text) })}
            borderless={false}
            onPress={() => {
              navigation.navigate("Entry", { id: entry.id });
            }}
            style={[
              styles.row,
              {
                backgroundColor: theme.voltip.card,
                borderColor: theme.voltip.cardEdge,
                borderRadius: theme.voltip.radius.card,
              },
              cardShadow(theme),
            ]}>
            <View style={styles.rowInner}>
              <View style={{ flex: 1, gap: 4 }}>
                <Text numberOfLines={1} style={[styles.text, { color: theme.colors.onSurface }]}>
                  {entry.text}
                </Text>
                <Text
                  numberOfLines={1}
                  testID="phone-recent-meta"
                  style={[styles.meta, { color: theme.voltip.subtle }]}>
                  {`${relativeTime(Math.floor(entry.at_ms / 1000), now, locale)} · ${resultPlace(entry, t)}`}
                </Text>
              </View>
              {done ? (
                <View
                  testID="phone-recent-copied"
                  accessibilityLiveRegion="polite"
                  style={[styles.copied, { backgroundColor: theme.colors.primaryContainer }]}>
                  <Icon source="check" size={16} color={theme.colors.primary} />
                  <Text style={[styles.copiedText, { color: theme.colors.primary }]}>
                    {t("mobile.recent.copied")}
                  </Text>
                </View>
              ) : (
                <TouchableRipple
                  accessibilityRole="button"
                  accessibilityLabel={t("mobile.recent.copyLabel", { text: quoted(entry.text) })}
                  testID="phone-recent-copy"
                  borderless
                  hitSlop={6}
                  onPress={() => {
                    copy(entry);
                  }}
                  style={[
                    styles.copy,
                    {
                      backgroundColor: theme.voltip.control,
                      borderColor: theme.voltip.controlEdge,
                    },
                  ]}>
                  <Icon source="content-copy" size={16} color={theme.colors.onSurfaceVariant} />
                </TouchableRipple>
              )}
            </View>
          </TouchableRipple>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  head: { flexDirection: "row", alignItems: "center", paddingHorizontal: 4, minHeight: 24 },
  title: { flex: 1, fontSize: 15, lineHeight: 20, fontWeight: "500" },
  all: { flexDirection: "row", alignItems: "center", gap: 2 },
  allText: { fontSize: 13, lineHeight: 18, fontWeight: "500" },
  row: { borderWidth: 1, minHeight: 64, overflow: "hidden" },
  rowInner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingLeft: 16,
    paddingRight: 14,
    paddingVertical: 12,
  },
  text: { fontSize: 14, lineHeight: 19, fontWeight: "500" },
  meta: { fontSize: 12, lineHeight: 16 },
  copy: {
    width: 36,
    height: 36,
    borderRadius: 12,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  copied: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    height: 28,
    borderRadius: 14,
    paddingHorizontal: 9,
  },
  copiedText: { fontSize: 12, lineHeight: 16, fontWeight: "500" },
});
