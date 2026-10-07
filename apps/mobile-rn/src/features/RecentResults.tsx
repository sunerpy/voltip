// What the phone recognised itself and the takes it sent to a computer (docs/dictation.md §20.7),
// newest first: a row opens its entry's page, and copies or shares the result with the two buttons
// at its end. Nothing to show, no section. apps/mobile's RecentResults on native views.
import { type HistoryEntry, relativeTime } from "@voltip/shared";
import { View } from "react-native";
import { IconButton, Text, TouchableRipple } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { Hint, Mono, RowDivider, Section, useAppTheme } from "../ui/kit";

/** How many of the phone's newest results the list shows. */
export const RECENT_SHOWN = 10;

/** Characters of a result the buttons' accessible names quote. */
const LABEL_CHARS = 24;

function quoted(text: string): string {
  const chars = Array.from(text.trim());
  return chars.length > LABEL_CHARS ? `${chars.slice(0, LABEL_CHARS).join("")}…` : chars.join("");
}

export function RecentResults() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const { history_recent: history } = useUiState();
  const now = useNow();
  const shown = history.filter((e) => e.text.trim().length > 0).slice(0, RECENT_SHOWN);
  if (shown.length === 0) return null;

  const copy = (entry: HistoryEntry) => {
    void backend.pasteText(entry.text).then(
      (outcome) => {
        if (outcome.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("mobile.recent.copied"));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };
  const share = (entry: HistoryEntry) => {
    backend.invoke("phone_share_text", { text: entry.text }).catch((e: unknown) => {
      shell.toast(
        t("mobile.toast.error", { message: e instanceof Error ? e.message : String(e) }),
        "danger",
      );
    });
  };

  return (
    <Section
      title={t("mobile.recent.title")}
      testID="phone-recent"
      right={
        <Button
          compact
          onPress={() => {
            navigation.navigate("Tabs", { screen: "History" });
          }}>
          {t("mobile.recent.all")}
        </Button>
      }
      footer={<Hint>{t("mobile.recent.body")}</Hint>}>
      {shown.map((entry, i) => (
        <View key={entry.id} testID="phone-recent-row">
          {i > 0 && <RowDivider />}
          <TouchableRipple
            accessibilityRole="button"
            accessibilityLabel={t("mobile.recent.openLabel", { text: quoted(entry.text) })}
            onPress={() => {
              navigation.navigate("Entry", { id: entry.id });
            }}>
            <View style={{ paddingLeft: 16, paddingTop: 12, paddingRight: 4 }}>
              <Text variant="bodyLarge" numberOfLines={3}>
                {entry.text}
              </Text>
              <View style={{ flexDirection: "row", alignItems: "center" }}>
                <Mono style={{ flex: 1, color: theme.voltip.subtle }}>
                  {relativeTime(Math.floor(entry.at_ms / 1000), now, locale)}
                </Mono>
                <IconButton
                  icon="content-copy"
                  size={20}
                  accessibilityLabel={t("mobile.recent.copyLabel", { text: quoted(entry.text) })}
                  onPress={() => {
                    copy(entry);
                  }}
                />
                <IconButton
                  icon="share-variant-outline"
                  size={20}
                  accessibilityLabel={t("mobile.recent.shareLabel", { text: quoted(entry.text) })}
                  onPress={() => {
                    share(entry);
                  }}
                />
              </View>
            </View>
          </TouchableRipple>
        </View>
      ))}
    </Section>
  );
}
