import { type NearbyDevice, failureLabel, platformLabel } from "@voltip/shared";
import {
  Button,
  Card,
  CodeInput,
  Input,
  LampText,
  Panel,
  Progress,
  Segmented,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { Lede, PAGE, TOUCH } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

type Method = "scan" | "code";

export function isPairingLink(value: string): boolean {
  return value.trim().startsWith("voltip://pair?");
}

/** The computers 「附近的电脑」 lists: not phones, not already paired; waiting ones first. */
export function nearbyComputers(nearby: readonly NearbyDevice[]): NearbyDevice[] {
  const computers = nearby.filter(
    (d) => d.platform !== "android" && d.platform !== "ios" && !d.trusted,
  );
  computers.sort((a, b) => Number(b.pairing) - Number(a.pairing) || a.name.localeCompare(b.name));
  return computers;
}

/** 附近的电脑 (docs/pairing.md 「局域网发现」): the computers the LAN browse sees; one that waits
 *  for a pairing is joined with a tap, and the safety code is compared as after a scan. */
function Nearby({ busy }: { busy: boolean }) {
  const { backend } = useBackend();
  const { nearby, settings } = useUiState();
  const { t, locale } = useI18n();
  const computers = nearbyComputers(nearby);
  return (
    <Panel eyebrow={t("mobile.pair.nearby.title")} data-testid="nearby">
      {!settings.lan_discovery ? (
        <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.pair.nearby.off")}</p>
      ) : computers.length === 0 ? (
        <div className="flex flex-col gap-1.5">
          <LampText tone="idle" pulse>
            {t("mobile.pair.nearby.searching")}
          </LampText>
          <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.pair.nearby.empty")}</p>
        </div>
      ) : (
        <ul
          className="-my-2 flex flex-col divide-y divide-border"
          aria-label={t("mobile.pair.nearby.title")}>
          {computers.map((d) => (
            <li
              key={d.fingerprint}
              className="flex min-h-14 items-center justify-between gap-3 py-2"
              data-testid="nearby-computer"
              data-pairing={d.pairing}>
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="truncate text-[14px] font-medium text-fg" data-user-text>
                  {d.name}
                </span>
                <span className="text-[12px] text-fg-muted">
                  {platformLabel(d.platform, locale)}
                  {!d.pairing && ` · ${t("mobile.pair.nearby.idle")}`}
                </span>
              </div>
              {d.pairing && (
                <Button
                  variant="primary"
                  className={TOUCH}
                  aria-label={t("mobile.pair.nearby.joinLabel", { name: d.name })}
                  disabled={busy}
                  onClick={() => {
                    void backend.invoke("pairing_join_nearby", { fingerprint: d.fingerprint });
                  }}>
                  {t("mobile.pair.nearby.join")}
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </Panel>
  );
}

export function PairDevice() {
  const { backend } = useBackend();
  const { pairing } = useUiState();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const [method, setMethod] = useState<Method>("scan");
  const [code, setCode] = useState("");
  const [link, setLink] = useState("");
  const [scanning, setScanning] = useState(false);
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

  const joinCode = (digits: string) => {
    void backend.invoke("pairing_join_code", { code: digits });
  };
  const joinLink = (uri: string) => {
    void backend.invoke("pairing_join_ticket", { uri: uri.trim() });
  };
  const scan = async () => {
    if (!shell.scanner) return;
    setScanning(true);
    try {
      const content = await shell.scanner.scan();
      if (content === undefined) shell.toast(t("mobile.pair.scanCancelled"), "danger");
      else joinLink(content);
    } finally {
      setScanning(false);
    }
  };
  const retry = () => {
    void backend.invoke("pairing_reset");
    setCode("");
  };

  return (
    <div className={PAGE}>
      <Lede>{t("mobile.pair.intro")}</Lede>
      <Nearby busy={busy} />
      <Segmented
        label={t("mobile.pair.method")}
        value={method}
        onChange={setMethod}
        className="h-11 w-full [&>button]:flex-1"
        options={[
          { value: "scan", label: t("mobile.pair.scan") },
          { value: "code", label: t("mobile.pair.code") },
        ]}
      />

      {method === "scan" && (
        <Card className="flex flex-col gap-3">
          {!shell.scannerReady ? (
            <LampText tone="idle" pulse>
              {t("mobile.pair.checkingCamera")}
            </LampText>
          ) : shell.scanner ? (
            <>
              <div className="text-[14px] font-medium text-fg">{t("mobile.pair.aim")}</div>
              <Button
                variant="primary"
                className={`${TOUCH} w-full`}
                icon="qr"
                loading={scanning}
                disabled={busy}
                onClick={() => void scan()}>
                {t("mobile.pair.openCamera")}
              </Button>
            </>
          ) : (
            <>
              <div className="flex flex-col gap-1">
                <div className="text-[14px] font-medium text-fg">{t("mobile.pair.noCamera")}</div>
                <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.pair.pasteLink")}</p>
              </div>
              <Input
                label={t("mobile.pair.link")}
                mono
                size="lg"
                value={link}
                placeholder="voltip://pair?v=1&s=…&t=…"
                onChange={(e) => {
                  setLink(e.target.value);
                }}
                error={
                  link.length > 0 && !isPairingLink(link) ? t("mobile.pair.notLink") : undefined
                }
              />
              <Button
                variant="primary"
                className={`${TOUCH} w-full`}
                disabled={!isPairingLink(link) || busy}
                onClick={() => {
                  joinLink(link);
                }}>
                {t("mobile.pair.join")}
              </Button>
            </>
          )}
        </Card>
      )}

      {method === "code" && (
        <Card className="flex flex-col gap-4">
          <div className="text-center text-[14px] font-medium text-fg">
            {t("mobile.pair.enterCode")}
          </div>
          <CodeInput
            value={code}
            onChange={setCode}
            onComplete={joinCode}
            disabled={busy}
            autoFocus
            error={failed}
          />
          <Button
            variant="primary"
            className={`${TOUCH} w-full`}
            disabled={code.length !== 6 || busy}
            onClick={() => {
              joinCode(code);
            }}>
            {t("mobile.pair.join")}
          </Button>
        </Card>
      )}

      {busy && (
        <div className="flex flex-col gap-2 px-1" role="status">
          <LampText tone="accent" pulse>
            {phase === "creating_session" ? t("mobile.pair.joining") : t("mobile.pair.negotiating")}
          </LampText>
          <Progress indeterminate />
        </div>
      )}
      {failed && method === "scan" && (
        <div
          className="flex items-center justify-between gap-3 rounded-10 bg-danger-soft py-1 pr-1 pl-4 text-[13px] text-danger"
          role="alert">
          <span>{failed}</span>
          <Button className={TOUCH} onClick={retry}>
            {t("mobile.pair.retry")}
          </Button>
        </div>
      )}
      {failed && method === "code" && (
        <Button variant="ghost" className={`${TOUCH} self-center`} onClick={retry}>
          {t("mobile.pair.clearRetry")}
        </Button>
      )}
    </div>
  );
}
