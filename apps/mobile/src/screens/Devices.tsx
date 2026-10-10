import {
  type DeviceView,
  connectionLabel,
  formatDate,
  mirrorStateText,
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
  Panel,
  RefineNotice,
  cx,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { PAGE, TOUCH, TOUCH_ICON } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";
import { PhoneMic } from "./PhoneMic";
import { RecentResults } from "./RecentResults";
import { SendText } from "./SendText";

/** Detail rows of a device card; the labels come from `mobile.devices.column.*`. */
const COLUMNS = ["device", "platform", "online", "lastSeen", "trusted"] as const;

/** One paired computer, as a row of the desktop's device table reads on a phone: its name and
 *  state, then the table's columns as label-over-value readouts. */
function DeviceRow({ view, now }: { view: DeviceView; now: number }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  // docs/dictation.md §20.8: how this computer's history and settings stand on the phone.
  const copy = useUiState().mirrors.find((m) => m.desktop === view.device.public_key);
  const online = connectionLabel(view.connection, locale);
  const cells: Record<(typeof COLUMNS)[number], string> = {
    device: view.device.name,
    platform: platformLabel(view.device.platform, locale),
    online: online.text,
    lastSeen: relativeTime(view.device.last_seen, now, locale),
    trusted: formatDate(view.device.trusted_at),
  };
  return (
    <Card
      className={cx(
        "flex flex-col gap-4",
        view.connection.state === "identity_changed" && "border-danger",
      )}
      data-testid="device-card">
      <div className="flex items-start justify-between gap-3">
        <span className="min-w-0 text-[15px] font-semibold break-words text-fg" data-user-text>
          {view.device.name}
        </span>
        <LampText
          tone={online.tone === "neutral" ? "idle" : online.tone}
          pulse={view.connection.state === "connecting"}
          className="mt-0.5">
          {online.text}
        </LampText>
      </div>
      <dl
        className="grid grid-cols-2 gap-x-4 gap-y-3"
        aria-label={t("mobile.devices.details", { name: view.device.name })}>
        {COLUMNS.map((c) => (
          <div key={c} className="flex min-w-0 flex-col gap-0.5">
            <dt className="text-[11px] text-fg-subtle">{t(`mobile.devices.column.${c}`)}</dt>
            <dd className={cx("text-[13px] break-words text-fg", c === "trusted" && "mono")}>
              {cells[c]}
            </dd>
          </div>
        ))}
      </dl>
      {copy !== undefined && (
        <p
          className="text-[12px] leading-5 text-fg-muted"
          data-testid="device-sync"
          data-state={copy.state}>
          {mirrorStateText(copy, now, locale)}
        </p>
      )}
      <div className="mono text-[11px] text-fg-subtle select-text">{view.device.fingerprint}</div>
      {view.connection.state === "identity_changed" && (
        <div
          className="rounded-6 bg-danger-soft px-3 py-2 text-[12px] leading-5 text-danger"
          role="alert">
          {t("mobile.devices.identityChanged", {
            fingerprint: view.connection.presented_fingerprint,
          })}
        </div>
      )}
      <div className="flex items-center gap-2 border-t border-border pt-3">
        {view.connection.state === "online" && (
          <Button
            className={TOUCH}
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
          className={cx(TOUCH_ICON, "ml-auto")}
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

/** 说话 once a computer is paired: the talk card, the phone's own results, the keyboard, then the
 *  paired computers, the connection check and the way to pair another. */
export function Devices() {
  const { backend } = useBackend();
  const { devices, relay, connectivity } = useUiState();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const now = useNow();
  const online = devices.filter((d) => d.connection.state === "online").length;
  return (
    <div className={cx(PAGE, "min-h-full")}>
      <div className="flex items-center justify-between gap-3 px-1 text-[12px] text-fg-muted">
        <LampText tone={online > 0 ? "ok" : "idle"}>
          {t("mobile.devices.count", { paired: devices.length, online })}
        </LampText>
        <span className="mono truncate">
          {t("mobile.devices.relay", { state: relayLabel(relay, locale).text })}
        </span>
      </div>
      <PhoneMic desktops={devices} />
      <RefineNotice
        onOpen={() => {
          shell.go("ai");
        }}
        buttonClassName={cx(TOUCH, "mt-2")}
      />
      <RecentResults />
      {devices.length === 0 ? (
        <Card padding="none">
          <EmptyState icon="monitor" title={t("mobile.devices.emptyTitle")}>
            {t("mobile.devices.emptyBody")}
          </EmptyState>
        </Card>
      ) : (
        <>
          <SendText desktops={devices} />
          {devices.map((d) => (
            <DeviceRow key={d.device.public_key} view={d} now={now} />
          ))}
          <Panel eyebrow={t("connectivity.title")}>
            <ConnectivityCheck
              status={connectivity}
              onRun={() => {
                void backend.invoke("connectivity_check");
              }}
            />
          </Panel>
        </>
      )}
      <Button
        variant="primary"
        className={cx(TOUCH, "mt-auto w-full")}
        icon="qr"
        onClick={() => {
          shell.go("pair");
        }}>
        {t("mobile.devices.pairNew")}
      </Button>
    </div>
  );
}
