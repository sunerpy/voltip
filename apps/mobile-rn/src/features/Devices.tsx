// The paired computers, the connection self-check and the built-in polish service's notice, as
// apps/mobile's 说话 screen shows them once a computer is paired.
import {
  type AddressCheck,
  type ConnectivityStatus,
  type DeviceView,
  type Locale,
  type PeerCheck,
  type ProbeResult,
  type TFunction,
  connectionKindLabel,
  connectionLabel,
  formatDate,
  formatDateTime,
  mirrorStateText,
  platformLabel,
  relativeTime,
} from "@voltip/shared";
import { View } from "react-native";
import { IconButton, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n, useT } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { Mono, Notice, Section, StateLine, type Tone, useAppTheme } from "../ui/kit";

/** Detail cells of a device card; the labels come from `mobile.devices.column.*`. */
const COLUMNS = ["device", "platform", "online", "lastSeen", "trusted", "connection"] as const;

function lampTone(tone: string): Tone {
  return tone === "ok" || tone === "accent" || tone === "danger" || tone === "warning"
    ? tone
    : "idle";
}

/** One paired computer: its name and state, the desktop table's columns as label-over-value
 *  readouts, how its copy on the phone stands, and the actions. */
export function DeviceCard({ view }: { view: DeviceView }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const now = useNow();
  const copy = useUiState().mirrors.find((m) => m.desktop === view.device.public_key);
  const online = connectionLabel(view.connection, locale);
  const cells: Record<(typeof COLUMNS)[number], string> = {
    device: view.device.name,
    platform: platformLabel(view.device.platform, locale),
    online: online.text,
    lastSeen: relativeTime(view.device.last_seen, now, locale),
    trusted: formatDate(view.device.trusted_at),
    connection: connectionKindLabel(
      view.connection.state === "online" ? view.connection.via : view.device.last_connection,
      locale,
    ),
  };
  return (
    <Section padded testID="device-card">
      <View style={{ flexDirection: "row", alignItems: "flex-start", gap: 12 }}>
        <Text variant="titleMedium" style={{ flex: 1 }}>
          {view.device.name}
        </Text>
        <StateLine tone={lampTone(online.tone)} pulse={view.connection.state === "connecting"}>
          {online.text}
        </StateLine>
      </View>
      <View
        style={{ flexDirection: "row", flexWrap: "wrap", rowGap: 12 }}
        accessibilityLabel={t("mobile.devices.details", { name: view.device.name })}>
        {COLUMNS.map((c) => (
          <View key={c} style={{ width: "50%", paddingRight: 8, gap: 2 }}>
            <Text variant="labelSmall" style={{ color: theme.voltip.subtle }}>
              {t(`mobile.devices.column.${c}`)}
            </Text>
            <Text variant="bodyMedium">{cells[c]}</Text>
          </View>
        ))}
      </View>
      {copy !== undefined && (
        <Text
          variant="bodySmall"
          testID="device-sync"
          style={{ color: theme.colors.onSurfaceVariant }}>
          {mirrorStateText(copy, now, locale)}
        </Text>
      )}
      <Mono selectable>{view.device.fingerprint}</Mono>
      {view.connection.state === "identity_changed" && (
        <Notice tone="danger">
          {t("mobile.devices.identityChanged", {
            fingerprint: view.connection.presented_fingerprint,
          })}
        </Notice>
      )}
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        {view.connection.state === "online" && (
          <Button
            mode="outlined"
            onPress={() =>
              void backend.invoke("send_text", {
                publicKey: view.device.public_key,
                body: t("mobile.devices.testBody"),
              })
            }>
            {t("mobile.devices.sendTest")}
          </Button>
        )}
        <View style={{ flex: 1 }} />
        <IconButton
          icon="trash-can-outline"
          mode="outlined"
          iconColor={theme.colors.error}
          accessibilityLabel={t("mobile.devices.forget", { name: view.device.name })}
          onPress={() => {
            shell.confirm({
              title: t("mobile.devices.forgetTitle", { name: view.device.name }),
              body: t("mobile.devices.forgetBody"),
              confirmLabel: t("mobile.devices.forgetConfirm"),
              onConfirm: () => {
                void backend.invoke("device_forget", { publicKey: view.device.public_key });
                shell.toast(t("mobile.devices.forgotten", { name: view.device.name }));
              },
            });
          }}
        />
      </View>
    </Section>
  );
}

/** `可连接 · 12 ms` / `没有响应` / `拒绝连接` / `连接失败 · …`. */
export function probeText(result: ProbeResult, t: TFunction): string {
  switch (result.result) {
    case "ok":
      return t("connectivity.result.ok", { ms: result.ms });
    case "timeout":
      return t("connectivity.result.timeout");
    case "refused":
      return t("connectivity.result.refused");
    case "failed":
      return result.reason.length > 0
        ? t("connectivity.result.failed", { reason: result.reason })
        : t("connectivity.result.failedBare");
  }
}

/** The line for one paired device: how its channel runs and the encrypted round trip. */
export function peerText(peer: PeerCheck, t: TFunction, locale: Locale): string {
  if (peer.via === undefined) return t("connectivity.peerOffline", { name: peer.name });
  const via = connectionKindLabel(peer.via, locale);
  return peer.rtt_ms === undefined
    ? t("connectivity.peerNoAnswer", { name: peer.name, via })
    : t("connectivity.peerOnline", { name: peer.name, via, ms: peer.rtt_ms });
}

/** A failed address on this device's own subnet: the port is blocked, not the route. */
export function blockedOnSubnet(addresses: readonly AddressCheck[]): boolean {
  return addresses.some((a) => a.same_subnet && a.result.result !== "ok");
}

/** The connectivity self-check (docs/pairing.md): what this phone could reach when asked. */
export function ConnectivityCheck({ status }: { status: ConnectivityStatus }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const report = status.report;
  const label = status.running
    ? t("connectivity.running")
    : report === undefined
      ? t("connectivity.run")
      : t("connectivity.runAgain");
  return (
    <Section title={t("connectivity.title")} padded testID="connectivity">
      <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
        {t("connectivity.intro")}
      </Text>
      <Button
        mode="outlined"
        icon="refresh"
        style={{ alignSelf: "flex-start" }}
        loading={status.running}
        disabled={status.running}
        onPress={() => void backend.invoke("connectivity_check")}>
        {label}
      </Button>
      {report !== undefined && (
        <View style={{ gap: 8 }} accessibilityLabel={t("connectivity.results")}>
          <StateLine tone={report.lan.listening ? "ok" : "idle"} small>
            {report.lan.listening
              ? t("connectivity.lanListening", { addresses: report.lan.addresses.join(" / ") })
              : t("connectivity.lanOff")}
          </StateLine>
          {report.relay.result === undefined ? (
            <StateLine tone="idle" small>
              {t("connectivity.relayNone")}
            </StateLine>
          ) : (
            <StateLine tone={report.relay.result.result === "ok" ? "ok" : "danger"} small>
              {t("connectivity.relay", { result: probeText(report.relay.result, t) })}
            </StateLine>
          )}
          {report.peers.map((peer) => (
            <View key={peer.public_key} style={{ gap: 4 }}>
              <StateLine
                tone={
                  peer.via === undefined ? "idle" : peer.rtt_ms === undefined ? "warning" : "ok"
                }
                small>
                {peerText(peer, t, locale)}
              </StateLine>
              {peer.addresses.length === 0 ? (
                <Text variant="bodySmall" style={{ paddingLeft: 14, color: theme.voltip.subtle }}>
                  {t("connectivity.noAddress")}
                </Text>
              ) : (
                peer.addresses.map((a) => (
                  <Mono
                    key={a.address}
                    style={{
                      paddingLeft: 14,
                      color:
                        a.result.result === "ok"
                          ? theme.colors.onSurfaceVariant
                          : theme.colors.error,
                    }}>
                    {t("connectivity.address", {
                      address: a.address,
                      result: probeText(a.result, t),
                    })}
                  </Mono>
                ))
              )}
              {blockedOnSubnet(peer.addresses) && (
                <Text variant="bodySmall" style={{ paddingLeft: 14, color: theme.voltip.subtle }}>
                  {t("connectivity.blocked")}
                </Text>
              )}
            </View>
          ))}
          <Text variant="bodySmall" style={{ color: theme.voltip.subtle }}>
            {t("connectivity.checkedAt", {
              at: formatDateTime(locale, report.checked_at, { timeStyle: "medium" }),
            })}
          </Text>
        </View>
      )}
    </Section>
  );
}

/** The built-in AI polish service turned a take down for want of capacity (docs/dictation.md
 *  §3.6): what happened, the page where a provider of one's own is set up, and a close button that
 *  asks the core to drop the notice for a day. */
export function RefineNotice() {
  const { refine_notice: notice } = useUiState();
  const { backend } = useBackend();
  const navigation = useRootNavigation();
  const t = useT();
  if (notice === undefined) return null;
  return (
    <Notice
      tone="warning"
      icon="alert-outline"
      testID="refine-notice"
      action={
        <View style={{ flexDirection: "row", gap: 8, flexWrap: "wrap" }}>
          <Button
            mode="outlined"
            compact
            onPress={() => {
              navigation.navigate("Ai");
            }}>
            {t("refineNotice.open")}
          </Button>
          <Button compact onPress={() => void backend.invoke("refine_notice_close")}>
            {t("ui.banner.close")}
          </Button>
        </View>
      }>
      <Text variant="titleSmall">
        {notice.failure === "quota"
          ? t("refineNotice.title.quota")
          : t("refineNotice.title.rate_limited")}
      </Text>
      <Text variant="bodyMedium">{t("refineNotice.body")}</Text>
    </Notice>
  );
}
