// 关于 (apps/mobile's About): the version, the licence and the project's pages, opened in the
// phone's browser by URLs the shell builds (`project_link_open`). This build has no update source
// (docs/mobile-rn.md §1): its update status is `disabled`, so there is no update card.
import { APP_LICENSE, type ProjectLink, coreMessageText } from "@voltip/shared";
import { View } from "react-native";
import { Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { FactRow, Page, RowDivider, Section, useAppTheme } from "../ui/kit";
import { Logo } from "../ui/Logo";

export function About() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const { app_version: version } = useUiState();
  const open = (link: ProjectLink) => {
    backend.projectLinkOpen(link).catch((e: unknown) => {
      shell.toast(
        t("mobile.toast.error", {
          message: coreMessageText(e instanceof Error ? e.message : String(e)),
        }),
        "danger",
      );
    });
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
          </View>
        </View>
      </Section>
    </Page>
  );
}
