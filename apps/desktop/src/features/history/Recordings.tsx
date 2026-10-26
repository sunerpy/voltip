import {
  type KeptRecording,
  type Locale,
  type RetakeModels,
  dictationFailureText,
  formatCount,
  formatTakeLength,
  retakingRecording,
  unrecognisedRecordings,
} from "@voltip/shared";
import { Button, Card, IconButton, cx, useBackend, useI18n, useUiState } from "@voltip/ui";
import { type ReactNode, useEffect, useRef } from "react";
import { useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";
import { useTickingNow } from "../dictation/useDictation";
import { RetakeModelPickers, useRetakeModels } from "./RetakeModels";
import { clockLabel, dayLabel, recentTimeLabel } from "./stats";

/** How long the end of a 重新识别 stays in its toast. */
const RETAKE_TOAST_MS = 5000;

/** How many of them the home page lists (user request 2026-10-26: a short area above 最近的结果);
 *  查看全部 leads to the rest on the history page. */
export const HOME_RECORDING_ROWS = 3;

/** 重新识别 (docs/dictation.md §26): `audio_retake`, on the settings' models or on `models` for that
 *  take only (§26.4), and a toast once the take this component asked for ends — the text on the
 *  clipboard, or why there is none. `running` is the recording being recognised again right now
 *  (by anyone), `busy` whether any take runs (the core refuses a second one). */
export function useRetake() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { dictation } = useUiState();
  const asked = useRef<string | undefined>(undefined);
  const phase = dictation.phase;
  useEffect(() => {
    const id = asked.current;
    if (id === undefined || dictation.retake !== true || dictation.recording !== id) return;
    if (phase.phase === "done") {
      asked.current = undefined;
      shell.toast({ message: t("history.recordings.done"), duration: RETAKE_TOAST_MS });
    } else if (phase.phase === "failed") {
      asked.current = undefined;
      shell.toast({
        message: t("history.recordings.failed", { reason: dictationFailureText(phase, locale) }),
        duration: RETAKE_TOAST_MS,
        tone: "danger",
      });
    } else if (phase.phase === "cancelled") {
      asked.current = undefined;
    }
  }, [dictation.retake, dictation.recording, phase, shell, t, locale]);
  return {
    retake: (id: string, models: RetakeModels = {}) => {
      asked.current = id;
      void backend.invoke("audio_retake", { id, ...models });
    },
    running: retakingRecording(dictation),
    busy: phase.phase === "listening" || phase.phase === "processing",
  };
}

/** Why a kept recording gave no text, in words. */
function statusText(
  recording: KeptRecording,
  t: ReturnType<typeof useI18n>["t"],
  locale: Locale,
): string {
  const status = recording.status;
  switch (status.kind) {
    case "failed":
      return t("history.recordings.status.failed", {
        reason: dictationFailureText(
          { phase: "failed", code: status.code, message: status.message },
          locale,
        ),
      });
    case "cancelled":
      return t("history.recordings.status.cancelled");
    case "interrupted":
      return t("history.recordings.status.interrupted");
    case "pending":
    case "recognized":
      return t("history.recordings.status.pending");
  }
}

/** The kept recordings no text came from, with what their rows do: 重新识别 on the chosen models,
 *  删除录音, and 清理 (all of them, the one being recognised again aside, after a confirmation). */
function useUnrecognised() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const retakes = useRetake();
  const choice = useRetakeModels();
  const rows = unrecognisedRecordings(state.audio_cache);
  return {
    rows,
    choice,
    ...retakes,
    retake: (id: string) => {
      retakes.retake(id, choice.models);
    },
    remove: (id: string) => {
      void backend.invoke("audio_cache_delete", { id });
      shell.toast({ message: t("history.recordings.deleted"), duration: 3000 });
    },
    clear: () => {
      const doomed = rows.filter((r) => r.id !== retakes.running);
      if (doomed.length === 0) return;
      shell.confirm({
        title: t("history.recordings.clearTitle"),
        body: t("history.recordings.clearBody", { n: doomed.length }),
        confirmLabel: t("history.recordings.clearConfirm"),
        tone: "danger",
        onConfirm: () => {
          for (const r of doomed) void backend.invoke("audio_cache_delete", { id: r.id });
          shell.toast({
            message: t("history.recordings.cleared", { n: doomed.length }),
            duration: 3000,
          });
        },
      });
    },
  };
}

type Unrecognised = ReturnType<typeof useUnrecognised>;

/** When a recording was made, how long it is, and where. */
function whenText(
  r: KeptRecording,
  now: number,
  t: ReturnType<typeof useI18n>["t"],
  locale: Locale,
): string {
  return t("history.recordings.when", {
    time: `${dayLabel(r.at_ms, now, locale)} ${clockLabel(r.at_ms)}`,
    length: formatTakeLength(r.duration_ms, locale),
  });
}

/** One recording's 重新识别 and 删除录音. */
function RowActions({
  r,
  list,
  testPrefix,
}: {
  r: KeptRecording;
  list: Unrecognised;
  testPrefix: string;
}) {
  const { t } = useI18n();
  const retaking = list.running === r.id;
  return (
    <div className="flex shrink-0 items-center gap-2">
      <Button
        size="sm"
        variant="outline"
        icon="refresh"
        disabled={list.busy}
        data-testid={`${testPrefix}-retake`}
        onClick={() => {
          list.retake(r.id);
        }}>
        {retaking ? t("history.recordings.retaking") : t("history.recordings.retake")}
      </Button>
      <IconButton
        icon="trash"
        label={t("history.recordings.delete")}
        disabled={retaking}
        data-testid={`${testPrefix}-delete`}
        onClick={() => {
          list.remove(r.id);
        }}
      />
    </div>
  );
}

/** The section's heading line: the title, the count, what the next 重新识别 runs on, 清理 and
 *  whatever `extra` the page adds (查看全部 on the home page). */
function Heading({
  list,
  lede,
  extra,
  testPrefix,
}: {
  list: Unrecognised;
  lede?: string;
  extra?: ReactNode;
  testPrefix: string;
}) {
  const { t } = useI18n();
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
      <div className="min-w-0">
        <div className="flex items-baseline gap-2">
          <span className="text-[14px] font-medium text-fg">{t("history.recordings.title")}</span>
          <span className="mono text-[11px] text-fg-muted" data-testid={`${testPrefix}-count`}>
            {t("history.recordings.count", { n: formatCount(list.rows.length) })}
          </span>
        </div>
        {lede !== undefined && <div className="text-[12px] text-fg-muted">{lede}</div>}
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <RetakeModelPickers choice={list.choice} disabled={list.busy} />
        <div className="flex items-center gap-1">
          {extra}
          <Button
            size="sm"
            variant="text"
            icon="trash"
            data-testid={`${testPrefix}-clear`}
            disabled={list.rows.every((r) => r.id === list.running)}
            onClick={list.clear}>
            {t("history.recordings.clear")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** The history page's 未识别的录音 (docs/dictation.md §26): the kept recordings no text came
 *  from, newest first, each with 重新识别 and 删除录音; the models the next 重新识别 runs on and
 *  清理 above them. Nothing when there are none. */
export function UnrecognisedRecordings() {
  const { t, locale } = useI18n();
  const now = useTickingNow(false);
  const list = useUnrecognised();
  if (list.rows.length === 0) return null;
  return (
    <Card padding="none" className="flex flex-col gap-2 px-4 py-3" data-testid="history-recordings">
      <Heading list={list} lede={t("history.recordings.lede")} testPrefix="history-recordings" />
      {/* About three rows, the next peeking out: the rest scroll here, so the log and the detail
          under it keep the window's height (user request 2026-10-26). */}
      <ul
        className="flex max-h-[176px] flex-col divide-y divide-border overflow-y-auto"
        data-testid="history-recordings-list"
        aria-label={t("history.recordings.title")}>
        {list.rows.map((r) => (
          <li
            key={r.id}
            data-testid="history-recording"
            data-recording-id={r.id}
            className="flex flex-wrap items-center gap-x-4 gap-y-2 py-2">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] text-fg">
                {whenText(r, now, t, locale)}
                {r.app !== undefined && (
                  <>
                    {" · "}
                    <span data-user-text>{t("history.recordings.app", { app: r.app.name })}</span>
                  </>
                )}
              </div>
              <div
                className={`break-words text-[12px] ${r.status.kind === "failed" ? "text-danger" : "text-fg-muted"}`}
                data-testid="history-recording-status">
                {statusText(r, t, locale)}
              </div>
            </div>
            <RowActions r={r} list={list} testPrefix="history-recording" />
          </li>
        ))}
      </ul>
    </Card>
  );
}

/** The home page's 未识别的录音 (user request 2026-10-26): above 最近的结果, one line a recording
 *  and no more than `HOME_RECORDING_ROWS` of them, with the same choice of models, 清理, and
 *  查看全部 for the history page. Nothing when there are none. */
export function HomeRecordings() {
  const { t, locale } = useI18n();
  const { navigate } = useRouter();
  const now = useTickingNow(false);
  const list = useUnrecognised();
  if (list.rows.length === 0) return null;
  const shown = list.rows.slice(0, HOME_RECORDING_ROWS);
  return (
    <Card
      padding="none"
      className="flex flex-col gap-1.5 px-4 py-2.5"
      data-testid="home-recordings">
      <Heading
        list={list}
        testPrefix="home-recordings"
        extra={
          list.rows.length > shown.length ? (
            <Button
              size="sm"
              variant="text"
              data-testid="home-recordings-all"
              onClick={() => {
                navigate({ name: "history" });
              }}>
              {t("history.recordings.viewAll", { n: formatCount(list.rows.length) })}
            </Button>
          ) : undefined
        }
      />
      <ul
        className="flex flex-col divide-y divide-border"
        aria-label={t("history.recordings.title")}>
        {shown.map((r) => {
          // The short time of the recent results under it (14:32:07, 昨天 18:21).
          const when = t("history.recordings.when", {
            time: recentTimeLabel(r.at_ms, now, locale),
            length: formatTakeLength(r.duration_ms, locale),
          });
          const status = statusText(r, t, locale);
          const app =
            r.app === undefined ? undefined : t("history.recordings.app", { app: r.app.name });
          return (
            <li
              key={r.id}
              data-testid="home-recording"
              data-recording-id={r.id}
              className="flex min-h-10 items-center gap-3 py-1">
              <span
                className="min-w-0 flex-1 truncate text-[13px] text-fg"
                title={[when, app, status].filter((s) => s !== undefined).join(" · ")}>
                {when}
                {app !== undefined && (
                  <>
                    {" · "}
                    <span data-user-text>{app}</span>
                  </>
                )}
                {" · "}
                <span
                  className={cx(r.status.kind === "failed" ? "text-danger" : "text-fg-muted")}
                  data-testid="home-recording-status">
                  {status}
                </span>
              </span>
              <RowActions r={r} list={list} testPrefix="home-recording" />
            </li>
          );
        })}
      </ul>
    </Card>
  );
}
