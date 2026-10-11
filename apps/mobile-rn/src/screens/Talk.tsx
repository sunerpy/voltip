// 说话, the first tab (apps/mobile's Welcome and Devices; Penpot 「澄空 Glass · Voltip」 board 03):
// its title over the pairing and relay state, the way to this phone's own page, the talk card and
// the recent results. Until a computer is paired, what pairing adds; after, the paired computers
// too, the keyboard and the connection check. A first Android back here says a second one within
// two seconds leaves.
import { useFocusEffect } from "@react-navigation/native";
import { relayLabel } from "@voltip/shared";
import { useCallback, useRef } from "react";
import { BackHandler, View } from "react-native";
import { Icon, Text } from "react-native-paper";

import { useUiState } from "../backend/BackendProvider";
import { useI18n, useT } from "../backend/i18n";
import { ConnectivityCheck, DeviceCard, RefineNotice } from "../features/Devices";
import { PhoneMic } from "../features/PhoneMic";
import { RecentResults } from "../features/RecentResults";
import { SendText } from "../features/SendText";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  EmptyState,
  Lamp,
  Lede,
  Page,
  RoundButton,
  Section,
  TabTitle,
  type Tone,
  useAppTheme,
} from "../ui/kit";

/** How long the second back at 说话 has to leave the app: the usual two seconds on Android. */
export const EXIT_WINDOW_MS = 2000;

/** The first back at 说话 says so; a second one within the window goes to the system, which
 *  leaves the app as it always does. */
function useBackToLeave() {
  const shell = useShell();
  const t = useT();
  const last = useRef(0);
  useFocusEffect(
    useCallback(() => {
      const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
        const now = Date.now();
        if (now - last.current < EXIT_WINDOW_MS) return false;
        last.current = now;
        shell.toast(t("mobile.backToLeave"));
        return true;
      });
      return () => {
        subscription.remove();
      };
    }, [shell, t]),
  );
}

/** 说话 over its state line, and the round button to this phone's own page (board 03). */
function TalkTitle({ tone, line }: { tone: Tone; line: string }) {
  const theme = useAppTheme();
  const t = useT();
  const navigation = useRootNavigation();
  return (
    <TabTitle
      title={t("mobile.tab.talk")}
      subtitle={
        <View testID="talk-status" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
          <Lamp tone={tone} />
          <Text
            numberOfLines={1}
            style={{
              flexShrink: 1,
              fontSize: 13,
              lineHeight: 18,
              color: theme.colors.onSurfaceVariant,
            }}>
            {line}
          </Text>
        </View>
      }
      right={
        <RoundButton
          icon="cellphone"
          label={t("mobile.thisDevice")}
          testID="open-this-device"
          onPress={() => {
            navigation.navigate("ThisDevice");
          }}
        />
      }
    />
  );
}

/** What pairing a computer brings, each with its mark. */
const POINTS = [
  { id: "e2ee", icon: "lock-outline" },
  { id: "safety", icon: "shield-check-outline" },
  { id: "identity", icon: "alert-circle-outline" },
] as const;

function Welcome() {
  const theme = useAppTheme();
  const t = useT();
  const navigation = useRootNavigation();
  return (
    <Page
      testID="phone-welcome"
      title={<TalkTitle tone="idle" line={t("mobile.welcome.status")} />}>
      <Lede>{t("mobile.welcome.intro")}</Lede>
      <PhoneMic desktops={[]} />
      <RefineNotice />
      <RecentResults />
      <Section title={t("mobile.welcome.pairing")} padded>
        <Text variant="bodyMedium" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("mobile.welcome.pairingBody")}
        </Text>
        {POINTS.map((point) => (
          <View key={point.id} style={{ flexDirection: "row", gap: 12 }}>
            <Icon source={point.icon} size={20} color={theme.colors.onSurfaceVariant} />
            <View style={{ flex: 1, gap: 2 }}>
              <Text variant="titleSmall">{t(`mobile.welcome.${point.id}`)}</Text>
              <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
                {t(`mobile.welcome.${point.id}Body`)}
              </Text>
            </View>
          </View>
        ))}
        <Button
          mode="contained"
          icon="monitor"
          testID="welcome-start"
          onPress={() => {
            navigation.navigate("ThisDevice");
          }}>
          {t("mobile.welcome.start")}
        </Button>
      </Section>
    </Page>
  );
}

function Paired() {
  const { devices, relay, connectivity } = useUiState();
  const { t, locale } = useI18n();
  const navigation = useRootNavigation();
  const online = devices.filter((d) => d.connection.state === "online").length;
  return (
    <Page
      testID="phone-devices"
      title={
        <TalkTitle
          tone={online > 0 ? "ok" : "idle"}
          line={t("mobile.devices.status", {
            paired: devices.length,
            online,
            state: relayLabel(relay, locale).text,
          })}
        />
      }>
      <PhoneMic desktops={devices} />
      <RefineNotice />
      <RecentResults />
      {devices.length === 0 ? (
        <Section>
          <EmptyState icon="monitor" title={t("mobile.devices.emptyTitle")}>
            {t("mobile.devices.emptyBody")}
          </EmptyState>
        </Section>
      ) : (
        <>
          <SendText desktops={devices} />
          {devices.map((d) => (
            <DeviceCard key={d.device.public_key} view={d} />
          ))}
          <ConnectivityCheck status={connectivity} />
        </>
      )}
      <Button
        mode="contained"
        icon="qrcode-scan"
        testID="pair-new"
        onPress={() => {
          navigation.navigate("Pair");
        }}>
        {t("mobile.devices.pairNew")}
      </Button>
    </Page>
  );
}

export function Talk() {
  const { devices } = useUiState();
  useBackToLeave();
  return devices.length > 0 ? <Paired /> : <Welcome />;
}
