import {
  type AddressCheck,
  type ConnectivityStatus,
  type PeerCheck,
  type ProbeResult,
  type Locale,
  type TFunction,
  connectionKindLabel,
  formatDateTime,
} from "@voltip/shared";
import { useI18n } from "../i18n/I18nProvider";
import { Button } from "./Button";
import { LampText } from "./LampText";
import type { LampTone } from "./Lamp";

export interface ConnectivityCheckProps {
  status: ConnectivityStatus;
  /** Start a check (`connectivity_check`). */
  onRun: () => void;
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

function probeTone(result: ProbeResult): LampTone {
  return result.result === "ok" ? "ok" : "danger";
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

/** The connectivity self-check (docs/pairing.md): a button and, once a check finished, what this
 *  device could reach — its LAN host, the relay, and each paired device's channel and LAN
 *  addresses. Every line is a measurement the core made when asked. */
export function ConnectivityCheck({ status, onRun }: ConnectivityCheckProps) {
  const { t, locale } = useI18n();
  const report = status.report;
  const label = status.running
    ? t("connectivity.running")
    : report === undefined
      ? t("connectivity.run")
      : t("connectivity.runAgain");
  return (
    <div className="flex flex-col gap-3" data-testid="connectivity" data-running={status.running}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="min-w-0 flex-1 text-[12px] leading-5 text-fg-muted">
          {t("connectivity.intro")}
        </p>
        <Button size="sm" icon="refresh" disabled={status.running} onClick={onRun}>
          {label}
        </Button>
      </div>
      {report !== undefined && (
        <ul className="flex flex-col gap-2 text-[12px]" aria-label={t("connectivity.results")}>
          <li data-testid="connectivity-lan">
            <LampText tone={report.lan.listening ? "ok" : "idle"} size="sm">
              {report.lan.listening
                ? t("connectivity.lanListening", { addresses: report.lan.addresses.join(" / ") })
                : t("connectivity.lanOff")}
            </LampText>
          </li>
          <li data-testid="connectivity-relay">
            {report.relay.result === undefined ? (
              <LampText tone="idle" size="sm">
                {t("connectivity.relayNone")}
              </LampText>
            ) : (
              <LampText tone={probeTone(report.relay.result)} size="sm">
                {t("connectivity.relay", { result: probeText(report.relay.result, t) })}
              </LampText>
            )}
          </li>
          {report.peers.map((peer) => (
            <li
              key={peer.public_key}
              className="flex flex-col gap-1 border-t border-border pt-2"
              data-testid="connectivity-peer">
              <LampText
                tone={peer.via === undefined ? "idle" : peer.rtt_ms === undefined ? "warn" : "ok"}
                size="sm">
                {peerText(peer, t, locale)}
              </LampText>
              {peer.addresses.length === 0 ? (
                <span className="pl-4 text-fg-subtle">{t("connectivity.noAddress")}</span>
              ) : (
                peer.addresses.map((a) => (
                  <span
                    key={a.address}
                    className={`mono pl-4 text-[11px] ${a.result.result === "ok" ? "text-fg-muted" : "text-danger"}`}>
                    {t("connectivity.address", {
                      address: a.address,
                      result: probeText(a.result, t),
                    })}
                  </span>
                ))
              )}
              {blockedOnSubnet(peer.addresses) && (
                <span className="pl-4 text-[11px] leading-4 text-fg-subtle">
                  {t("connectivity.blocked")}
                </span>
              )}
            </li>
          ))}
          <li className="text-[11px] text-fg-subtle">
            {t("connectivity.checkedAt", {
              at: formatDateTime(locale, report.checked_at, { timeStyle: "medium" }),
            })}
          </li>
        </ul>
      )}
    </div>
  );
}
