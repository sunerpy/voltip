import {
  type HistoryEntry,
  type HistoryOutcome,
  activationChip,
  activationHint,
  dayLabel,
  clockLabel,
  engineReady as engineReadiness,
  enginesReported,
  formatCount,
  formatDuration,
  durationParts,
  formatTakeLength,
  groupByDay,
  startOfDay,
  isBuiltinPreset,
  modelDisplayName,
  outcomeLabel,
  presetLabel,
  resetLabel,
  takePhaseLabel,
  usableFallback,
} from "@voltip/shared";
import {
  Button,
  Card,
  cx,
  EmptyState,
  Icon,
  type IconName,
  IconButton,
  Lamp,
  RefineNotice,
  splitKeys,
  Toggle,
  UsageInline,
  useBackend,
  useI18n,
  useServiceUsage,
  useUiState,
  useUsageNow,
} from "@voltip/ui";
import { useRef } from "react";
import { AI_ROUTE, SPEECH_ROUTE, useRouter } from "../app/router";
import {
  UsageNotices,
  installedLocalModel,
  useSwitchToLocal,
} from "../features/usage/UsageNotices";
import { useShell } from "../app/shell-context";
import { serviceTarget } from "./settings/engines/helpers";
import { levelFraction, useAudioMeter } from "../features/audio/useAudioMeter";
import { useChosenMicrophone, useMicrophoneTest } from "../features/audio/useMicrophoneTest";
import { useDictation, useTickingNow } from "../features/dictation/useDictation";
import { HomeRecordings } from "../features/history/Recordings";
import { ResultActions } from "../features/history/ResultActions";
import { PermissionNotice } from "../features/permissions/PermissionNotice";
import { PresetMenu } from "../features/presets/PresetMenu";
import {
  MicrophoneMenu,
  PolishModelMenu,
  ReadoutTrigger,
  SpeechModelMenu,
} from "../features/switchers/SwitcherMenus";
import { type HistoryFilter, recentTimeLabel } from "../features/history/stats";
import { useRowsThatFit } from "../features/history/recent-rows";
import { useHomeStats } from "../features/history/useHomeStats";
import { shortModel } from "../shell/page-meta";

/** A card's first line (the model) as a menu button. */
const TITLE_MENU =
  "-mx-1 inline-flex min-w-0 items-center gap-1.5 rounded-control px-1 text-[14px] font-semibold text-fg transition-colors hover:bg-inset";
/** A card's detail line (the clean-up's model) as a menu button. */
const DETAIL_MENU =
  "-mx-1 inline-flex min-w-0 items-center gap-1 rounded-control px-1 text-[12px] text-fg-muted transition-colors hover:bg-inset";

/** The hue each span's stat tile takes (tokens.css); the total has a card of its own. */
const TILE_HUE: Partial<Record<HistoryFilter, string>> = {
  today: "stat-tile-today",
  week: "stat-tile-week",
  month: "stat-tile-month",
};

/** Rows of the recent results at the least; a taller window shows more (`useRowsThatFit`). */
const RECENT_ROWS = 4;

/** The soft hues an app's letter tile takes (tokens only), picked by the app's id so that an app
 *  keeps its colour from row to row. */
const APP_HUES = [
  "bg-accent-soft text-accent-text",
  "bg-ok-soft text-ok-text",
  "bg-warning-soft text-warning",
  "bg-info-soft text-info",
] as const;

export function appHue(id: string): string {
  let h = 0;
  for (const ch of id) h = (h * 31 + (ch.codePointAt(0) ?? 0)) % 1009;
  return APP_HUES[h % APP_HUES.length] ?? APP_HUES[0];
}

/** A span of time on a stat tile: each number with its unit smaller (docs/dictation.md §4.5). */
function Duration({
  ms,
  unitClassName = "text-[11px] text-fg-muted",
}: {
  ms: number;
  unitClassName?: string;
}) {
  const { locale } = useI18n();
  return (
    <span data-testid="home-duration">
      {durationParts(ms, locale).map((part, i) => (
        <span key={part.unit} className={i > 0 ? "ml-1.5" : undefined}>
          {part.value}
          <span className={cx("ml-1 font-normal", unitClassName)}>{part.unit}</span>
        </span>
      ))}
    </span>
  );
}

/** The outcome chip at a row's end: inserted (a status dot), only copied (warning) or failed. */
function OutcomeChip({ outcome }: { outcome: HistoryOutcome }) {
  const { locale } = useI18n();
  const label = outcomeLabel(outcome, locale);
  return (
    <span
      data-testid="home-outcome"
      data-tone={label.tone}
      className={cx(
        "pointer-events-none inline-flex h-7 max-w-[240px] min-w-0 shrink-0 items-center gap-1.5 rounded-pill px-2.5 text-[12px] font-medium whitespace-nowrap",
        label.tone === "ok" && "border border-field-edge bg-field text-fg-muted",
        label.tone === "warn" && "bg-warning-soft text-warning",
        label.tone === "danger" && "bg-danger-soft text-danger",
      )}>
      {label.tone === "ok" ? (
        <Lamp tone="ok" size={6} />
      ) : (
        <Icon name={label.tone === "danger" ? "alert" : "copy"} size={13} className="shrink-0" />
      )}
      <span className="truncate">{label.text}</span>
    </span>
  );
}

/** Home (Penpot 「澄空 Glass · Voltip」 board 01): the 听写 card (readiness, the chord, start), the
 *  识别与润色 card (the speech model, AI 润色, the microphone), the four stat tiles, the recordings
 *  no text came from (while there are any) and the recent results. Every number comes from the core: `state.dictation` drives the start / stop button and
 *  the live phase line, `state.engines` the cards, `state.history_recent` the list and
 *  `history_stats` (docs/dictation.md §4.5) the tiles. */
export function Home() {
  const { navigate } = useRouter();
  const state = useUiState();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const dictation = useDictation();
  const now = useTickingNow(dictation.listening);
  // The microphone stays closed while idle (user feedback 2026-09-28): it meters only during a
  // take (the recorder's own frames) or a 测试麦克风 run, on the device the settings choose.
  const micTest = useMicrophoneTest();
  const meter = useAudioMeter(micTest.testing || dictation.listening, useChosenMicrophone());
  const engines = state.engines;
  const hotkey = state.settings.hotkey;
  // docs/dictation.md §13: how the chord drives a take.
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
  const phaseText = !ready
    ? blockedReason
    : idle
      ? local
        ? t("home.status.readyDetailLocal")
        : t("home.status.readyDetail")
      : phaseLabel.text;
  const textTarget = serviceTarget(engines.llm_provider, engines.refine_host, t);
  const openEngines = () => {
    navigate(SPEECH_ROUTE);
  };
  // docs/dictation.md §27: the built-in service's day, under each model while it is the one in use.
  const usageNow = useUsageNow();
  const asrUsage = useServiceUsage("asr");
  const llmUsage = useServiceUsage("llm");
  const builtinAsr = state.usage.builtin && engines.asr_provider === "builtin";
  const builtinLlm = state.usage.builtin && engines.llm_provider === "builtin";
  // Used up with nothing to stand in for it: the 听写 card says so and offers the way out; the
  // shortcut and the button still record, and the recording is kept to recognise again.
  const asrOut =
    builtinAsr &&
    asrUsage.tone === "out" &&
    usableFallback(engines.asr_fallback, usageNow) === undefined;
  const localModel = installedLocalModel(state.models, state.settings.engines);
  const switchToLocal = useSwitchToLocal();
  const openLocalModels = () => {
    navigate({ name: "speech", section: "local" });
  };
  const usedUp = asrOut && ready && idle;
  // The usage notice (or, once closed, the usage line) says the built-in clean-up's day is used
  // up; the core's quota notice would say it again.
  const refineOut = builtinLlm && llmUsage.tone === "out";
  const heroText = usedUp
    ? t("usage.hero.body", {
        reset: resetLabel(asrUsage.resetAtMs ?? usageNow, usageNow, locale),
      })
    : phaseText;

  const stats = useHomeStats(now);
  const pageRef = useRef<HTMLDivElement>(null);
  const recentRef = useRef<HTMLDivElement>(null);
  // Which entries open a new day: each brings its day's heading into the list.
  const opensDay = state.history_recent.map(
    (entry, i) =>
      i === 0 || startOfDay(entry.at_ms) !== startOfDay(state.history_recent[i - 1]?.at_ms ?? 0),
  );
  const recentRows = useRowsThatFit(pageRef, recentRef, RECENT_ROWS, opensDay);
  const recent = state.history_recent.slice(0, recentRows);
  const openHistory = (filter?: string) => {
    navigate(filter === undefined ? { name: "history" } : { name: "history", filter });
  };
  const setRefine = (refine_enabled: boolean) => {
    void backend.invoke("settings_set_engines", {
      engines: { ...state.settings.engines, refine_enabled },
    });
  };
  // The same confirmation as the history page's delete.
  const remove = (entry: HistoryEntry) => {
    shell.confirm({
      title: t("history.confirm.deleteTitle"),
      body: t("history.confirm.deleteBody", {
        when: `${dayLabel(entry.at_ms, now, locale)} ${clockLabel(entry.at_ms)}`,
        excerpt: `${entry.text.slice(0, 40)}${entry.text.length > 40 ? "…" : ""}`,
      }),
      confirmLabel: t("common.delete"),
      tone: "danger",
      onConfirm: () => {
        void backend.invoke("history_delete", { id: entry.id });
      },
    });
  };

  // The start button's rings follow the input while a take or a test meters (board 01).
  const level =
    (dictation.listening || micTest.testing) && meter.frame !== undefined
      ? levelFraction(meter.frame.rms_dbfs)
      : 0;
  const lampTone = !ready
    ? "danger"
    : asrOut && idle
      ? "danger"
      : idle
        ? "ok"
        : phaseLabel.tone === "neutral"
          ? "ok"
          : phaseLabel.tone;

  const tiles: {
    eyebrow: string;
    icon: IconName;
    savedMs: number;
    count: number;
    secondary: string;
    filter: HistoryFilter;
  }[] = [
    {
      eyebrow: t("home.tiles.today"),
      icon: "sun",
      savedMs: stats.today.savedMs,
      count: stats.today.count,
      secondary: t("count.chars", { n: formatCount(stats.today.rawChars) }),
      filter: "today",
    },
    {
      eyebrow: t("home.tiles.week"),
      icon: "calendar",
      savedMs: stats.week.savedMs,
      count: stats.week.count,
      secondary: t("count.chars", { n: formatCount(stats.week.rawChars) }),
      filter: "week",
    },
    {
      eyebrow: t("home.tiles.month"),
      icon: "calendarGrid",
      savedMs: stats.month.savedMs,
      count: stats.month.count,
      secondary: t("count.chars", { n: formatCount(stats.month.rawChars) }),
      filter: "month",
    },
    {
      eyebrow: t("home.tiles.total"),
      icon: "clock",
      savedMs: stats.total.savedMs,
      count: stats.total.count,
      secondary: state.settings.history.enabled
        ? t("count.chars", { n: formatCount(stats.total.rawChars) })
        : t("home.tiles.recordingOff"),
      filter: "all",
    },
  ];

  const keys = splitKeys(hotkey);
  const hint = t(`home.hero.hint.${activation}`, {
    target:
      engines.inject === "paste" ? t("home.hero.target.paste") : t("home.hero.target.clipboard"),
  });
  const micState = meter.error
    ? t("home.mic.unavailable")
    : usedUp && !micTest.testing
      ? t("usage.hero.mic")
      : dictation.listening
        ? t("home.mic.recording")
        : micTest.testing
          ? t("home.mic.testing", { n: micTest.remaining })
          : t("home.mic.idle");
  const micDevice = meter.device;

  return (
    <div
      ref={pageRef}
      className="mx-auto flex w-full max-w-[1600px] flex-col gap-4 p-6"
      data-testid="page-home">
      <PermissionNotice />
      {/* The usage notice says it when the built-in clean-up's day is used up. */}
      {!(refineOut && state.refine_notice?.failure === "quota") && (
        <RefineNotice
          onOpen={() => {
            navigate(AI_ROUTE);
          }}
        />
      )}

      {/* The board's 600 : 360 split, fluid (no fixed column: regression test below). */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,5fr)_minmax(0,3fr)]">
        {/* 听写: readiness, the chord and the start button. */}
        <Card padding="none" className="flex min-h-[176px] gap-4 px-6 py-4" data-testid="home-hero">
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="text-[12px] font-medium text-fg-subtle">{t("home.hero.eyebrow")}</span>
            <div className="mt-2 flex min-w-0 items-center gap-2" data-testid="home-readiness">
              <Lamp tone={lampTone} size={8} pulse={dictation.listening || dictation.processing} />
              <span className="truncate text-[18px] font-medium text-fg">
                {!ready
                  ? t("home.status.notReady")
                  : dictation.listening
                    ? t("home.status.listening")
                    : dictation.processing
                      ? t("home.status.processing")
                      : usedUp
                        ? t("usage.hero.title")
                        : t("home.status.ready")}
              </span>
            </div>
            <span
              className={cx(
                "mt-0.5 truncate pl-4 text-[13px]",
                phaseLabel.tone === "danger" || !ready ? "text-danger" : "text-fg-muted",
              )}
              title={heroText}
              data-testid="home-phase">
              {heroText}
            </span>
            {usedUp ? (
              // docs/dictation.md §27: the ways on while the built-in recognition's day is used up.
              <div
                className="mt-auto flex flex-wrap items-center gap-2 pt-4"
                data-testid="home-usage-out">
                <Button
                  size="sm"
                  variant="primary"
                  data-testid="home-use-local"
                  onClick={() => {
                    if (localModel === undefined) openLocalModels();
                    else switchToLocal(localModel);
                  }}>
                  {localModel === undefined
                    ? t("usage.notice.downloadLocal")
                    : t("usage.notice.useLocal")}
                </Button>
                <Button size="sm" variant="outline" onClick={openEngines}>
                  {t("usage.notice.openSpeech")}
                </Button>
              </div>
            ) : (
              <div
                className="mt-auto flex flex-wrap items-center gap-2 pt-4"
                data-testid="home-hotkey"
                aria-label={keys.join(" ")}>
                {keys.map((key, i) => (
                  <kbd
                    key={`${key}-${i}`}
                    className="mono inline-flex h-11 min-w-[56px] items-center justify-center rounded-row border border-keycap-border bg-keycap-bg px-3.5 text-[18px] font-semibold text-fg shadow-field">
                    {key}
                  </kbd>
                ))}
                <button
                  type="button"
                  className="ml-2 rounded-control px-1 text-[15px] font-medium text-fg-muted transition-colors hover:text-fg"
                  data-testid="home-activation"
                  onClick={() => {
                    navigate({ name: "settings", section: "hotkey" });
                  }}>
                  {activationChip(activation, locale)}
                </button>
              </div>
            )}
            <span className="mt-2.5 text-[12px] text-fg-subtle" data-testid="home-hint">
              {usedUp ? t("usage.hero.note") : hint}
            </span>
          </div>
          <div className="flex w-34 shrink-0 flex-col items-center justify-center">
            <div className="relative grid size-[104px] place-items-center">
              <span
                aria-hidden
                data-testid="home-halo"
                className="absolute inset-0 rounded-full bg-accent opacity-10 transition-transform duration-150"
                style={{ transform: `scale(${(0.86 + level * 0.14).toFixed(3)})` }}
              />
              <span
                aria-hidden
                className="absolute inset-[10px] rounded-full bg-accent opacity-15 transition-transform duration-150"
                style={{ transform: `scale(${(0.9 + level * 0.1).toFixed(3)})` }}
              />
              {dictation.listening ? (
                <button
                  type="button"
                  aria-label={t("home.button.stop")}
                  title={t("home.button.stop")}
                  onClick={dictation.stop}
                  data-testid="home-stop"
                  className="relative grid size-16 place-items-center rounded-full bg-danger text-primary-fg shadow-primary transition-opacity hover:opacity-90">
                  <Icon name="stop" size={24} />
                </button>
              ) : (
                <button
                  type="button"
                  aria-label={
                    dictation.processing ? t("home.button.processing") : t("home.button.start")
                  }
                  title={
                    !ready
                      ? blockedReason
                      : dictation.processing
                        ? t("home.button.processingTitle")
                        : undefined
                  }
                  disabled={!ready || dictation.processing}
                  onClick={dictation.start}
                  data-testid="home-start"
                  className="relative grid size-16 place-items-center rounded-full bg-primary text-primary-fg shadow-primary transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50">
                  <Icon name="mic" size={26} />
                </button>
              )}
            </div>
            <span aria-hidden className="mt-1.5 text-[13px] font-medium text-fg">
              {dictation.listening
                ? t("home.button.stop")
                : dictation.processing
                  ? t("home.button.processing")
                  : t("home.button.start")}
            </span>
            {/* Two centred lines at most: 「Idle · microphone closed」 is wider than the column. */}
            <span
              className="mt-0.5 line-clamp-2 max-w-full text-center text-[11px] leading-4 text-fg-subtle"
              title={micState}
              data-testid="home-mic-state">
              {micState}
            </span>
            {dictation.listening && (
              <Button variant="text" size="sm" onClick={dictation.cancel} className="mt-1">
                {t("home.button.cancel")}
              </Button>
            )}
          </div>
        </Card>

        {/* 识别与润色: the speech model, AI 润色 with its preset and model, the microphone. */}
        <Card
          padding="none"
          className="flex min-h-[176px] flex-col px-6 py-4"
          data-testid="home-engine">
          <div className="flex items-center justify-between gap-2">
            <span className="text-[12px] font-medium text-fg-subtle">
              {t("home.engine.cardTitle")}
            </span>
            <button
              type="button"
              onClick={openEngines}
              data-testid="home-configure"
              className="inline-flex items-center gap-0.5 text-[12px] font-medium text-accent-text hover:text-accent-text-hover">
              {t("home.engine.configureShort")}
              <Icon name="chevronRight" size={12} />
            </button>
          </div>
          <div className="mt-2.5 flex min-w-0 items-start gap-2.5">
            <Icon name="wave" size={16} className="mt-1 shrink-0 text-accent" />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-2">
                {reported ? (
                  <SpeechModelMenu
                    data-testid="home-engine-model"
                    triggerClassName={TITLE_MENU}
                    trigger={<ReadoutTrigger value={asrModel} />}
                  />
                ) : (
                  <span className="truncate text-[14px] font-semibold text-fg">
                    {t("home.engine.waiting")}
                  </span>
                )}
                {local && (
                  <span className="shrink-0 rounded-pill bg-accent-soft px-2 py-0.5 text-[11px] font-medium text-accent-text">
                    {t("home.engine.local")}
                  </span>
                )}
                {engines.live_preview_ready && (
                  <span
                    className="shrink-0 rounded-pill bg-accent-soft px-2 py-0.5 text-[11px] font-medium text-accent-text"
                    data-testid="home-live-preview">
                    {t("home.engine.livePreview")}
                  </span>
                )}
              </div>
              {builtinAsr && asrUsage.tone !== "pending" ? (
                <UsageInline
                  usage={asrUsage}
                  prefix={`${providerName} · `}
                  data-testid="home-engine-usage"
                />
              ) : (
                <div
                  className="truncate text-[12px] text-fg-muted"
                  data-testid="home-engine-detail">
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
              )}
              {liveDegraded !== undefined && (
                <div
                  className="truncate text-[11px] text-warning"
                  title={liveDegraded}
                  data-testid="home-live-degraded">
                  {t("home.engine.liveDegraded")}
                </div>
              )}
            </div>
          </div>
          <div className="mt-2.5 flex min-w-0 items-start gap-2.5">
            <Icon name="wand" size={16} className="mt-1 shrink-0 text-primary" />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-1 text-[14px] font-medium text-fg">
                <span className="shrink-0">{t("shell.polish.label")}</span>
                <span aria-hidden className="text-fg-subtle">
                  ·
                </span>
                <PresetMenu
                  data-testid="home-preset"
                  title={engines.refine_enabled ? undefined : t("home.engine.refineOff")}
                  triggerClassName="-mx-1 inline-flex min-w-0 items-center gap-1 rounded-control px-1 text-[14px] font-medium transition-colors hover:bg-inset"
                  trigger={
                    <>
                      <span className="truncate" data-testid="home-engine-preset" {...presetText}>
                        {presetName}
                      </span>
                      <Icon name="chevronDown" size={12} className="text-fg-subtle" />
                    </>
                  }
                />
              </div>
              <div className="flex min-w-0 items-center gap-0.5 text-[12px] text-fg-muted">
                <PolishModelMenu data-testid="home-refine-model" triggerClassName={DETAIL_MENU} />
                {engines.refine_enabled && builtinLlm && llmUsage.tone !== "pending" ? (
                  <UsageInline
                    usage={llmUsage}
                    prefix="· "
                    className="ml-0.5"
                    data-testid="home-refine-usage"
                  />
                ) : (
                  engines.refine_enabled &&
                  textTarget !== undefined && (
                    <span className="truncate" data-testid="home-privacy">
                      {t("home.engine.privacyText", { target: textTarget })}
                    </span>
                  )
                )}
              </div>
            </div>
            <span className="mt-1 shrink-0" data-testid="home-refine-toggle">
              <Toggle
                checked={engines.refine_enabled}
                onChange={setRefine}
                ariaLabel={
                  engines.refine_enabled
                    ? t("home.engine.refineOn", { model: shortModel(engines.refine_model) })
                    : t("home.engine.refineOff")
                }
              />
            </span>
          </div>
          <div className="mt-auto flex min-w-0 items-center gap-2.5 pt-3">
            <Icon name="mic" size={16} className="shrink-0 text-fg-muted" />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-1 text-[13px] text-fg">
                <MicrophoneMenu
                  data-testid="home-mic-device"
                  triggerClassName="-mx-1 inline-flex min-w-0 items-center gap-1 rounded-control px-1 text-[13px] text-fg transition-colors hover:bg-inset"
                  trigger={
                    <ReadoutTrigger
                      value={micDevice?.name ?? meter.error ?? t("home.mic.enumerating")}
                    />
                  }
                />
                {micDevice?.sample_rate_hz ? (
                  <span className="shrink-0 text-fg-muted">
                    · {micDevice.sample_rate_hz / 1000} kHz
                  </span>
                ) : null}
              </div>
              {meter.missing && (
                <div className="truncate text-[11px] text-warning" data-testid="home-mic-missing">
                  {t("home.mic.missing")}
                </div>
              )}
            </div>
            {!dictation.listening && (
              <Button
                size="sm"
                variant="outline"
                icon={micTest.testing ? "stop" : undefined}
                disabled={meter.error !== undefined || dictation.processing}
                data-testid="home-mic-test"
                onClick={micTest.testing ? micTest.stop : micTest.start}>
                {micTest.testing ? t("home.mic.stopTest") : t("home.mic.test")}
              </Button>
            )}
          </div>
        </Card>
      </div>

      <UsageNotices
        openSpeech={openEngines}
        openLocalModels={openLocalModels}
        openAi={() => {
          navigate(AI_ROUTE);
        }}
      />

      <div className="grid grid-cols-2 gap-4 xl:grid-cols-4" data-testid="home-tiles">
        {tiles.map((tile) => (
          // Each span in its own hue, the total an ink card lit by the accent (user decision
          // 2026-10-10, Penpot 「Voltip 统计卡」 page 02, option C with total ①; tokens.css
          // `stat-tile-*`): the themes and the chosen accent decide the colours.
          <div
            key={tile.filter}
            role="button"
            tabIndex={0}
            aria-label={`${tile.eyebrow} ${t("home.tiles.saved", { time: formatDuration(tile.savedMs, locale) })}`}
            data-span={tile.filter}
            onClick={() => {
              openHistory(tile.filter);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") openHistory(tile.filter);
            }}
            className={cx(
              "relative flex min-h-[100px] cursor-pointer flex-col justify-between gap-2 overflow-hidden rounded-[20px] px-4 py-3.5 shadow-panel transition-shadow hover:shadow-pop",
              tile.filter === "all"
                ? "stat-tile-total"
                : `stat-tile ${TILE_HUE[tile.filter] ?? ""}`,
            )}>
            <span className="flex items-center justify-between gap-2">
              <span className="inline-flex min-w-0 items-center gap-2.5">
                <span
                  aria-hidden
                  data-testid="home-tile-icon"
                  className="grid size-[30px] shrink-0 place-items-center rounded-[9px] bg-[var(--tile-soft)] text-[var(--tile-ink)]">
                  <Icon name={tile.icon} size={16} />
                </span>
                <span className="truncate text-[14px] font-medium text-[var(--tile-fg)]">
                  {tile.eyebrow}
                </span>
              </span>
              <span
                className="truncate text-[11px] text-[var(--tile-subtle)]"
                data-testid="home-tile-count">
                {t("home.session.count", { n: formatCount(tile.count) })}
              </span>
            </span>
            <span className="flex items-end justify-between gap-2">
              <span
                className="mono text-[26px] leading-none font-semibold whitespace-nowrap text-[var(--tile-fg)]"
                data-testid="home-tile-value">
                <Duration ms={tile.savedMs} unitClassName="text-[13px] text-[var(--tile-subtle)]" />
              </span>
              <span
                className="flex shrink-0 flex-col items-end text-right"
                data-testid="home-tile-caption">
                <span className="text-[11px] text-[var(--tile-subtle)]">
                  {t("home.session.saved")}
                </span>
                <span
                  className="text-[12px] font-medium text-[var(--tile-muted)]"
                  title={tile.secondary}
                  data-testid="home-tile-note">
                  {tile.secondary}
                </span>
              </span>
            </span>
          </div>
        ))}
      </div>

      {/* docs/dictation.md §26: the recordings no text came from, for 重新识别 in place. */}
      <HomeRecordings />

      <Card padding="none" className="px-2 pt-4 pb-2" data-testid="home-recent">
        <div className="flex items-center justify-between px-4">
          <h2 className="text-[14px] font-medium text-fg">{t("home.recent.title")}</h2>
          <Button
            variant="text"
            size="sm"
            onClick={() => {
              openHistory();
            }}>
            {t("home.recent.all")}
          </Button>
        </div>
        <div ref={recentRef} className="mt-1" data-testid="home-recent-list">
          {recent.length === 0 ? (
            <EmptyState
              icon="mic"
              title={t("home.recent.emptyTitle")}
              mono={activationHint(activation, hotkey, locale)}
              className="min-h-[160px]">
              {t("home.recent.emptyBody")}
            </EmptyState>
          ) : (
            groupByDay(recent, now, locale).map((group) => (
              <section key={group.day} aria-label={group.day}>
                <h3
                  data-recent-day
                  className="px-4 pt-2 pb-1 text-[12px] font-medium text-fg-subtle">
                  {group.day}
                </h3>
                <ul className="flex flex-col">
                  {group.items.map((r) => {
                    const app = r.app;
                    const meta = [
                      recentTimeLabel(r.at_ms, now, locale),
                      app?.name,
                      formatTakeLength(r.duration_ms, locale),
                      t("history.timing.asr", { n: formatCount(Math.round(r.asr_ms)) }),
                      r.refine_ms === undefined
                        ? undefined
                        : t("history.timing.refine", {
                            n: formatCount(Math.round(r.refine_ms)),
                          }),
                    ]
                      .filter((part): part is string => part !== undefined && part.length > 0)
                      .join(" · ");
                    return (
                      <li
                        key={r.id}
                        data-recent-row
                        className="group relative flex min-h-[60px] items-center gap-3 rounded-row px-4 py-2.5 transition-colors hover:bg-nav-active hover:shadow-row">
                        {/* The row opens the entry in 历史记录; its buttons do their own thing. */}
                        <button
                          type="button"
                          aria-label={t("mobile.recent.openLabel", { text: r.text })}
                          data-testid="home-row-open"
                          onClick={() => {
                            openHistory(r.id);
                          }}
                          className="absolute inset-0 rounded-row"
                        />
                        <span
                          aria-hidden
                          data-testid="home-app"
                          className={cx(
                            "pointer-events-none relative grid size-9 shrink-0 place-items-center rounded-[11px] text-[15px] font-bold",
                            app ? appHue(app.id) : "bg-inset text-fg-muted",
                          )}>
                          {app ? (
                            Array.from(app.name)[0]?.toUpperCase()
                          ) : (
                            <Icon name={r.origin === undefined ? "mic" : "phone"} size={16} />
                          )}
                        </span>
                        <span className="pointer-events-none relative flex min-w-0 flex-1 flex-col">
                          <span className="flex min-w-0 items-center gap-2">
                            {r.kind === "edit" && (
                              // docs/dictation.md §19.5: a voice edit's row is its rewrite, badged.
                              <span
                                className="mono shrink-0 rounded-6 border border-accent px-1 text-[10px] text-accent-text"
                                data-testid="home-edit-badge">
                                {t("history.edit.badge")}
                              </span>
                            )}
                            <span
                              className="truncate text-[14px] font-medium text-fg"
                              data-user-text>
                              {r.text}
                            </span>
                          </span>
                          <span
                            className="truncate text-[12px] text-fg-subtle"
                            data-testid="home-row-meta">
                            {meta}
                          </span>
                        </span>
                        <span className="relative flex shrink-0 items-center gap-1.5">
                          <span className="pointer-events-none hidden text-[12px] text-fg-subtle group-hover:inline">
                            {t("home.recent.details")}
                          </span>
                          {/* Board 01: the actions come up with the pointer or the keyboard focus. */}
                          <span
                            data-testid="home-row-actions"
                            className="flex items-center gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                            <ResultActions entry={r} />
                            <IconButton
                              icon="trash"
                              label={t("common.delete")}
                              onClick={() => {
                                remove(r);
                              }}
                            />
                          </span>
                          <OutcomeChip outcome={r.outcome} />
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))
          )}
        </div>
      </Card>
    </div>
  );
}
