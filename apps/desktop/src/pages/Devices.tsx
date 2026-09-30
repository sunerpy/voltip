import {
  type ArgsOf,
  type DeviceView,
  type DictationStatus,
  type MutationCommand,
  type TFunction,
  connectionKindLabel,
  connectionLabel,
  formatDate,
  formatElapsed,
  platformLabel,
  relativeTime,
  shortFingerprint,
  takeFailureText,
} from "@voltip/shared";
import {
  Banner,
  Button,
  IconButton,
  Card,
  ConnectivityCheck,
  EmptyState,
  LampText,
  LedMeter,
  Panel,
  Readout,
  Table,
  type TableColumn,
  Toggle,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { useMemo, useState } from "react";
import { PairingPanel } from "../features/pairing/PairingPanel";
import { levelFraction, useAudioMeter } from "../features/audio/useAudioMeter";
import { useTickingNow } from "../features/dictation/useDictation";
import { usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { useShell } from "../app/shell-context";

const PAIRING_TTL_SECS = 120;

/** What travels over the E2EE channel (docs/dictation.md §20). */
const SYNC_ROWS = ["text", "audio", "result", "unpair"] as const;

const LIVE_METER_SEGMENTS = 28;

/** The phone microphone panel's line: a phone's take by its phase, else who is online. */
export function phoneTakeText(
  dictation: DictationStatus,
  onlinePhone: string | undefined,
  now: number,
  t: TFunction,
  locale: Parameters<typeof takeFailureText>[1],
): { text: string; tone: "ok" | "accent" | "danger" | "idle" } {
  const name = dictation.remote;
  const phase = dictation.phase;
  if (name !== undefined) {
    switch (phase.phase) {
      case "listening":
        return {
          text: t("devices.live.listening", {
            name,
            elapsed: formatElapsed(phase.ready ? now - phase.started_at : 0),
          }),
          tone: "ok",
        };
      case "processing":
        return { text: t("devices.live.processing", { name }), tone: "accent" };
      case "done":
        return { text: t("devices.live.done", { name, n: phase.chars }), tone: "ok" };
      case "failed":
        return {
          text: t("devices.live.failed", {
            name,
            reason: takeFailureText(
              { phase, kind: dictation.kind, source: dictation.source },
              locale,
            ),
          }),
          tone: "danger",
        };
      case "cancelled":
        return { text: t("devices.live.cancelled", { name }), tone: "idle" };
      case "idle":
        break;
    }
  }
  return onlinePhone === undefined
    ? { text: t("devices.live.none"), tone: "idle" }
    : { text: t("devices.live.ready", { name: onlinePhone }), tone: "idle" };
}

const LIMITS = ["notIme", "relayBlind", "identityAlert", "oneMic"] as const;

/** Phone microphone: pairing panel + device table (left), live / sync / limits (right). */
export function Devices() {
  const { backend } = useBackend();
  const state = useUiState();
  const shell = useShell();
  const { t, locale } = useI18n();
  const [dismissed, setDismissed] = useState<readonly string[]>([]);
  const now = useNow();

  const changed = useMemo(
    () =>
      state.devices.flatMap((d) =>
        d.connection.state === "identity_changed" && !dismissed.includes(d.device.public_key)
          ? [{ device: d, presented: d.connection.presented_fingerprint }]
          : [],
      ),
    [state.devices, dismissed],
  );

  const invoke = <C extends MutationCommand>(name: C, ...args: ArgsOf<C>) => {
    void backend.invoke(name, ...args);
  };

  const forget = (view: DeviceView) => {
    shell.confirm({
      title: t("devices.confirm.forgetTitle", { name: view.device.name }),
      body: t("devices.confirm.forgetBody"),
      facts: t("devices.confirm.forgetFacts", {
        fingerprint: view.device.fingerprint,
        date: formatDate(view.device.trusted_at),
      }),
      confirmLabel: t("devices.confirm.forget"),
      tone: "danger",
      onConfirm: () => {
        invoke("device_forget", { publicKey: view.device.public_key });
        shell.toast({
          message: t("devices.confirm.forgotten", { name: view.device.name }),
          duration: 3000,
        });
      },
    });
  };

  const online = state.devices.filter((d) => d.connection.state === "online");
  const columns: TableColumn<DeviceView>[] = [
    {
      id: "device",
      header: t("devices.column.device"),
      minWidth: 112,
      mono: false,
      cell: (r) => ({
        type: "two",
        primary: `${r.device.name} · ${platformLabel(r.device.platform, locale)}`,
        secondary: t("devices.column.pairedOn", {
          fingerprint: shortFingerprint(r.device.fingerprint),
          date: formatDate(r.device.trusted_at).slice(5),
        }),
      }),
    },
    {
      id: "lan",
      header: t("devices.column.lan"),
      width: 156,
      cell: (r) => ({ type: "mono", text: r.device.direct_hints?.[0] ?? "—", muted: true }),
    },
    {
      id: "seen",
      header: t("devices.column.lastSeen"),
      fit: true,
      align: "right",
      cell: (r) => ({
        type: "mono",
        text: relativeTime(r.device.last_seen, now, locale),
        muted: true,
      }),
    },
    {
      id: "state",
      header: t("devices.column.state"),
      fit: true,
      mono: false,
      cell: (r) => {
        const l = connectionLabel(r.connection, locale);
        return {
          type: "lamp",
          tone: l.tone === "neutral" ? "idle" : l.tone,
          text: l.text,
          pulse: r.connection.state === "connecting",
        };
      },
    },
    {
      id: "actions",
      header: "",
      width: 68,
      align: "right",
      mono: false,
      cell: (r) => (
        <span className="inline-flex gap-0.5">
          <IconButton
            icon="chat"
            label={t("devices.action.sendTest")}
            disabled={r.connection.state !== "online"}
            onClick={() => {
              invoke("send_text", {
                publicKey: r.device.public_key,
                body: t("devices.action.testBody"),
              });
            }}
          />
          <IconButton
            icon="trash"
            label={t("devices.action.forget")}
            tone="danger"
            onClick={() => {
              forget(r);
            }}
          />
        </span>
      ),
    },
  ];

  const startPairing = () => {
    invoke("pairing_start");
  };
  const regeneratePairing = () => {
    invoke("pairing_reset");
    invoke("pairing_start");
  };
  // Ctrl R (the footer's 重新生成二维码): a new code while one is shown or expired, a first one
  // when nothing is running; a handshake in progress is never interrupted by a key.
  usePageShortcuts((e) => {
    if (!withCommand(e) || e.key.toLowerCase() !== "r") return false;
    const phase = state.pairing.state.state;
    if (phase === "waiting_for_peer" || phase === "expired") regeneratePairing();
    else if (phase === "idle" || phase === "failed" || phase === "rejected") startPairing();
    // Swallowed either way: in the webview Ctrl R would reload the whole client.
    return true;
  });

  // docs/dictation.md §20: a phone's take runs here; the meter shows the phone's audio (the shell
  // feeds the phone's levels to the meter channel while it records).
  const dictation = state.dictation;
  const phoneListening = dictation.remote !== undefined && dictation.phase.phase === "listening";
  const meter = useAudioMeter(phoneListening);
  const tick = useTickingNow(phoneListening);
  const phone =
    (dictation.remote === undefined
      ? undefined
      : online.find((d) => d.device.name === dictation.remote)) ?? online[0];
  const live = phoneTakeText(dictation, phone?.device.name, tick, t, locale);
  const phoneLink =
    phone?.connection.state === "online" ? connectionKindLabel(phone.connection.via, locale) : "—";

  return (
    // pairing + device table left, LIVE / SYNC / LIMITS right. The right column follows
    // the window between 300 and 380 px (designed at 336); below `lg` it drops under the table.
    <div
      className="mx-auto grid w-full max-w-[1440px] grid-cols-1 gap-6 p-6 lg:grid-cols-[minmax(0,1fr)_minmax(300px,380px)]"
      data-testid="page-devices">
      <div className="flex min-w-0 flex-col gap-4">
        <PairingPanel
          pairing={state.pairing}
          identity={state.identity}
          relay={state.relay}
          ttlSecs={PAIRING_TTL_SECS}
          lanDiscovery={state.settings.lan_discovery}
          alwaysOn={state.settings.pairing_always_on}
          onAlwaysOn={(enabled) => {
            invoke("settings_set_pairing_always_on", { enabled });
          }}
          onStart={startPairing}
          onCancel={() => {
            invoke("pairing_cancel");
          }}
          onRegenerate={regeneratePairing}
          onDone={() => {
            invoke("pairing_reset");
          }}
          onConfirm={() => {
            invoke("pairing_confirm");
          }}
          onReject={() => {
            invoke("pairing_reject");
          }}
          onCopied={(what, ok) => {
            shell.toast(
              ok
                ? { message: t("devices.copied", { what }), duration: 2000 }
                : { message: t("common.clipboardUnavailable"), duration: 3000, tone: "danger" },
            );
          }}
        />

        {changed.map((c) => (
          <Banner
            key={c.device.device.public_key}
            tone="danger"
            marker="bar"
            title={t("devices.identityBanner.title", { name: c.device.device.name })}
            actions={
              <>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() => {
                    forget(c.device);
                  }}>
                  {t("devices.identityBanner.forget")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setDismissed((prev) => [...prev, c.device.device.public_key]);
                  }}>
                  {t("devices.identityBanner.later")}
                </Button>
              </>
            }>
            {t("devices.identityBanner.body", {
              recorded: c.device.device.fingerprint,
              presented: c.presented,
            })}
          </Banner>
        ))}

        <Panel
          eyebrow={t("devices.panel.paired")}
          right={
            <div className="flex flex-wrap items-center justify-end gap-2">
              <Toggle
                checked={state.settings.lan_discovery}
                onChange={(enabled) => {
                  invoke("settings_set_lan_discovery", { enabled });
                }}
                label={t("devices.panel.lanDiscovery")}
              />
              <Toggle
                checked={state.settings.relay_enabled}
                onChange={(enabled) => {
                  invoke("settings_set_relay", { url: state.settings.relay_url ?? null, enabled });
                }}
                label={t("devices.panel.allowRelay")}
                readout={
                  state.relay.endpoint ??
                  (state.relay.source === "builtin"
                    ? t("devices.panel.builtinRelay")
                    : t("devices.panel.noRelay"))
                }
              />
              <span className="mono whitespace-nowrap text-fg-muted">
                {t("devices.panel.count", { paired: state.devices.length, online: online.length })}
              </span>
            </div>
          }
          padding="sm">
          <Table
            label={t("devices.panel.paired")}
            columns={columns}
            rows={state.devices}
            rowKey={(r) => r.device.public_key}
            rowClassName={(r) =>
              r.connection.state === "identity_changed" ? "bg-danger-soft/40" : undefined
            }
            empty={
              <EmptyState compact icon="phone" title={t("devices.panel.emptyTitle")}>
                {t("devices.panel.emptyBody")}
              </EmptyState>
            }
          />
          <p className="mt-2 text-[11px] text-fg-subtle">{t("devices.panel.note")}</p>
          <p className="mt-1 text-[11px] text-fg-subtle">{t("devices.panel.lanNote")}</p>
        </Panel>
      </div>

      <div className="flex min-w-0 flex-col gap-4">
        <Panel
          eyebrow={t("devices.live.title")}
          data-testid="live-panel"
          data-remote={dictation.remote}>
          <LampText tone={live.tone} pulse={phoneListening}>
            {live.text}
          </LampText>
          <LedMeter
            level={phoneListening && meter.frame ? levelFraction(meter.frame.rms_dbfs) : 0}
            segments={LIVE_METER_SEGMENTS}
            size="md"
            className="mt-3"
            disabled={!phoneListening}
            label={t("devices.live.meter")}
          />
          <div className="mono mt-1 flex justify-between text-[10px] text-fg-subtle">
            <span>-60</span>
            <span>-40</span>
            <span>-20</span>
            <span>-12</span>
            <span>-6</span>
            <span>0 dBFS</span>
          </div>
          <div className="mt-3 grid grid-cols-3 gap-2">
            <Readout
              label={t("devices.live.audio")}
              value={t("devices.live.audioValue")}
              size="sm"
            />
            <Readout
              label={t("devices.live.phone")}
              value={phone?.device.name ?? "—"}
              size="sm"
              muted={phone === undefined}
            />
            <Readout
              label={t("devices.live.link")}
              value={phoneLink}
              size="sm"
              muted={phone === undefined}
            />
          </div>
          <p className="mt-3 text-[12px] text-fg-muted">{t("devices.live.note")}</p>
        </Panel>

        <Panel eyebrow={t("connectivity.title")}>
          <ConnectivityCheck
            status={state.connectivity}
            onRun={() => {
              void backend.invoke("connectivity_check");
            }}
          />
        </Panel>

        <Panel
          eyebrow={t("devices.syncPanel.title")}
          right={
            <span className="mono text-fg-subtle">
              {online.length > 0 ? t("devices.syncPanel.inSync") : t("devices.syncPanel.localOnly")}
            </span>
          }>
          <p className="text-[12px] leading-5 text-fg-muted">{t("devices.syncPanel.body")}</p>
          <table className="mt-2 w-full text-[12px]">
            <tbody>
              {SYNC_ROWS.map((row) => (
                <tr key={row} className="border-t border-border">
                  <td className="py-1.5 text-fg">{t(`devices.sync.${row}`)}</td>
                  <td className="mono py-1.5 text-fg-muted">{t(`devices.sync.${row}Dir`)}</td>
                  <td className="mono py-1.5 text-right text-fg-subtle">
                    {t(`devices.sync.${row}How`)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="mono mt-2 text-[10px] text-fg-subtle">
            {t("devices.syncPanel.crypto")}
          </div>
        </Panel>

        <Card>
          <div className="eyebrow mb-2">{t("devices.limitsTitle")}</div>
          <ul className="flex flex-col gap-1.5 text-[12px] leading-5 text-fg-muted">
            {LIMITS.map((l) => (
              <li key={l} className="flex gap-2">
                <span className="text-fg-subtle">×</span>
                <span>{t(`devices.limits.${l}`)}</span>
              </li>
            ))}
          </ul>
        </Card>
      </div>
    </div>
  );
}
