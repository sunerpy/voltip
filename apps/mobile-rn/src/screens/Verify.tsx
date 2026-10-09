// 核对安全码 (docs/pairing.md): the computer the handshake reached and the four words both screens
// show; the same words on both means the channel is honest. Leaving the page with Android's back
// (or the header's) while the comparison is open cancels the pairing, as apps/mobile does.
import { usePreventRemove } from "@react-navigation/native";
import { platformLabel } from "@voltip/shared";
import { useEffect } from "react";
import { View } from "react-native";
import { Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useRootNavigation } from "../routes";
import { Button } from "../ui/Button";
import { Page, Section, StateLine, useAppTheme } from "../ui/kit";
import { SafetyCodeView } from "../ui/SafetyCode";

export function Verify() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { pairing } = useUiState();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const phase = pairing.state.state;

  usePreventRemove(phase === "awaiting_verification", ({ data }) => {
    void backend.invoke("pairing_cancel");
    navigation.dispatch(data.action);
  });
  // Trusted: back to 说话, and the session is reset. Here rather than in the event handler, so the
  // page has seen the new phase (and lets itself be removed) before it goes.
  useEffect(() => {
    if (phase !== "trusted") return;
    navigation.navigate("Tabs", { screen: "Talk" });
    void backend.invoke("pairing_reset");
  }, [phase, navigation, backend]);

  if (phase === "rejected" || phase === "failed" || phase === "expired" || phase === "idle") {
    return (
      <View
        style={{
          flex: 1,
          alignItems: "center",
          justifyContent: "center",
          gap: 16,
          padding: 24,
          backgroundColor: theme.colors.background,
        }}>
        <StateLine tone={phase === "idle" ? "idle" : "danger"}>
          {phase === "rejected"
            ? t("mobile.verify.rejected")
            : phase === "idle"
              ? t("mobile.verify.idle")
              : t("mobile.verify.failed")}
        </StateLine>
        <Button
          mode="contained"
          onPress={() => {
            void backend.invoke("pairing_reset");
            navigation.navigate("Pair");
          }}>
          {t("mobile.verify.again")}
        </Button>
      </View>
    );
  }

  return (
    <Page testID="phone-verify">
      <Section title={t("mobile.verify.peer")} padded>
        <Text variant="headlineSmall">{pairing.peer?.name ?? t("mobile.verify.computer")}</Text>
        <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("mobile.verify.handshake", {
            platform: pairing.peer ? platformLabel(pairing.peer.platform, locale) : "—",
          })}
        </Text>
      </Section>
      <Section title={t("mobile.verify.safetyCode")} padded>
        <Text variant="bodyMedium" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("mobile.verify.note")}
        </Text>
        {pairing.safety_code !== undefined && <SafetyCodeView code={pairing.safety_code} />}
        {pairing.peer_confirmed && (
          <StateLine tone="ok">{t("mobile.verify.peerConfirmed")}</StateLine>
        )}
        {pairing.local_confirmed && !pairing.peer_confirmed && (
          <StateLine tone="accent" pulse>
            {t("mobile.verify.waiting")}
          </StateLine>
        )}
      </Section>
      <Button
        mode="contained"
        icon="check"
        testID="verify-confirm"
        disabled={pairing.local_confirmed}
        onPress={() => void backend.invoke("pairing_confirm")}>
        {pairing.local_confirmed ? t("mobile.verify.waitingButton") : t("mobile.verify.confirm")}
      </Button>
      <Button
        mode="text"
        textColor={theme.colors.error}
        onPress={() => void backend.invoke("pairing_reject")}>
        {t("mobile.verify.reject")}
      </Button>
    </Page>
  );
}
