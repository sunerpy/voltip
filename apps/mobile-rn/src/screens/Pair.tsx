// 配对电脑 (docs/pairing.md), apps/mobile's PairDevice on native views: the QR code the computer
// shows (the camera page), or its six-digit code; both meet the computer on the relay (docs/
// pairing.md 「只走中继」). The safety code is compared on the next page. The relay link the pairing
// needs is shown below it: its state, why it is not connected, and the self-check (issue #193).
// Below the relay's minimum version the page leads to the update instead (docs/pairing.md
// 「最低版本」).
import { type Tone as LabelTone, failureLabel, relayLabel, relayTooOld } from "@voltip/shared";
import { useCameraPermissions } from "expo-camera";
import { useState } from "react";
import { View } from "react-native";
import { Icon, ProgressBar, SegmentedButtons, Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { ConnectivityCheck } from "../features/Devices";
import { useRootNavigation } from "../routes";
import { Button } from "../ui/Button";
import { CodeInput } from "../ui/CodeInput";
import { Lede, Notice, Page, Section, StateLine, type Tone, useAppTheme } from "../ui/kit";

type Method = "scan" | "code";

export function isPairingLink(value: string): boolean {
  return value.trim().startsWith("voltip://pair?");
}

/** A label's tone as the kit's lamps have it. */
function lampTone(tone: LabelTone): Tone {
  switch (tone) {
    case "warn":
      return "warning";
    case "neutral":
      return "idle";
    default:
      return tone;
  }
}

/** The relay the pairing needs (docs/pairing.md 「只走中继」): its state, and while it is not
 *  connected why the last attempt failed and the self-check. Issue #193: the page said only
 *  「尚未连上中继」, and only once a code was entered. */
function RelayLink() {
  const theme = useAppTheme();
  const { relay, connectivity } = useUiState();
  const { t, locale } = useI18n();
  const label = relayLabel(relay, locale);
  const connected = relay.state === "connected";
  const failure = relay.last_failure;
  let why: string | undefined;
  if (!connected && failure !== undefined) {
    const reason = t(`relay.why.${failure.kind}`);
    why = t("relay.lastFailure", {
      reason:
        failure.detail === undefined
          ? reason
          : t("mobile.pair.relayDetail", { reason, detail: failure.detail }),
    });
  }
  return (
    <>
      <Section padded testID="pair-relay">
        <StateLine tone={lampTone(label.tone)} testID="pair-relay-state">
          {t("mobile.pair.relay", { state: label.text })}
        </StateLine>
        {why !== undefined && (
          <Text
            variant="bodySmall"
            testID="pair-relay-failure"
            style={{ color: theme.colors.error }}>
            {why}
          </Text>
        )}
        {!connected && (
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("mobile.pair.relayHelp")}
          </Text>
        )}
      </Section>
      {!connected && <ConnectivityCheck status={connectivity} />}
    </>
  );
}

/** Below the relay's minimum version (board 20a): the version it needs, and the update. Google Play
 *  updates what it installed; any other install checks for the release on 关于. */
function TooOld({ minVersion }: { minVersion: string | undefined }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { update, app_version: current } = useUiState();
  const navigation = useRootNavigation();
  const { t } = useI18n();
  const store = update.state === "store";
  return (
    <Section padded testID="pair-too-old">
      <View style={{ alignItems: "center", gap: 12, paddingVertical: 8 }}>
        <View
          style={{
            width: 72,
            height: 72,
            borderRadius: 24,
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: theme.voltip.warningSoft,
          }}>
          <Icon source="download" size={36} color={theme.voltip.warning} />
        </View>
        <Text variant="titleMedium" style={{ textAlign: "center" }}>
          {t("mobile.pair.tooOld.title")}
        </Text>
        <Text
          variant="bodyMedium"
          style={{ color: theme.colors.onSurfaceVariant, textAlign: "center" }}>
          {minVersion === undefined
            ? t("mobile.pair.tooOld.bodyAny", { current })
            : t("mobile.pair.tooOld.body", { min: minVersion, current })}
        </Text>
        <Button
          mode="contained"
          icon={store ? "google-play" : "download"}
          testID="pair-too-old-update"
          onPress={() => {
            if (store) {
              void backend.invoke("update_install");
              return;
            }
            void backend.invoke("update_check");
            navigation.navigate("About");
          }}>
          {store ? t("mobile.pair.tooOld.store") : t("mobile.pair.tooOld.check")}
        </Button>
      </View>
    </Section>
  );
}

export function Pair() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { pairing, relay } = useUiState();
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
  // The core refuses to pair below the relay's minimum; a pairing under way finishes as it would.
  const tooOld = relayTooOld(relay);

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

  if (tooOld !== undefined && (phase === "idle" || phase === "failed")) {
    return (
      <Page testID="phone-pair">
        <TooOld minVersion={tooOld.min_version} />
      </Page>
    );
  }

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
      <RelayLink />
    </Page>
  );
}
