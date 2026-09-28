import {
  type RelayStatus,
  type Snapshot,
  type UiState,
  failureLabel,
  formatCode,
  formatRemaining,
  pairingStateLabel,
  platformLabel,
  relayLabel,
} from "@voltip/shared";
import {
  Button,
  Chip,
  Icon,
  IconButton,
  LampText,
  Panel,
  Progress,
  QrCode,
  SafetyCodeView,
  useI18n,
} from "@voltip/ui";
import { copyText } from "../../app/shell-context";

export interface PairingPanelProps {
  pairing: Snapshot;
  identity: UiState["identity"];
  /** The relay link, for the connection footer. */
  relay: RelayStatus;
  ttlSecs: number;
  /** `Settings.lan_discovery`: phones on the LAN see the waiting session under 「附近的电脑」. */
  lanDiscovery?: boolean;
  onStart: () => void;
  onCancel: () => void;
  /** Reset the session and immediately start a new one (regenerate the QR code). */
  onRegenerate: () => void;
  /** Back to idle after a terminal state (done). */
  onDone: () => void;
  onConfirm: () => void;
  onReject: () => void;
  /** `ok` is false when the clipboard refused or is unavailable; nothing was copied then. */
  onCopied?: (what: string, ok: boolean) => void;
}

/** PAIRING panel: QR + six-digit code + countdown, then safety-code verification. */
export function PairingPanel({
  pairing,
  identity,
  relay: relayStatus,
  ttlSecs,
  lanDiscovery = false,
  onStart,
  onCancel,
  onRegenerate,
  onDone,
  onConfirm,
  onReject,
  onCopied,
}: PairingPanelProps) {
  const { t, locale } = useI18n();
  const phase = pairing.state.state;
  const remaining = pairing.remaining_secs ?? 0;
  const label = pairingStateLabel(pairing.state, locale);
  const headerRight =
    phase === "waiting_for_peer" ? (
      <LampText tone="ok" mono>
        {t("pairing.windowOpen", { remaining: formatRemaining(remaining) })}
      </LampText>
    ) : (
      <LampText tone={label.tone === "neutral" ? "idle" : label.tone} mono>
        {label.text}
      </LampText>
    );

  const copy = async (what: string, text: string) => {
    const ok = await copyText(text);
    onCopied?.(what, ok);
  };
  const relay = relayLabel(relayStatus, locale);

  return (
    <Panel
      eyebrow={t("pairing.title")}
      right={headerRight}
      className="min-h-[292px]"
      data-testid="pairing-panel"
      data-phase={phase}>
      {phase === "idle" && (
        <div className="flex flex-col items-start gap-3 py-2">
          <p className="text-[13px] leading-5 text-fg-muted">
            {t("pairing.intro", { ttl: ttlSecs })}
          </p>
          <Button variant="primary" onClick={onStart} icon="qr">
            {t("pairing.start")}
          </Button>
        </div>
      )}

      {phase === "creating_session" && (
        <div className="flex flex-col gap-3 py-2">
          <div
            className="h-[168px] w-[168px] animate-pulse rounded-6 bg-inset hairline"
            aria-label={t("pairing.generating")}
            role="img"
          />
          <span className="mono text-[11px] text-fg-muted">{t("pairing.creating")}</span>
        </div>
      )}

      {(phase === "waiting_for_peer" || phase === "expired") && (
        <div className="flex flex-wrap gap-6">
          {/* The QR keeps its 168 px module size (scannability); everything else is fluid. */}
          <div className="flex shrink-0 flex-col gap-1.5">
            <QrCode
              value={pairing.ticket_uri ?? "voltip://pair"}
              dimmed={phase === "expired"}
              overlay={phase === "expired" ? <Chip round>{t("pairing.expired")}</Chip> : undefined}
            />
            <span className="mono text-[11px] text-fg-subtle">
              {t("pairing.qrMeta", { ttl: ttlSecs })}
            </span>
          </div>
          <div className="flex min-w-[200px] flex-1 flex-col gap-3">
            <div>
              <div className="text-[11px] text-fg-subtle">{t("pairing.code")}</div>
              <div
                className="mono text-[28px] leading-none tracking-[0.12em] text-fg"
                data-testid="pairing-code">
                {formatCode(pairing.code ?? "")}
              </div>
            </div>
            <div>
              <div className="text-[11px] text-fg-subtle">{t("pairing.fingerprint")}</div>
              <div className="mono flex items-center gap-2 text-[13px] text-fg">
                {identity?.fingerprint ?? "—"}
                {identity && (
                  <IconButton
                    icon="copy"
                    label={t("pairing.copyFingerprint")}
                    onClick={() => void copy(t("pairing.fingerprintWhat"), identity.fingerprint)}
                  />
                )}
              </div>
              <div className="mt-0.5 text-[11px] text-fg-muted">{t("pairing.scanNote")}</div>
              {lanDiscovery && phase === "waiting_for_peer" && (
                <div className="mt-0.5 text-[11px] text-fg-muted" data-testid="pairing-lan-note">
                  {t("pairing.lanNote")}
                </div>
              )}
            </div>
            <div>
              <div className="text-[11px] text-fg-subtle">{t("pairing.validity")}</div>
              <div className="flex items-center gap-3">
                <span
                  className="mono text-[13px] whitespace-nowrap text-fg"
                  data-testid="pairing-countdown">
                  {formatRemaining(remaining)} / {formatRemaining(ttlSecs)}
                </span>
                <div className="min-w-0 max-w-[240px] flex-1">
                  <Progress value={remaining / ttlSecs} />
                </div>
              </div>
            </div>
            <div className="flex items-center gap-1.5 text-[12px] text-fg-muted">
              <Icon name="alert" size={13} className="text-danger" />
              {t("pairing.warning")}
            </div>
            <div className="mt-auto flex flex-wrap items-center gap-2">
              <Button variant="primary" onClick={onRegenerate} icon="refresh">
                {phase === "expired" ? t("pairing.regenerate") : t("pairing.regenerateQr")}
              </Button>
              {phase === "waiting_for_peer" && (
                <>
                  <Button
                    onClick={() => void copy(t("pairing.linkWhat"), pairing.ticket_uri ?? "")}>
                    {t("pairing.copyLink")}
                  </Button>
                  <Button variant="ghost" onClick={onCancel}>
                    {t("pairing.cancel")}
                  </Button>
                </>
              )}
              {phase === "expired" && (
                <span className="text-[12px] text-danger">
                  {t("pairing.expiredNote", { ttl: ttlSecs })}
                </span>
              )}
            </div>
          </div>
        </div>
      )}

      {phase === "key_exchange" && (
        <div className="flex flex-col gap-3 py-2">
          <div className="text-[13px] text-fg">
            {t("pairing.joined", { name: pairing.peer?.name ?? t("pairing.phone") })}
          </div>
          <div className="max-w-[240px]">
            <Progress indeterminate />
          </div>
          <span className="mono text-[11px] text-fg-subtle">Noise_XX_25519_ChaChaPoly_SHA256</span>
        </div>
      )}

      {phase === "awaiting_verification" && pairing.safety_code && (
        <div className="flex flex-col gap-4 py-1">
          <div className="text-[13px] text-fg-muted">
            {pairing.peer ? (
              <>
                <span className="font-medium text-fg">{pairing.peer.name}</span>
                {t("pairing.peerDone", {
                  name: "",
                  platform: platformLabel(pairing.peer.platform, locale),
                })}
              </>
            ) : (
              t("pairing.peerDoneAnon")
            )}{" "}
            {t("pairing.verifyNote")}
          </div>
          <SafetyCodeView code={pairing.safety_code} />
          <div className="flex items-center gap-2">
            <Button
              variant="primary"
              onClick={onConfirm}
              disabled={pairing.local_confirmed}
              icon="check">
              {pairing.local_confirmed ? t("pairing.waitingPeer") : t("pairing.confirm")}
            </Button>
            <Button variant="ghost" onClick={onReject} className="text-danger">
              {t("pairing.reject")}
            </Button>
            {pairing.peer_confirmed && <LampText tone="ok">{t("pairing.peerConfirmed")}</LampText>}
          </div>
        </div>
      )}

      {phase === "trusted" && (
        <div className="flex flex-col items-start gap-3 py-2">
          <LampText tone="ok">
            {t("pairing.trusted", { name: pairing.peer?.name ?? t("pairing.newDevice") })}
          </LampText>
          <p className="text-[13px] text-fg-muted">{t("pairing.trustedNote")}</p>
          <div className="flex gap-2">
            <Button variant="primary" onClick={onDone}>
              {t("pairing.done")}
            </Button>
            <Button onClick={onStart}>{t("pairing.another")}</Button>
          </div>
        </div>
      )}

      {(phase === "rejected" || phase === "failed") && (
        <div className="flex flex-col items-start gap-3 py-2">
          <LampText tone="danger">
            {phase === "rejected"
              ? t("pairing.rejected")
              : t("pairing.failed", {
                  reason: failureLabel(
                    pairing.state.state === "failed" ? pairing.state.reason : { kind: "protocol" },
                    locale,
                  ),
                })}
          </LampText>
          <Button variant="primary" onClick={onStart} icon="refresh">
            {t("pairing.restart")}
          </Button>
        </div>
      )}

      <div
        className="mt-4 flex flex-col gap-1 border-t border-border pt-3"
        data-testid="pairing-connect">
        <div className="flex items-center justify-between gap-2">
          <span className="eyebrow">{t("pairing.connectTitle")}</span>
          <LampText tone={relay.tone === "neutral" ? "idle" : relay.tone} mono>
            {t("pairing.relay", { state: relay.text })}
          </LampText>
        </div>
        <p className="text-[11px] leading-4 text-fg-muted">{t("pairing.connect")}</p>
      </div>
    </Panel>
  );
}
