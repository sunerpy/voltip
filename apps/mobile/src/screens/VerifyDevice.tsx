import { platformLabel } from "@voltip/shared";
import {
  Button,
  Card,
  LampText,
  SafetyCodeView,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
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
    <div className="flex h-full flex-col gap-4 p-4">
      <Card className="flex flex-col gap-3">
        <div className="eyebrow">{t("mobile.verify.peer")}</div>
        <div className="text-[20px] font-semibold text-fg">
          {pairing.peer?.name ?? t("mobile.verify.computer")}
        </div>
        <div className="text-[12px] text-fg-muted">
          {t("mobile.verify.handshake", {
            platform: pairing.peer ? platformLabel(pairing.peer.platform, locale) : "—",
          })}
        </div>
      </Card>
      <Card className="flex flex-col gap-4">
        <div className="eyebrow">{t("mobile.verify.safetyCode")}</div>
        <p className="text-[13px] leading-5 text-fg-muted">{t("mobile.verify.note")}</p>
        {pairing.safety_code && <SafetyCodeView code={pairing.safety_code} size="sm" />}
        {pairing.peer_confirmed && (
          <LampText tone="ok">{t("mobile.verify.peerConfirmed")}</LampText>
        )}
        {pairing.local_confirmed && !pairing.peer_confirmed && (
          <LampText tone="accent" pulse>
            {t("mobile.verify.waiting")}
          </LampText>
        )}
      </Card>
      <div className="mt-auto flex flex-col gap-2">
        <Button
          variant="primary"
          className="h-11 w-full text-[15px]"
          icon="check"
          disabled={pairing.local_confirmed}
          onClick={() => void backend.invoke("pairing_confirm")}>
          {pairing.local_confirmed ? t("mobile.verify.waitingButton") : t("mobile.verify.confirm")}
        </Button>
        <Button
          variant="ghost"
          className="h-11 w-full text-danger"
          onClick={() => void backend.invoke("pairing_reject")}>
          {t("mobile.verify.reject")}
        </Button>
      </div>
    </div>
  );
}
