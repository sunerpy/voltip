import {
  type DeviceView,
  connectionKindLabel,
  connectionLabel,
  formatDate,
  platformLabel,
  relativeTime,
  relayLabel,
} from "@voltip/shared";
import {
  Button,
  Card,
  ConnectivityCheck,
  EmptyState,
  IconButton,
  LampText,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { useMobileShell } from "../app/shell";
import { PhoneMic } from "./PhoneMic";
import { RecentResults } from "./RecentResults";
import { SendText } from "./SendText";

/** Detail rows of a device card; the labels come from `mobile.devices.column.*`. */
const COLUMNS = ["device", "platform", "online", "lastSeen", "trusted", "connection"] as const;

function DeviceRow({ view, now }: { view: DeviceView; now: number }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
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
    <Card
      className={`flex flex-col gap-3 ${view.connection.state === "identity_changed" ? "border-danger" : ""}`}
      data-testid="device-card">
      <div className="flex items-center justify-between">
        <span className="text-[16px] font-semibold text-fg">{view.device.name}</span>
        <LampText
          tone={online.tone === "neutral" ? "idle" : online.tone}
          pulse={view.connection.state === "connecting"}>
          {online.text}
        </LampText>
      </div>
      <dl
        className="grid grid-cols-2 gap-x-4 gap-y-2 text-[12px]"
        aria-label={t("mobile.devices.details", { name: view.device.name })}>
        {COLUMNS.map((c) => (
          <div key={c}>
            <dt className="mono text-[10px] uppercase tracking-wider text-fg-subtle">
              {t(`mobile.devices.column.${c}`)}
            </dt>
            <dd className="text-fg">{cells[c]}</dd>
          </div>
        ))}
      </dl>
      <div className="mono text-[11px] text-fg-subtle">{view.device.fingerprint}</div>
      {view.connection.state === "identity_changed" && (
        <div className="rounded-6 bg-danger-soft px-3 py-2 text-[12px] text-danger" role="alert">
          {t("mobile.devices.identityChanged", {
            fingerprint: view.connection.presented_fingerprint,
          })}
        </div>
      )}
      <div className="flex items-center gap-2">
        {view.connection.state === "online" && (
          <Button
            size="sm"
            onClick={() =>
              void backend.invoke("send_text", {
                publicKey: view.device.public_key,
                body: t("mobile.devices.testBody"),
              })
            }>
            {t("mobile.devices.sendTest")}
          </Button>
        )}
        <IconButton
          icon="trash"
          label={t("mobile.devices.forget", { name: view.device.name })}
          tone="danger"
          bordered
          size={28}
          className="ml-auto"
          onClick={() => {
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
      </div>
    </Card>
  );
}

export function Devices() {
  const { backend } = useBackend();
  const { devices, relay, connectivity } = useUiState();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const now = useNow();
  const online = devices.filter((d) => d.connection.state === "online").length;
  return (
    <div className="flex h-full flex-col gap-3 p-4">
      <div className="flex items-center justify-between text-[12px] text-fg-muted">
        <span>{t("mobile.devices.count", { paired: devices.length, online })}</span>
        <span className="mono">
          {t("mobile.devices.relay", { state: relayLabel(relay, locale).text })}
        </span>
      </div>
      <PhoneMic desktops={devices} />
      <RecentResults />
      {devices.length === 0 ? (
        <EmptyState icon="monitor" title={t("mobile.devices.emptyTitle")}>
          {t("mobile.devices.emptyBody")}
        </EmptyState>
      ) : (
        <>
          <SendText desktops={devices} />
          {devices.map((d) => (
            <DeviceRow key={d.device.public_key} view={d} now={now} />
          ))}
          <Card className="flex flex-col gap-2">
            <span className="text-[15px] font-semibold text-fg">{t("connectivity.title")}</span>
            <ConnectivityCheck
              status={connectivity}
              onRun={() => {
                void backend.invoke("connectivity_check");
              }}
            />
          </Card>
        </>
      )}
      <Button
        variant="primary"
        className="mt-auto h-11 w-full text-[15px]"
        icon="qr"
        onClick={() => {
          shell.go("pair");
        }}>
        {t("mobile.devices.pairNew")}
      </Button>
    </div>
  );
}
