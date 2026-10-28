import { platformLabel } from "@voltip/shared";
import {
  Button,
  LampText,
  Panel,
  SafetyCodeView,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { PAGE, TOUCH } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

export function VerifyDevice() {
  const { backend } = useBackend();
  const { pairing } = useUiState();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const phase = pairing.state.state;

  if (phase === "rejected" || phase === "failed" || phase === "expired" || phase === "idle") {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 p-6 text-center">
        <LampText tone={phase === "idle" ? "idle" : "danger"}>
          {phase === "rejected"
            ? t("mobile.verify.rejected")
            : phase === "idle"
              ? t("mobile.verify.idle")
              : t("mobile.verify.failed")}
        </LampText>
        <Button
          variant="primary"
          className={TOUCH}
          onClick={() => {
            void backend.invoke("pairing_reset");
            shell.go("pair");
          }}>
          {t("mobile.verify.again")}
        </Button>
      </div>
    );
  }

  return (
    <div className={`${PAGE} min-h-full`}>
      <Panel eyebrow={t("mobile.verify.peer")} bodyClassName="flex flex-col gap-1">
        <div className="text-[18px] font-semibold break-words text-fg" data-user-text>
          {pairing.peer?.name ?? t("mobile.verify.computer")}
        </div>
        <div className="text-[12px] text-fg-muted">
          {t("mobile.verify.handshake", {
            platform: pairing.peer ? platformLabel(pairing.peer.platform, locale) : "—",
          })}
        </div>
      </Panel>
      <Panel eyebrow={t("mobile.verify.safetyCode")} bodyClassName="flex flex-col gap-4">
        {/* A scanned QR code: the computer confirms by itself (docs/pairing.md 「扫码配对只在手机上
            确认」), so this phone's confirmation finishes the pairing. */}
        <p className="text-[13px] leading-5 text-fg-muted">
          {pairing.scanned ? t("mobile.verify.scannedNote") : t("mobile.verify.note")}
        </p>
        {pairing.safety_code && <SafetyCodeView code={pairing.safety_code} size="sm" />}
        {pairing.peer_confirmed && !pairing.scanned && (
          <LampText tone="ok">{t("mobile.verify.peerConfirmed")}</LampText>
        )}
        {pairing.local_confirmed && !pairing.peer_confirmed && (
          <LampText tone="accent" pulse>
            {pairing.scanned ? t("mobile.verify.finishing") : t("mobile.verify.waiting")}
          </LampText>
        )}
      </Panel>
      <div className="mt-auto flex flex-col gap-2">
        <Button
          variant="primary"
          className={`${TOUCH} w-full`}
          icon="check"
          disabled={pairing.local_confirmed}
          onClick={() => void backend.invoke("pairing_confirm")}>
          {!pairing.local_confirmed
            ? t("mobile.verify.confirm")
            : pairing.scanned
              ? t("mobile.verify.finishingButton")
              : t("mobile.verify.waitingButton")}
        </Button>
        <Button
          variant="text-danger"
          className={`${TOUCH} w-full`}
          onClick={() => void backend.invoke("pairing_reject")}>
          {t("mobile.verify.reject")}
        </Button>
      </div>
    </div>
  );
}
