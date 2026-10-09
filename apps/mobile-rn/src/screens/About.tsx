// 关于 (apps/mobile's About): the version, the licence, the project's pages and the privacy
// policy, opened in the phone's browser by URLs the shell builds (`project_link_open`,
// `guide_open`), and 软件更新 (docs/dictation.md §20.9): Google Play updates what it installed, so
// its install only opens the listing; any other install asks GitHub (检查更新, or at start with
// 自动检查更新) and opens the newer release's APK in the browser, which Android installs over this
// one.
import {
  APP_LICENSE,
  type ProjectLink,
  type UpdateStatus,
  coreMessageText,
  formatDateTime,
} from "@voltip/shared";
import { View } from "react-native";
import { List, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  FactRow,
  Hint,
  Page,
  RowDivider,
  Rows,
  Section,
  StateLine,
  SwitchRow,
  type Tone,
  useAppTheme,
} from "../ui/kit";
import { Logo } from "../ui/Logo";

type T = ReturnType<typeof useI18n>["t"];

/** The lamp beside the update line, as the line reads it (apps/mobile's About): the desktop's
 *  tones, with the download steps (which the phone words as a newer version) lit as one is. */
const UPDATE_TONE: Record<UpdateStatus["state"], Tone> = {
  idle: "idle",
  checking: "accent",
  up_to_date: "ok",
  available: "accent",
  downloading: "accent",
  ready: "accent",
  installing: "accent",
  failed: "danger",
  store: "idle",
  disabled: "idle",
};

/** The updater's state in one line, in the desktop's words (`settings.general.update`); `current`
 *  is this build's version. */
function updateLine(
  update: UpdateStatus,
  t: T,
  formatAt: (unixSecs: number) => string,
  current: string,
): string {
  switch (update.state) {
    case "idle":
      return t("settings.general.update.idle");
    case "checking":
      return t("settings.general.update.checking");
    case "up_to_date":
      return t("settings.general.update.up_to_date", {
        version: update.version,
        at: formatAt(update.checked_at),
      });
    case "available":
      return t("settings.general.update.available", {
        version: update.version,
        current: update.current,
      });
    case "failed":
      return t("settings.general.update.failed", { message: coreMessageText(update.message) });
    // The desktop's download steps (and the store, which has a card of its own) never reach the
    // phone's line: a newer version is all they say.
    case "downloading":
    case "ready":
    case "installing":
    case "store":
      return t("settings.general.update.available", { version: update.version, current });
    case "disabled":
      return t("settings.general.update.disabled");
  }
}

/** 软件更新: the listing for an install from Google Play, the release check for any other. */
function UpdateCard({ fail }: { fail: (e: unknown) => void }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const { update, settings, app_version: version } = useUiState();
  const install = () => {
    backend.invoke("update_install").catch(fail);
  };
  if (update.state === "disabled") return null;
  if (update.state === "store") {
    return (
      <Section title={t("mobile.about.update.storeTitle")} testID="phone-update" padded>
        <View style={{ gap: 12 }}>
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("mobile.about.update.storeBody")}
          </Text>
          <View style={{ flexDirection: "row" }}>
            <Button mode="outlined" icon="open-in-new" onPress={install}>
              {t("mobile.about.update.openStore")}
            </Button>
          </View>
        </View>
      </Section>
    );
  }
  const line = updateLine(
    update,
    t,
    (secs) => formatDateTime(locale, secs * 1000, { dateStyle: "medium", timeStyle: "short" }),
    version,
  );
  return (
    <Section title={t("mobile.about.update.title")} testID="phone-update">
      <View style={{ padding: 16, gap: 12 }}>
        <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("mobile.about.update.directBody")}
        </Text>
        <StateLine
          tone={UPDATE_TONE[update.state]}
          pulse={update.state === "checking"}
          testID="phone-update-status">
          {line}
        </StateLine>
        {update.state === "available" && (
          <View style={{ gap: 12 }}>
            {update.notes !== undefined && (
              <List.Accordion
                title={t("mobile.about.update.notes")}
                testID="phone-update-notes"
                style={{ paddingHorizontal: 0 }}>
                <Text variant="bodySmall" selectable>
                  {update.notes}
                </Text>
              </List.Accordion>
            )}
            <View style={{ flexDirection: "row" }}>
              <Button
                mode="contained"
                icon="download"
                onPress={install}
                testID="phone-update-download">
                {t("mobile.about.update.download")}
              </Button>
            </View>
            <Hint>{t("mobile.about.update.downloadHint")}</Hint>
          </View>
        )}
        <View style={{ flexDirection: "row" }}>
          <Button
            mode="outlined"
            disabled={update.state === "checking"}
            onPress={() => {
              backend.invoke("update_check").catch(fail);
            }}
            testID="phone-update-check">
            {t("mobile.about.update.check")}
          </Button>
        </View>
      </View>
      <RowDivider />
      <Rows>
        <SwitchRow
          title={t("mobile.about.update.auto")}
          description={t("mobile.about.update.autoHelp")}
          value={settings.auto_update}
          onValueChange={(enabled) => {
            backend.invoke("settings_set_auto_update", { enabled }).catch(fail);
          }}
          testID="phone-update-auto"
        />
      </Rows>
    </Section>
  );
}

export function About() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { app_version: version } = useUiState();
  const fail = (e: unknown) => {
    shell.toast(
      t("mobile.toast.error", {
        message: coreMessageText(e instanceof Error ? e.message : String(e)),
      }),
      "danger",
    );
  };
  const open = (link: ProjectLink) => {
    backend.projectLinkOpen(link).catch(fail);
  };
  return (
    <Page testID="phone-about">
      <Section>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12, padding: 16 }}>
          <Logo size={40} />
          <Text variant="titleLarge">Voltip</Text>
        </View>
        <RowDivider />
        <FactRow label={t("mobile.about.version")} value={version} mono />
        <RowDivider />
        <FactRow label={t("mobile.about.license")} value={APP_LICENSE} mono />
        <RowDivider />
        <View style={{ padding: 16, gap: 12 }}>
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("mobile.about.licenseBody")}
          </Text>
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("mobile.about.notices")}
          </Text>
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
            <Button
              mode="outlined"
              icon="open-in-new"
              onPress={() => {
                open("source");
              }}>
              {t("mobile.about.source")}
            </Button>
            <Button
              mode="outlined"
              icon="open-in-new"
              onPress={() => {
                open("releases");
              }}>
              {t("mobile.about.releases")}
            </Button>
            <Button
              mode="outlined"
              icon="open-in-new"
              onPress={() => {
                backend.guideOpen("privacy", locale).catch(fail);
              }}>
              {t("mobile.about.privacy")}
            </Button>
          </View>
        </View>
      </Section>
      <UpdateCard fail={fail} />
    </Page>
  );
}
