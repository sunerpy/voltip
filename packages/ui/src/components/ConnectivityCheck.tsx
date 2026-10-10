import {
  type ConnectivityStatus,
  type PeerCheck,
  type ProbeResult,
  type TFunction,
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

/** The line for one paired device: online or not, and the encrypted round trip. */
export function peerText(peer: PeerCheck, t: TFunction): string {
  if (!peer.online) return t("connectivity.peerOffline", { name: peer.name });
  return peer.rtt_ms === undefined
    ? t("connectivity.peerNoAnswer", { name: peer.name })
    : t("connectivity.peerOnline", { name: peer.name, ms: peer.rtt_ms });
}

/** The connectivity self-check (docs/pairing.md): a button and, once a check finished, what this
 *  device could reach: the relay, and each paired device's channel. Every line is a measurement
 *  the core made when asked. */
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
                tone={!peer.online ? "idle" : peer.rtt_ms === undefined ? "warn" : "ok"}
                size="sm">
                {peerText(peer, t)}
              </LampText>
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
