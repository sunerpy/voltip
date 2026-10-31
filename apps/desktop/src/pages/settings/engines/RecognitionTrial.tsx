import {
  type TFunction,
  type TrialResult,
  type UiState,
  dictationFailureText,
  formatMs,
  formatTakeLength,
  notRefinedText,
  servedByText,
} from "@voltip/shared";
import { Button, useBackend, useI18n, useUiState } from "@voltip/ui";

/** Why no recording can be tried, or nothing when the newest kept WAV can: the history and the
 *  cache keep recordings only together, and a long take (kept as Opus parts) is not tried. */
function blockedReason(state: UiState, t: TFunction): string | undefined {
  if (!state.settings.history.enabled) return t("engines.trial.historyOff");
  if (!state.settings.audio_cache.enabled) return t("engines.trial.cacheOff");
  if (!state.audio_cache.recordings.some((r) => r.format !== "opus"))
    return t("engines.trial.noRecording");
  return undefined;
}

/** 试识别 (docs/dictation.md §30.5), under the recognition providers: the newest kept recording of
 *  two minutes at most, recognised (and cleaned up when the settings say so) with the saved
 *  settings. The text, who wrote it, how long it took and whether the glossary went out are shown
 *  here only: nothing is pasted, copied or written to the history. The button is off, and says
 *  why, while no recording can be tried. Not shown where the app keeps no recordings (the phone). */
export function RecognitionTrial() {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  if (!state.audio_cache.available) return null;
  const blocked = blockedReason(state, t);
  const newest = state.audio_cache.recordings.find((r) => r.format !== "opus");
  const trial = state.recognition_trial;
  const running = trial?.state === "running";
  return (
    <section
      className="flex flex-col gap-2 rounded-10 bg-inset p-3"
      aria-label={t("engines.trial.title")}
      data-testid="recognition-trial">
      <div className="flex items-center justify-between gap-3">
        <span className="text-[13px] font-medium text-fg">{t("engines.trial.title")}</span>
        <Button
          size="sm"
          variant="outline"
          icon="play"
          disabled={blocked !== undefined || running}
          loading={running}
          onClick={() => {
            void backend.invoke("recognition_trial");
          }}>
          {running ? t("engines.trial.running") : t("engines.trial.run")}
        </Button>
      </div>
      <p className="text-[12px] text-fg-muted">{blocked ?? t("engines.trial.help")}</p>
      {newest !== undefined && blocked === undefined && (
        <span className="mono text-[11px] text-fg-subtle">
          {t("engines.trial.recording", { length: formatTakeLength(newest.duration_ms, locale) })}
        </span>
      )}
      {trial?.state === "done" && <TrialDone result={trial} />}
      {trial?.state === "failed" && (
        <p className="text-[12px] text-danger" role="alert" data-testid="recognition-trial-error">
          {trial.failure.kind === "no_recording"
            ? t("engines.trial.noRecording")
            : dictationFailureText(
                { phase: "failed", code: trial.failure.code, message: trial.failure.message },
                locale,
              )}
        </p>
      )}
    </section>
  );
}

/** A trial's text and the facts behind it. */
function TrialDone({ result }: { result: TrialResult }) {
  const { t, locale } = useI18n();
  const refine = result.refined
    ? [
        result.refine_provider === undefined
          ? undefined
          : servedByText(result.refine_provider, undefined, locale),
        result.refine_model,
        formatMs(result.refine_ms),
      ]
        .filter((part) => part !== undefined)
        .join(" · ")
    : result.refine_failure === undefined
      ? t("engines.trial.refineOff")
      : notRefinedText(result.refine_failure, locale);
  const rows: [string, string][] = [
    [
      t("engines.trial.provider"),
      `${servedByText(result.asr_provider, result.asr_fallback, locale)} · ${result.asr_model}`,
    ],
    [t("engines.trial.asrTime"), formatMs(result.asr_ms)],
    [
      t("engines.trial.glossary"),
      t(result.glossary_sent ? "history.detail.glossary.sent" : "history.detail.glossary.notSent"),
    ],
    [t("engines.trial.refine"), refine],
    [t("engines.trial.total"), formatMs(result.elapsed_ms)],
  ];
  return (
    <div className="flex flex-col gap-2" data-testid="recognition-trial-result">
      <div
        className="rounded-6 bg-surface p-3 text-[13px] leading-5 whitespace-pre-wrap text-fg hairline"
        data-user-text>
        {result.text}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[12px]">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-fg-subtle">{label}</dt>
            <dd className="mono text-fg">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
