// 配对电脑 (docs/pairing.md), apps/mobile's PairDevice on native views: the QR code the computer
// shows (the camera page), or its six-digit code; both meet the computer on the relay (docs/
// pairing.md 「只走中继」). The safety code is compared on the next page.
import { failureLabel } from "@voltip/shared";
import { useCameraPermissions } from "expo-camera";
import { useState } from "react";
import { View } from "react-native";
import { ProgressBar, SegmentedButtons, Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useRootNavigation } from "../routes";
import { Button } from "../ui/Button";
import { CodeInput } from "../ui/CodeInput";
import { Lede, Notice, Page, Section, StateLine, useAppTheme } from "../ui/kit";

type Method = "scan" | "code";

export function isPairingLink(value: string): boolean {
  return value.trim().startsWith("voltip://pair?");
}

export function Pair() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { pairing } = useUiState();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const [permission] = useCameraPermissions();
  const [method, setMethod] = useState<Method>("scan");
  const [code, setCode] = useState("");
  const [link, setLink] = useState("");
  const phase = pairing.state.state;
  const busy = phase === "creating_session" || phase === "key_exchange";
  const failed =
    pairing.state.state === "failed"
      ? failureLabel(pairing.state.reason, locale)
      : phase === "expired"
        ? t("mobile.pair.expired")
        : phase === "rejected"
          ? t("mobile.pair.rejected")
          : undefined;
  // A camera the user refused for good: the link from the computer, pasted, instead.
  const noCamera = permission !== null && !permission.granted && !permission.canAskAgain;

  const joinCode = (digits: string) => {
    void backend.invoke("pairing_join_code", { code: digits });
  };
  const joinLink = (uri: string) => {
    void backend.invoke("pairing_join_ticket", { uri: uri.trim() });
  };
  const retry = () => {
    void backend.invoke("pairing_reset");
    setCode("");
  };

  return (
    <Page testID="phone-pair">
      <Lede>{t("mobile.pair.intro")}</Lede>
      <SegmentedButtons
        value={method}
        onValueChange={(v) => {
          setMethod(v);
        }}
        buttons={[
          {
            value: "scan",
            label: t("mobile.pair.scan"),
            icon: "qrcode-scan",
            testID: "pair-method-scan",
          },
          {
            value: "code",
            label: t("mobile.pair.code"),
            icon: "dialpad",
            testID: "pair-method-code",
          },
        ]}
      />
      {method === "scan" &&
        (noCamera ? (
          <Section padded>
            <Text variant="titleSmall">{t("mobile.pair.noCamera")}</Text>
            <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
              {t("mobile.pair.pasteLink")}
            </Text>
            <TextInput
              mode="outlined"
              label={t("mobile.pair.link")}
              value={link}
              placeholder="voltip://pair?v=1&s=…&t=…"
              onChangeText={setLink}
              autoCapitalize="none"
              autoCorrect={false}
              error={link.length > 0 && !isPairingLink(link)}
            />
            {link.length > 0 && !isPairingLink(link) && (
              <Text variant="bodySmall" style={{ color: theme.colors.error }}>
                {t("mobile.pair.notLink")}
              </Text>
            )}
            <Button
              mode="contained"
              disabled={!isPairingLink(link) || busy}
              onPress={() => {
                joinLink(link);
              }}>
              {t("mobile.pair.join")}
            </Button>
          </Section>
        ) : (
          <Section padded>
            <Text variant="titleSmall">{t("mobile.pair.aim")}</Text>
            <Button
              mode="contained"
              icon="camera-outline"
              testID="pair-open-camera"
              disabled={busy}
              onPress={() => {
                navigation.navigate("Scanner");
              }}>
              {t("mobile.pair.openCamera")}
            </Button>
          </Section>
        ))}
      {method === "code" && (
        <Section padded>
          <Text variant="titleSmall" style={{ textAlign: "center" }}>
            {t("mobile.pair.enterCode")}
          </Text>
          <CodeInput
            value={code}
            onChange={setCode}
            onComplete={joinCode}
            disabled={busy}
            autoFocus
            error={failed}
          />
          <Button
            mode="contained"
            testID="pair-join-code"
            disabled={code.length !== 6 || busy}
            onPress={() => {
              joinCode(code);
            }}>
            {t("mobile.pair.join")}
          </Button>
          {failed !== undefined && (
            <Button mode="text" onPress={retry}>
              {t("mobile.pair.clearRetry")}
            </Button>
          )}
        </Section>
      )}
      {busy && (
        <View style={{ gap: 8, paddingHorizontal: 4 }} accessibilityLiveRegion="polite">
          <StateLine tone="accent" pulse>
            {phase === "creating_session" ? t("mobile.pair.joining") : t("mobile.pair.negotiating")}
          </StateLine>
          <ProgressBar indeterminate />
        </View>
      )}
      {failed !== undefined && method === "scan" && (
        <Notice
          tone="danger"
          action={
            <Button mode="outlined" compact style={{ alignSelf: "flex-start" }} onPress={retry}>
              {t("mobile.pair.retry")}
            </Button>
          }>
          {failed}
        </Notice>
      )}
    </Page>
  );
}
