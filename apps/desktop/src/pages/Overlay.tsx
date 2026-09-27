import {
  type DictationPhase,
  type LiveText,
  formatElapsed,
  joinLiveText,
  liveCaptionParts,
  takeFailureText,
  takePhaseLabel,
  viaLabel,
} from "@voltip/shared";
import {
  PILL_STATES,
  Pill,
  type PillLiveCaption,
  type PillState,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useAudioMeter } from "../features/audio/useAudioMeter";
import { useLevelHistory, useTickingNow } from "../features/dictation/useDictation";
import { useOverlayWindowState } from "../features/overlay/useOverlayWindowState";
import { copyWithToast, useShell } from "../app/shell-context";

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

/** Route states the transparent pill window renders without the shell around it. A single sample
 *  pill (`state=listening`) is a `pnpm dev` view (`OverlaySheet`); a release build knows only the
 *  two states the shell loads. */
export function isOverlayWindowState(value: string, dev: boolean = import.meta.env.DEV): boolean {
  return value === BLANK_STATE || value === LIVE_STATE || (dev && isPillState(value));
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
      return "inserted";
    case "failed":
      return "error";
    case "cancelled":
      return "cancel-armed";
  }
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
  const shell = useShell();
  const { t, locale } = useI18n();
  const uiState = useUiState();
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
  const meter = useAudioMeter(listening);
  const levels = useLevelHistory(listening ? meter.frame : undefined, LIVE_BARS);
  const now = useTickingNow(listening);

  if (pill === undefined) {
    return (
      <div className="h-full bg-transparent" data-testid="overlay-window" data-state="blank" />
    );
  }

  const phase = dictation.phase;
  const label = takePhaseLabel(dictation, now, locale);
  // docs/dictation.md §19: a voice edit says so, and says what it replaced.
  const edit = dictation.kind === "edit";
  const copyLive = () => {
    // Only wired when the failed phase kept text (see the Pill's `onCopy` below).
    const text = phase.phase === "failed" ? phase.text : undefined;
    if (text === undefined) return;
    void copyWithToast(shell, text, t("overlay.copied", { n: Array.from(text).length }));
  };
  return (
    // Anchored to the window's top edge: the shell places the window so that this is 24 px above
    // the work area's bottom; if a platform gives the window more height than asked (X11 without a
    // window manager did), the pill still lands where the design puts it instead of below the screen.
    <div
      className="flex h-full items-start justify-center bg-transparent pt-2"
      data-testid="overlay-window"
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
            : phase.phase === "done"
              ? t(
                  phase.via === "paste"
                    ? edit
                      ? "overlay.live.editDonePaste"
                      : "overlay.live.donePaste"
                    : edit
                      ? "overlay.live.editDoneClipboard"
                      : "overlay.live.doneClipboard",
                  {
                    n: phase.chars,
                  },
                )
              : phase.phase === "failed"
                ? t("overlay.live.failed", {
                    reason: takeFailureText({ phase, kind: dictation.kind }, locale),
                  })
                : phase.phase === "cancelled"
                  ? phase.injected_chars > 0
                    ? // §12: live_inject does not take back what it already pasted.
                      t("overlay.live.cancelledKept", { n: phase.injected_chars })
                    : t("overlay.live.cancelled")
                  : undefined
        }
        // The tag names where the audio goes; while refining it is the LLM, while a streaming
        // mode waits for its final text (§12 `finalizing`) it is that stage, next to the preview.
        mode={
          phase.phase === "processing"
            ? phase.stage === "refining"
              ? "LLM"
              : phase.stage === "finalizing"
                ? label.text
                : asrMode
            : phase.phase === "listening"
              ? asrMode
              : undefined
        }
        // docs/dictation.md §18.6: the scene the take runs under, next to the mode tag.
        scene={dictation.context?.scene?.name}
        tag={
          edit
            ? t("overlay.live.editTag")
            : dictation.remote === undefined
              ? undefined
              : t("overlay.live.phoneTag", { name: dictation.remote })
        }
        onCopy={phase.phase === "failed" && phase.text !== undefined ? copyLive : undefined}
      />
    </div>
  );
}
