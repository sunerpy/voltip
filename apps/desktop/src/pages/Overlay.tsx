import {
  type DictationPhase,
  type DictationStatus,
  type LiveText,
  type PasteOutcome,
  type TFunction,
  formatElapsed,
  formatSeconds,
  joinLiveText,
  liveCaptionParts,
  presetRefLabel,
  recordingSourceLabel,
  sceneLabel,
  segmentsDoneLabel,
  takeFailureText,
  takePhaseLabel,
  viaLabel,
  notRefinedText,
} from "@voltip/shared";
import {
  PILL_STATES,
  Pill,
  floatingTheme,
  type PillAction,
  type PillLiveCaption,
  type PillState,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useAudioMeter } from "../features/audio/useAudioMeter";
import { useChosenMicrophone } from "../features/audio/useMicrophoneTest";
import { useLevelHistory, useStageNow, useTickingNow } from "../features/dictation/useDictation";
import { useOverlayWindowState } from "../features/overlay/useOverlayWindowState";
import { useAppearance } from "../app/appearance";
import { pasteToast } from "../features/history/paste";

export interface OverlayProps {
  /** The pill window's route state: `live` follows the core's `state.dictation` (the real pill
   *  window), `blank` paints nothing (the prewarmed window). */
  state?: string;
}

export function isPillState(value: string): value is PillState {
  return (PILL_STATES as readonly string[]).includes(value);
}

/** The shell prewarms the pill window on this state: it paints nothing at all. */
export const BLANK_STATE = "blank";
/** The pill follows `state.dictation` (docs/dictation.md §5): idle paints nothing. */
export const LIVE_STATE = "live";
/** The shell's flash after 复制上一条结果 (docs/dictation.md §28), between takes. */
export const COPIED_STATE = "copied";

/** Route states the transparent pill window renders without the shell around it. A single sample
 *  pill (`state=listening`) is a `pnpm dev` view (`OverlaySheet`); a release build knows only the
 *  two states the shell loads. */
export function isOverlayWindowState(value: string, dev: boolean = import.meta.env.DEV): boolean {
  return (
    value === BLANK_STATE ||
    value === LIVE_STATE ||
    value === COPIED_STATE ||
    (dev && isPillState(value))
  );
}

/** The result cards (docs/dictation.md §28, D11): 没有写进去, 窗口已切换, a failure that kept its
 *  text, and a failure without text (but speech) whose recording is kept — the terminal states
 *  the core keeps on screen until 关闭 or the next take. */
export type ResultCard = "not_written" | "window_changed" | "failed_text" | "failed_kept";

export function resultCardFor(dictation: DictationStatus): ResultCard | undefined {
  const phase = dictation.phase;
  if (phase.phase === "done") {
    if (phase.check === "not_written") return "not_written";
    return phase.clipboard_code === "target_changed" ? "window_changed" : undefined;
  }
  if (phase.phase !== "failed") return undefined;
  if (phase.text !== undefined) return "failed_text";
  return dictation.recording !== undefined && phase.code !== "no_speech"
    ? "failed_kept"
    : undefined;
}

/** What the card's 粘贴 says when it could not paste (the card stays); the history's words. */
function pasteFeedback(outcome: PasteOutcome, t: TFunction): string | undefined {
  return outcome.kind === "pasted" ? undefined : pasteToast(outcome, t).message;
}

/** `DictationPhase` → pill state; `undefined` while idle (nothing to paint). */
export function pillStateFor(phase: DictationPhase): PillState | undefined {
  switch (phase.phase) {
    case "idle":
      return undefined;
    case "listening":
      return "listening";
    case "processing":
      return "processing";
    case "done":
      // docs/dictation.md §28: a paste the field never received is no success.
      return phase.check === "not_written" ? "error" : "inserted";
    case "failed":
      return "error";
    case "cancelled":
      return "cancel-armed";
  }
}

/** When the processing step began: `stage_started_at`, or the run's start from a core that does
 *  not send it. */
export function stageStart(phase: Extract<DictationPhase, { phase: "processing" }>): number {
  return phase.stage_started_at !== undefined && phase.stage_started_at > 0
    ? phase.stage_started_at
    : phase.started_at;
}

/** Waveform bars of the listening pill (`Pill` draws 36). */
const LIVE_BARS = 36;

/** The pill's caption from the core's live preview; `undefined` until any text arrived (a degraded
 *  preview keeps showing what it had — the pill itself says nothing about it). Under `live_inject`
 *  (docs/dictation.md §12) the first `injected` committed sentences are already in the front app
 *  and come back as the fainter `injected` part. */
export function pillLiveCaption(live: LiveText | undefined): PillLiveCaption | undefined {
  if (live === undefined) return undefined;
  const parts = liveCaptionParts(live);
  if (parts.committed.length === 0 && parts.current.length === 0) return undefined;
  if (live.injected <= 0) return parts;
  return {
    injected: joinLiveText(live.committed.slice(0, live.injected).map((s) => s.text)),
    committed: joinLiveText(live.committed.slice(live.injected).map((s) => s.text)),
    current: parts.current,
  };
}

/** The transparent pill window. The shell loads it on `state=live` and only shows / hides it; the
 *  pill itself follows `state.dictation`, the meter channel and the live preview. */
export function Overlay({ state }: OverlayProps) {
  const pillTheme = floatingTheme(useAppearance().resolvedTheme);
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const uiState = useUiState();
  // docs/dictation.md §28: what the card's last button did, for the take it was pressed on.
  const [feedback, setFeedback] = useState<{ session: number; text: string } | undefined>();
  const dictation = uiState.dictation;
  // The pill's mode tag names where the audio goes: a remote provider or the local model.
  const asrMode =
    uiState.engines.asr_provider === "local" ? t("overlay.live.local") : t("overlay.live.cloud");
  // The shell drives the pill window through `voltip://overlay` events; the route's `state` is only
  // the first frame. `blank` paints nothing (prewarmed window, and the frame before hiding).
  const current = useOverlayWindowState(state);
  const pill = current === LIVE_STATE ? pillStateFor(dictation.phase) : undefined;
  const listening = pill !== undefined && dictation.phase.phase === "listening";
  // While the recorder is open the core broadcasts its levels through the same meter channel.
  const meter = useAudioMeter(listening, useChosenMicrophone());
  const levels = useLevelHistory(listening ? meter.frame : undefined, LIVE_BARS);
  const now = useTickingNow(listening);
  const stageNow = useStageNow(pill !== undefined && dictation.phase.phase === "processing");

  if (current === COPIED_STATE) {
    return (
      <div
        className="flex h-full items-start justify-center bg-transparent pt-2"
        data-testid="overlay-window"
        data-theme={pillTheme}
        data-state={COPIED_STATE}>
        <Pill state="inserted" label={t("insertCheck.copiedLast")} />
      </div>
    );
  }
  if (pill === undefined) {
    return (
      <div className="h-full bg-transparent" data-testid="overlay-window" data-state="blank" />
    );
  }

  const phase = dictation.phase;
  const label = takePhaseLabel(dictation, now, locale);
  // docs/dictation.md §19: a voice edit says so, and says what it replaced.
  const edit = dictation.kind === "edit";
  // docs/dictation.md §26: a take that gave no text whose recording the cache holds can be
  // recognised again from the history.
  const kept =
    dictation.recording !== undefined &&
    uiState.audio_cache.recordings.some((r) => r.id === dictation.recording);
  // docs/dictation.md §27: the built-in service's day ran out — the pill says so in those words.
  const builtinAsrOut =
    phase.phase === "failed" &&
    phase.code === "quota" &&
    uiState.engines.asr_provider === "builtin" &&
    uiState.usage.builtin;
  const builtinRefineOut =
    phase.phase === "done" &&
    !phase.refined &&
    phase.refine_failure === "quota" &&
    uiState.engines.llm_provider === "builtin" &&
    uiState.usage.builtin;
  // docs/dictation.md §28: a result card's sentence, buttons and 关闭.
  const card = resultCardFor(dictation);
  const session = dictation.session;
  const say = (text: string | undefined) => {
    if (text !== undefined) setFeedback({ session, text });
  };
  // Bound to this card's take: a 粘贴 that answers after the next take has put up its own card
  // must not close that one.
  const dismiss = () => {
    void backend.invoke("dictation_dismiss", { session });
  };
  const cardText =
    phase.phase === "done" ? phase.text : phase.phase === "failed" ? phase.text : undefined;
  const textActions = (text: string): PillAction[] => [
    {
      label: t("insertCheck.card.paste"),
      primary: true,
      onClick: () => {
        void backend.overlayPaste(text).then(
          (outcome) => {
            if (outcome.kind === "pasted") dismiss();
            else say(pasteFeedback(outcome, t));
          },
          () => {
            // A command that fails outright reads like one that did not finish, as in the history.
            say(pasteFeedback({ kind: "failed", reason: "timeout" }, t));
          },
        );
      },
    },
    {
      label: t("insertCheck.card.copy"),
      onClick: () => {
        void backend.overlayCopy(text).then(
          () => {
            say(t("insertCheck.card.copied"));
          },
          () => {
            say(t("insertCheck.card.copyFailed"));
          },
        );
      },
    },
  ];
  const recording = dictation.recording;
  const actions: PillAction[] | undefined =
    card === undefined
      ? undefined
      : card === "failed_kept"
        ? [
            ...(kept && recording !== undefined
              ? [
                  {
                    label: t("insertCheck.card.retake"),
                    primary: true,
                    onClick: () => {
                      void backend.invoke("audio_retake", { id: recording });
                    },
                  },
                ]
              : []),
            {
              label: t("insertCheck.card.openHistory"),
              onClick: () => {
                void backend.openHistory();
              },
            },
          ]
        : cardText === undefined
          ? []
          : textActions(cardText);
  const failure =
    phase.phase === "failed"
      ? builtinAsrOut
        ? t("usage.pill.asrOut")
        : takeFailureText({ phase, kind: dictation.kind, source: dictation.source }, locale)
      : undefined;
  return (
    // Anchored to the window's top edge: the shell places the window so that this is 24 px above
    // the work area's bottom; if a platform gives the window more height than asked (X11 without a
    // window manager did), the pill still lands where the design puts it instead of below the screen.
    <div
      className="flex h-full items-start justify-center bg-transparent pt-2"
      data-testid="overlay-window"
      // In 澄空 the pill is 夜空's dark glass, legible over any window (board 02).
      data-theme={pillTheme}
      data-session={dictation.session}>
      <Pill
        state={pill}
        levels={levels}
        // The timer counts from the device's first samples (docs/dictation.md §11 `ready`).
        readout={
          phase.phase === "listening"
            ? phase.ready
              ? formatElapsed(now - phase.started_at)
              : "00:00"
            : phase.phase === "processing"
              ? formatSeconds(stageNow - stageStart(phase))
              : phase.phase === "done"
                ? viaLabel(phase.via, locale)
                : undefined
        }
        waiting={phase.phase === "listening" && !phase.ready}
        // docs/dictation.md §13: a short press under hold_or_toggle locked the take.
        locked={phase.phase === "listening" && phase.locked}
        live={phase.phase === "listening" ? pillLiveCaption(phase.live) : undefined}
        preview={phase.phase === "processing" ? phase.preview : undefined}
        label={
          phase.phase === "processing"
            ? label.text
            : card === "not_written"
              ? t("insertCheck.notWritten")
              : card === "failed_kept" && failure !== undefined
                ? t("insertCheck.failedKept", { reason: failure })
                : phase.phase === "done"
                  ? t(
                      phase.via === "paste"
                        ? edit
                          ? "overlay.live.editDonePaste"
                          : "overlay.live.donePaste"
                        : // docs/dictation.md §14.3: another window came to the front meanwhile.
                          phase.clipboard_code === "target_changed"
                          ? edit
                            ? "overlay.live.editDoneTargetChanged"
                            : "overlay.live.doneTargetChanged"
                          : edit
                            ? "overlay.live.editDoneClipboard"
                            : "overlay.live.doneClipboard",
                      {
                        n: phase.chars,
                      },
                    )
                  : builtinAsrOut
                    ? t("usage.pill.asrOut")
                    : phase.phase === "failed"
                      ? t("overlay.live.failed", {
                          reason: takeFailureText(
                            { phase, kind: dictation.kind, source: dictation.source },
                            locale,
                          ),
                        })
                      : phase.phase === "cancelled"
                        ? phase.injected_chars > 0
                          ? // §12: live_inject does not take back what it already pasted.
                            t("overlay.live.cancelledKept", { n: phase.injected_chars })
                          : kept
                            ? t("overlay.live.cancelledRecording")
                            : t("overlay.live.cancelled")
                        : undefined
        }
        // The tag names where the audio goes; while refining it is the preset the clean-up runs
        // with (§21; AI 润色 when the status names none), while a streaming mode waits for its
        // final text (§12 `finalizing`) it is that stage, next to the preview.
        mode={
          phase.phase === "processing"
            ? phase.stage === "refining"
              ? dictation.preset === undefined
                ? t("overlay.live.refineTag")
                : presetRefLabel(dictation.preset, locale)
              : phase.stage === "finalizing"
                ? label.text
                : asrMode
            : phase.phase === "listening"
              ? asrMode
              : undefined
        }
        // docs/dictation.md §18.6: the scene the take runs under, next to the mode tag.
        scene={
          dictation.context?.scene === undefined
            ? undefined
            : sceneLabel(dictation.context.scene, locale)
        }
        tag={
          edit
            ? t("overlay.live.editTag")
            : dictation.remote === undefined
              ? undefined
              : t("overlay.live.phoneTag", { name: dictation.remote })
        }
        // docs/dictation.md §22: what the take records, and how far a long take's recognition got
        // while it records (the processing line counts on its own, `takePhaseLabel`).
        source={
          dictation.source === undefined
            ? undefined
            : recordingSourceLabel(dictation.source, locale)
        }
        progress={
          phase.phase === "listening" && dictation.segments !== undefined
            ? segmentsDoneLabel(dictation.segments, locale)
            : undefined
        }
        // docs/dictation.md §3.6: a take whose polish was asked for and not used says why.
        // §26: a failed take whose recording is kept says where to recognise it again (a card
        // says it in its sentence, §28).
        note={
          builtinRefineOut
            ? t("usage.pill.refineOut")
            : phase.phase === "done" && !phase.refined && phase.refine_failure !== undefined
              ? notRefinedText(phase.refine_failure, locale)
              : phase.phase === "failed" && phase.text === undefined && kept && card === undefined
                ? builtinAsrOut
                  ? t("usage.pill.kept")
                  : t("overlay.live.keptNote")
                : undefined
        }
        // docs/dictation.md §28: a result card's buttons, what the last one did, and 关闭.
        actions={actions}
        feedback={feedback?.session === session ? feedback.text : undefined}
        onDismiss={card === undefined ? undefined : dismiss}
      />
    </div>
  );
}
