import {
  type HistoryEntry,
  activationChip,
  activationHint,
  connectionLabel,
  engineReady as engineReadiness,
  enginesReported,
  formatCount,
  formatMs,
  formatSeconds,
  isBuiltinPreset,
  modelDisplayName,
  outcomeLabel,
  platformLabel,
  presetLabel,
  relayLabel,
  takePhaseLabel,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  Chip,
  EmptyState,
  Eyebrow,
  Heatmap,
  Icon,
  Keycaps,
  Lamp,
  LampText,
  Panel,
  Readout,
  Table,
  type TableColumn,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useMemo } from "react";
import { SPEECH_ROUTE, useRouter } from "../app/router";
import { serviceTarget } from "./settings/engines/helpers";
import { MicrophoneStrength } from "../features/audio/MicrophoneStrength";
import { useAudioMeter } from "../features/audio/useAudioMeter";
import { useChosenMicrophone, useMicrophoneTest } from "../features/audio/useMicrophoneTest";
import { useDictation, useTickingNow } from "../features/dictation/useDictation";
import { ResultActions } from "../features/history/ResultActions";
import { PermissionNotice } from "../features/permissions/PermissionNotice";
import { PresetMenu } from "../features/presets/PresetMenu";
import {
  type HistoryFilter,
  historyStats,
  recentTimeLabel,
  spokenLabel,
  todayLabel,
} from "../features/history/stats";
import { shortModel } from "../shell/page-meta";

/** How many paired phones the phone-microphone card lists before pointing at the devices page. */
const HOME_DEVICE_ROWS = 3;
/** Rows of the recent-results table. */
const RECENT_ROWS = 6;

/** Home: readiness row, four dashboard panels, stat strip and the recent table, every
 *  number from the core: `state.dictation` drives the start / stop button and the live phase
 *  line, `state.engines` the engine card, `state.history` the session panel, the tiles and the
 *  table, the native meter the microphone card, `state.devices` / `state.relay` the phone card. */
export function Home() {
  const { navigate } = useRouter();
  const state = useUiState();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const dictation = useDictation();
  const now = useTickingNow(dictation.listening);
  // The microphone stays closed while idle (user feedback 2026-09-28): the card meters only during
  // a take (the recorder's own frames) or a 测试麦克风 run, on the device the settings choose.
  const micTest = useMicrophoneTest();
  const meter = useAudioMeter(micTest.testing || dictation.listening, useChosenMicrophone());
  const engines = state.engines;
  const hotkey = state.settings.hotkey;
  // docs/dictation.md §13: the chip and the empty-state hint say how the chord drives a take.
  const activation = state.settings.activation;
  const local = engines.asr_provider === "local";
  const reported = enginesReported(engines);
  const providerName = t(`engines.provider.${engines.asr_provider}`);
  // docs/dictation.md §21: the preset the next take's clean-up runs with (a custom one's name is
  // the user's text).
  const presetId = state.settings.engines.refine_preset;
  const presetName = presetLabel(presetId, state.presets, locale);
  const presetText = isBuiltinPreset(presetId) ? {} : { "data-user-text": "" };

  // docs/dictation.md §3 / §10: the core says whether recognition can run (a local model installed,
  // a provider with its key and endpoint).
  const engineReady = engineReadiness(engines);
  const micReady = meter.error === undefined;
  const ready = engineReady && micReady;
  const phase = dictation.phase;
  const idle = phase.phase === "idle";
  // docs/dictation.md §19: a voice edit reads as one (编辑指令 / 改写中 / 已替换).
  const phaseLabel = takePhaseLabel(dictation.status, now, locale);
  // Local mode: the core's display name (zh) or the dictionary's name by id under en.
  const asrModel = local
    ? modelDisplayName(engines.local_model ?? "", engines.asr_model, locale)
    : shortModel(engines.asr_model);
  // docs/dictation.md §11: the streaming path stopped following the audio this take; the final
  // text is unaffected, so this is a one-line note, not an error.
  const liveDegraded = phase.phase === "listening" ? phase.live?.degraded : undefined;
  const blockedReason = !engineReady
    ? !reported
      ? t("home.blocked.waitingCore")
      : engines.asr_issue === "model_not_installed"
        ? t("home.blocked.noModel")
        : t("home.blocked.issue", {
            issue: t(`engines.issue.${engines.asr_issue ?? "unavailable"}`),
          })
    : t("home.blocked.mic", { error: meter.error ?? "" });
  // The chip beside it names the model and the provider, so the ready line does not repeat them.
  const phaseText = !ready
    ? blockedReason
    : idle
      ? local
        ? t("home.status.readyDetailLocal")
        : t("home.status.readyDetail")
      : phaseLabel.text;
  const audioTarget = serviceTarget(engines.asr_provider, engines.asr_host, t);
  const textTarget = serviceTarget(engines.llm_provider, engines.refine_host, t);
  const openEngines = () => {
    navigate(SPEECH_ROUTE);
  };

  const stats = useMemo(() => historyStats(state.history, now), [state.history, now]);
  const recent = state.history.slice(0, RECENT_ROWS);

  // Phone link summary: the first online phone names the card's lamp, otherwise a connecting one,
  // otherwise offline (or "no device" when nothing is paired).
  const devices = state.devices;
  const linked =
    devices.find((d) => d.connection.state === "online") ??
    devices.find((d) => d.connection.state === "connecting");
  const phoneLink = linked
    ? connectionLabel(linked.connection, locale)
    : devices.length === 0
      ? { text: t("home.devices.unpaired"), tone: "idle" as const }
      : { text: t("home.devices.offline"), tone: "idle" as const };
  const relay = relayLabel(state.relay, locale);
  const openDevices = () => {
    navigate({ name: "devices" });
  };
  const openHistory = (filter?: string) => {
    navigate(filter === undefined ? { name: "history" } : { name: "history", filter });
  };
  const setRefine = (refine_enabled: boolean) => {
    void backend.invoke("settings_set_engines", {
      engines: { ...state.settings.engines, refine_enabled },
    });
  };

  const tiles: { eyebrow: string; value: string; secondary: string; filter: HistoryFilter }[] = [
    {
      eyebrow: t("home.tiles.today"),
      value: t("count.entries", { n: stats.today.count }),
      secondary: t("count.chars", { n: formatCount(stats.today.chars) }),
      filter: "today",
    },
    {
      eyebrow: t("home.tiles.week"),
      value: t("count.entries", { n: stats.week.count }),
      secondary: t("count.chars", { n: formatCount(stats.week.chars) }),
      filter: "week",
    },
    {
      eyebrow: t("home.tiles.month"),
      value: t("count.entries", { n: stats.month.count }),
      secondary: t("count.chars", { n: formatCount(stats.month.chars) }),
      filter: "month",
    },
    {
      eyebrow: t("home.tiles.total"),
      value: `${stats.total.count} / ${state.settings.history.keep}`,
      secondary: state.settings.history.enabled
        ? t("home.tiles.limit", { n: state.settings.history.keep })
        : t("home.tiles.recordingOff"),
      filter: "all",
    },
  ];

  const columns: TableColumn<HistoryEntry>[] = [
    // shows every mono cell on one line (`14:32:07`, `1,384 ms`): the widths below fit the
    // longest value at 13 px mono plus the cell padding.
    {
      id: "time",
      header: t("home.table.time"),
      width: 104,
      cell: (r) => ({ type: "mono", text: recentTimeLabel(r.at_ms, now, locale), muted: true }),
    },
    {
      id: "text",
      header: t("home.table.text"),
      mono: false,
      cell: (r) => {
        const outcome = outcomeLabel(r.outcome, locale);
        return (
          <span className="flex items-center gap-2">
            {r.kind === "edit" && (
              // docs/dictation.md §19.5: a voice edit's row is its rewrite, badged as one.
              <span
                className="mono shrink-0 rounded-6 border border-accent px-1 text-[10px] text-accent-text"
                data-testid="home-edit-badge">
                {t("history.edit.badge")}
              </span>
            )}
            <span className="truncate text-fg" data-user-text>
              {r.text}
            </span>
            {r.outcome.kind !== "inserted" && (
              <span
                className={`mono shrink-0 rounded-6 border px-1 text-[10px] ${outcome.tone === "danger" ? "border-danger text-danger" : "border-warning text-warning"}`}>
                {outcome.text}
              </span>
            )}
          </span>
        );
      },
    },
    {
      id: "engine",
      header: t("home.table.engine"),
      width: 150,
      mono: false,
      cell: (r) => ({ type: "text", text: shortModel(r.asr_model), muted: true }),
    },
    {
      id: "dur",
      header: t("home.table.duration"),
      width: 64,
      align: "right",
      cell: (r) => ({ type: "mono", text: formatSeconds(r.duration_ms) }),
    },
    {
      id: "asr",
      header: t("home.table.asr"),
      fit: true,
      align: "right",
      cell: (r) => ({ type: "mono", text: formatMs(r.asr_ms) }),
    },
    {
      id: "llm",
      header: t("home.table.refine"),
      fit: true,
      align: "right",
      cell: (r) => ({
        type: "mono",
        text: formatMs(r.refine_ms),
        muted: r.refine_ms === undefined,
      }),
    },
    {
      // Copy / paste into the previous window without opening the row (plan 1.4).
      id: "actions",
      header: <span className="sr-only">{t("home.table.actions")}</span>,
      width: 64,
      align: "right",
      mono: false,
      cell: (r) => <ResultActions entry={r} />,
    },
  ];

  return (
    <div className="mx-auto flex w-full max-w-[1600px] flex-col gap-3 p-6" data-testid="page-home">
      <Card
        padding="none"
        className="flex min-h-[52px] flex-wrap items-center gap-3 px-3.5 py-2"
        data-testid="home-readiness">
        <Lamp
          tone={
            !ready ? "danger" : idle ? "ok" : phaseLabel.tone === "neutral" ? "ok" : phaseLabel.tone
          }
          size={10}
          pulse={dictation.listening || dictation.processing}
        />
        <div className="flex min-w-0 flex-1 items-baseline gap-2">
          <span className="text-[15px] font-medium whitespace-nowrap text-fg">
            {!ready
              ? t("home.status.notReady")
              : dictation.listening
                ? t("home.status.listening")
                : dictation.processing
                  ? t("home.status.processing")
                  : t("home.status.ready")}
          </span>
          <span
            className={`truncate text-[13px] ${phaseLabel.tone === "danger" ? "text-danger" : "text-fg-muted"}`}
            title={phaseText}
            data-testid="home-phase">
            {phaseText}
          </span>
        </div>
        <Keycaps keys={hotkey} />
        <Chip
          onClick={() => {
            navigate({ name: "settings", section: "hotkey" });
          }}>
          {activationChip(activation, locale)}
        </Chip>
        <Chip onClick={openEngines}>
          {!reported
            ? t("home.chip.notReady")
            : local
              ? t("home.chip.local", { model: asrModel })
              : t("home.chip.provider", { provider: providerName, model: asrModel })}
        </Chip>
        <PresetMenu
          align="end"
          data-testid="home-preset"
          title={engines.refine_enabled ? undefined : t("home.engine.refineOff")}
          triggerClassName={`inline-flex h-7 items-center gap-1.5 rounded-6 bg-surface px-2.5 text-[12px] whitespace-nowrap hairline hover:border-fg-subtle ${engines.refine_enabled ? "text-fg" : "text-fg-muted"}`}
          trigger={
            <>
              <Icon
                name="wand"
                size={14}
                className={engines.refine_enabled ? "text-accent-text" : "text-fg-subtle"}
              />
              <span {...presetText}>{presetName}</span>
              <Icon name="chevronDown" size={12} className="text-fg-subtle" />
            </>
          }
        />
        {dictation.listening && (
          <Button variant="text" size="sm" onClick={dictation.cancel}>
            {t("home.button.cancel")}
          </Button>
        )}
        {dictation.listening ? (
          <Button variant="danger" icon="stop" onClick={dictation.stop}>
            {t("home.button.stop")}
          </Button>
        ) : (
          <Button
            variant="primary"
            icon="mic"
            disabled={!ready || dictation.processing}
            title={
              !ready
                ? blockedReason
                : dictation.processing
                  ? t("home.button.processingTitle")
                  : undefined
            }
            onClick={dictation.start}>
            {dictation.processing ? t("home.button.processing") : t("home.button.start")}
          </Button>
        )}
      </Card>

      <PermissionNotice />

      {/* The 2×2 dashboard: one column below `lg`, two fluid columns above; each panel keeps
          its 144 px minimum height while the width follows the window. */}
      <div className="grid grid-cols-1 gap-x-4 gap-y-3 lg:grid-cols-2">
        <Panel
          eyebrow={t("home.mic.eyebrow")}
          right={
            <LampText
              tone={
                meter.error
                  ? "danger"
                  : dictation.listening || micTest.testing
                    ? meter.frame
                      ? "ok"
                      : "idle"
                    : "idle"
              }
              mono
              pulse={meter.frame !== undefined}>
              <span data-testid="home-mic-state">
                {meter.error
                  ? t("home.mic.unavailable")
                  : dictation.listening
                    ? t("home.mic.recording")
                    : micTest.testing
                      ? t("home.mic.testing", { n: micTest.remaining })
                      : t("home.mic.idle")}
              </span>
            </LampText>
          }
          className="min-h-[144px]"
          data-testid="home-mic">
          <div className="text-[13px] font-medium text-fg" data-testid="home-mic-device">
            {meter.device?.name ?? meter.error ?? t("home.mic.enumerating")}
          </div>
          <div className="mono mt-0.5 text-[11px] text-fg-muted">
            {meter.device
              ? [
                  meter.device.sample_rate_hz ? `${meter.device.sample_rate_hz / 1000} kHz` : "",
                  meter.device.channels === 1
                    ? t("settings.microphone.mono")
                    : meter.device.channels
                      ? t("settings.microphone.channels", { n: meter.device.channels })
                      : "",
                  meter.device.is_default ? t("home.mic.systemDefault") : t("home.mic.selected"),
                ]
                  .filter((part) => part.length > 0)
                  .join(" · ")
              : "—"}
          </div>
          {meter.missing && (
            <div className="mt-0.5 text-[11px] text-warning" data-testid="home-mic-missing">
              {t("home.mic.missing")}
            </div>
          )}
          <MicrophoneStrength
            frame={meter.frame}
            disabled={meter.error !== undefined}
            className="mt-3"
            data-testid="home-mic-level"
          />
          <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
            {!dictation.listening && (
              <Button
                size="sm"
                variant={micTest.testing ? "outline" : "primary"}
                icon={micTest.testing ? "stop" : "mic"}
                disabled={meter.error !== undefined || dictation.processing}
                data-testid="home-mic-test"
                onClick={micTest.testing ? micTest.stop : micTest.start}>
                {micTest.testing ? t("home.mic.stopTest") : t("home.mic.test")}
              </Button>
            )}
            <span className="min-w-0 flex-1 text-[11px] text-fg-muted" data-testid="home-mic-hint">
              {dictation.listening
                ? t("home.mic.recordingHint")
                : micTest.testing
                  ? t("home.mic.testingHint", { n: micTest.remaining })
                  : t("home.mic.idleHint")}
            </span>
            <Button
              size="sm"
              variant="text"
              data-testid="home-mic-switch"
              title={meter.devices ? t("home.mic.devices", { n: meter.devices.length }) : undefined}
              onClick={() => {
                navigate({ name: "settings", section: "microphone" });
              }}>
              {t("home.mic.switch")}
            </Button>
          </div>
        </Panel>

        <Panel
          eyebrow={t("home.engine.eyebrow")}
          right={
            <Button size="sm" variant="ghost" icon="key" onClick={openEngines}>
              {t("home.engine.configure")}
            </Button>
          }
          className="min-h-[144px]"
          data-testid="home-engine">
          <div className="flex items-center gap-2 text-[13px] font-medium text-fg">
            <span className="truncate">{reported ? asrModel : t("home.engine.waiting")}</span>
            {local && <Badge tone="accent">{t("home.engine.local")}</Badge>}
            {engines.live_preview_ready && (
              <span className="inline-flex" data-testid="home-live-preview">
                <Badge tone="accent">{t("home.engine.livePreview")}</Badge>
              </span>
            )}
          </div>
          <div className="mono mt-0.5 text-[11px] text-fg-muted">
            {reported
              ? local
                ? t("home.engine.detailLocal", {
                    state: engineReady
                      ? t("home.engine.localReady")
                      : t("home.engine.localMissing"),
                    language: engines.language ?? t("home.engine.auto"),
                  })
                : t("home.engine.detail", {
                    provider: providerName,
                    language: engines.language ?? t("home.engine.auto"),
                  })
              : "—"}
          </div>
          {liveDegraded !== undefined && (
            <div
              className="mt-1 truncate text-[11px] text-warning"
              title={liveDegraded}
              data-testid="home-live-degraded">
              {t("home.engine.liveDegraded")}
            </div>
          )}
          <div className="mt-3 grid grid-cols-3 gap-3">
            <Readout
              label={t("home.engine.refineModel")}
              value={engines.refine_enabled ? shortModel(engines.refine_model) : t("common.off")}
              size="sm"
              muted={!engines.refine_enabled}
            />
            <Readout
              label={t("home.engine.preset")}
              value={
                <span data-testid="home-engine-preset" {...presetText}>
                  {presetName}
                </span>
              }
              size="sm"
              muted={!engines.refine_enabled}
            />
            <Readout
              label={t("home.engine.inject")}
              value={
                // The one place that sets it is 设置 › 听写 (plan 1.5).
                <button
                  type="button"
                  className="text-accent-text hover:text-accent-text-hover"
                  title={t("home.engine.injectChange")}
                  data-testid="home-inject-link"
                  onClick={() => {
                    navigate({ name: "settings", section: "dictation" });
                  }}>
                  {engines.inject === "paste"
                    ? t("home.engine.injectPaste")
                    : t("home.engine.injectClipboard")}
                </button>
              }
              size="sm"
            />
          </div>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
            <Toggle
              checked={engines.refine_enabled}
              onChange={setRefine}
              label={
                engines.refine_enabled
                  ? t("home.engine.refineOn", { model: shortModel(engines.refine_model) })
                  : t("home.engine.refineOff")
              }
            />
            <span className="text-[11px] text-fg-muted" data-testid="home-privacy">
              {reported
                ? `${audioTarget === undefined ? t("home.engine.privacyLocal") : t("home.engine.privacyAudio", { target: audioTarget })}${engines.refine_enabled && textTarget !== undefined ? t("home.engine.privacyText", { target: textTarget }) : ""}`
                : ""}
            </span>
          </div>
        </Panel>

        <Panel
          eyebrow={t("home.devices.eyebrow")}
          right={
            <LampText
              tone={phoneLink.tone}
              mono
              pulse={linked?.connection.state === "connecting"}
              readout={t("home.devices.paired", { n: devices.length })}>
              {phoneLink.text}
            </LampText>
          }
          className="min-h-[144px]"
          data-testid="home-devices">
          {devices.length === 0 ? (
            <button
              type="button"
              className="text-left text-[13px] text-fg-muted hover:underline"
              onClick={openDevices}>
              {t("home.devices.none")}
            </button>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {devices.slice(0, HOME_DEVICE_ROWS).map((d) => {
                const link = connectionLabel(d.connection, locale);
                return (
                  <li key={d.device.public_key} className="flex items-center gap-2 text-[13px]">
                    <Lamp tone={link.tone} size={6} pulse={d.connection.state === "connecting"} />
                    <span className="truncate font-medium text-fg">{d.device.name}</span>
                    <span className="mono rounded-6 border border-border px-1.5 text-[10px] text-fg-muted">
                      {platformLabel(d.device.platform, locale)}
                    </span>
                    <span className="mono ml-auto text-[11px] whitespace-nowrap text-fg-muted">
                      {link.text}
                    </span>
                  </li>
                );
              })}
              {devices.length > HOME_DEVICE_ROWS && (
                <li className="mono text-[11px] text-fg-subtle">
                  {t("home.devices.more", { n: devices.length - HOME_DEVICE_ROWS })}
                </li>
              )}
            </ul>
          )}
          <div className="mt-3 flex items-center gap-2">
            <LampText tone={relay.tone} mono size="sm">
              {t("home.devices.relay", { state: relay.text })}
            </LampText>
            <Button size="sm" className="ml-auto" onClick={openDevices}>
              {t("home.devices.open")}
            </Button>
          </div>
        </Panel>

        <Panel
          eyebrow={t("home.session.eyebrow")}
          right={<span className="mono text-fg-muted">{todayLabel(now, locale)}</span>}
          className="min-h-[144px]"
          data-testid="home-session">
          <div className="flex justify-between gap-4">
            <div className="grid grid-cols-2 gap-x-6 gap-y-3">
              <Readout
                label={t("home.session.sentences")}
                value={stats.today.count}
                unit={t("home.session.sentencesUnit") || undefined}
                size="lg"
              />
              <Readout
                label={t("home.session.chars")}
                value={formatCount(stats.today.chars)}
                unit={t("home.session.charsUnit") || undefined}
                size="lg"
              />
              <Readout
                label={t("home.session.spoken")}
                value={spokenLabel(stats.today.spokenMs)}
                unit={t("home.session.spokenUnit")}
                size="lg"
              />
              <Readout
                label={t("home.session.latency")}
                value={stats.today.latencyMs ?? "—"}
                unit="ms"
                size="lg"
                muted={stats.today.latencyMs === undefined}
              />
            </div>
            <div className="flex flex-col items-end gap-1">
              <Heatmap values={stats.heatmap} legend={false} />
              <span className="mono text-[10px] text-fg-subtle">{t("home.session.heatmap")}</span>
            </div>
          </div>
        </Panel>
      </div>

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        {tiles.map((tile) => (
          <Card
            key={tile.filter}
            padding="none"
            interactive
            role="button"
            tabIndex={0}
            aria-label={`${tile.eyebrow} ${tile.value}`}
            onClick={() => {
              openHistory(tile.filter);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") openHistory(tile.filter);
            }}
            className="flex h-[52px] cursor-pointer flex-col justify-center px-3">
            <span className="text-[10px] text-fg-subtle">{tile.eyebrow}</span>
            <span className="flex items-baseline justify-between">
              <span className="mono text-[16px] whitespace-nowrap text-fg">{tile.value}</span>
              <span
                className="mono truncate pl-2 text-[11px] text-fg-muted"
                title={tile.secondary}
                data-testid="home-tile-note">
                {tile.secondary}
              </span>
            </span>
          </Card>
        ))}
      </div>

      <div>
        <Eyebrow
          className="mb-1.5"
          right={
            <Button
              variant="text"
              size="sm"
              onClick={() => {
                openHistory();
              }}>
              {t("home.recent.all")}
            </Button>
          }>
          {t("home.recent.title")}
        </Eyebrow>
        <Card padding="none" className="overflow-hidden">
          {recent.length === 0 ? (
            <EmptyState
              icon="mic"
              title={t("home.recent.emptyTitle")}
              mono={activationHint(activation, hotkey, locale)}
              className="min-h-[160px]">
              {t("home.recent.emptyBody")}
            </EmptyState>
          ) : (
            <Table
              label={t("home.recent.title")}
              columns={columns}
              rows={recent}
              rowKey={(r) => r.id}
              dense
              onSelect={(r) => {
                openHistory(r.id);
              }}
            />
          )}
        </Card>
      </div>
    </div>
  );
}
