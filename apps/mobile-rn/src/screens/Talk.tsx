// 说话, the first tab (apps/mobile's Welcome and Devices): until a computer is paired, talk on the
// phone right away and what pairing adds; after, the paired computers too, the keyboard and the
// connection check. A first Android back here says a second one within two seconds leaves.
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
import { EmptyState, Lede, Page, Section, StateLine, useAppTheme } from "../ui/kit";

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
    <Page testID="phone-welcome">
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
    <Page testID="phone-devices">
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 12,
          paddingHorizontal: 4,
        }}>
        <StateLine tone={online > 0 ? "ok" : "idle"} small>
          {t("mobile.devices.count", { paired: devices.length, online })}
        </StateLine>
        <Text variant="bodySmall" numberOfLines={1} style={{ flexShrink: 1 }}>
          {t("mobile.devices.relay", { state: relayLabel(relay, locale).text })}
        </Text>
      </View>
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
