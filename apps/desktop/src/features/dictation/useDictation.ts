import type { DictationPhase, DictationStatus, LevelFrame } from "@voltip/shared";
import { useBackend, useNow, useUiState } from "@voltip/ui";
import { useCallback, useState, useSyncExternalStore } from "react";
import { levelFraction } from "../audio/useAudioMeter";

export interface DictationControls {
  status: DictationStatus;
  phase: DictationPhase;
  /** The recorder is open (`listening`). */
  listening: boolean;
  /** ASR / refine / injection are running (`processing`). */
  processing: boolean;
  /** `dictation_start`: a new session; interrupts any done / failed dwell. */
  start: () => void;
  /** `dictation_stop`: closes the recorder and runs the pipeline. */
  stop: () => void;
  /** `dictation_cancel`: discards the recording. */
  cancel: () => void;
  /** Start while idle, stop while listening (the home button and the palette entry). */
  toggle: () => void;
}

/** The dictation state machine as the core reports it (`state.dictation`), plus the three commands.
 *  Nothing here simulates anything: every transition comes back through `voltip://event`. */
export function useDictation(): DictationControls {
  const { backend } = useBackend();
  const status = useUiState().dictation;
  const phase = status.phase;
  const listening = phase.phase === "listening";
  const processing = phase.phase === "processing";
  const start = useCallback(() => {
    void backend.invoke("dictation_start");
  }, [backend]);
  const stop = useCallback(() => {
    void backend.invoke("dictation_stop");
  }, [backend]);
  const cancel = useCallback(() => {
    void backend.invoke("dictation_cancel");
  }, [backend]);
  const toggle = useCallback(() => {
    if (listening) stop();
    else if (!processing) start();
  }, [listening, processing, start, stop]);
  return { status, phase, listening, processing, start, stop, cancel, toggle };
}

interface ClockStore {
  subscribe: (listener: () => void) => () => void;
  snapshot: () => number;
}

/** A clock as an external store (the same shape as `useNow`): pure during render, ticking every
 *  `intervalMs` only while something is subscribed. */
function clockStore(intervalMs: number): ClockStore {
  const listeners = new Set<() => void>();
  let cached = Date.now();
  let timer: ReturnType<typeof setInterval> | undefined;
  return {
    subscribe(listener) {
      listeners.add(listener);
      if (timer === undefined) {
        cached = Date.now();
        timer = setInterval(() => {
          cached = Date.now();
          for (const l of listeners) l();
        }, intervalMs);
      }
      return () => {
        listeners.delete(listener);
        if (listeners.size === 0 && timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
    snapshot: () => cached,
  };
}

export const TICK_MS = 1000;
/** The processing pill's step time (`0.4 s`) moves ten times a second. */
export const STAGE_TICK_MS = 100;
const secondClock = clockStore(TICK_MS);
const stageClock = clockStore(STAGE_TICK_MS);

function subscribeNothing(): () => void {
  return () => undefined;
}

/** Milliseconds now: a one-second clock while `active` (the `正在录音… 00:03` readout), the shared
 *  30-second clock otherwise (day grouping, statistics). */
export function useTickingNow(active: boolean): number {
  const coarse = useNow() * 1000;
  const fine = useSyncExternalStore(
    active ? secondClock.subscribe : subscribeNothing,
    secondClock.snapshot,
    secondClock.snapshot,
  );
  return active ? fine : coarse;
}

/** Milliseconds now, ten times a second while `active`: the processing pill counts the current
 *  step with it (user feedback 2026-09-29: the step time stood at 0.0 s). */
export function useStageNow(active: boolean): number {
  return useSyncExternalStore(
    active ? stageClock.subscribe : subscribeNothing,
    stageClock.snapshot,
    stageClock.snapshot,
  );
}

/** Keeps the last `bars` level fractions (0..1) so the pill's waveform scrolls with the live
 *  meter; empties as soon as frames stop (recorder closed). */
export function useLevelHistory(frame: LevelFrame | undefined, bars: number): number[] {
  // Derived state adjusted during render (React's documented pattern for reacting to a prop
  // change): each new frame appends one bar; no frame empties the strip.
  const [entry, setEntry] = useState<{ frame: LevelFrame | undefined; levels: number[] }>({
    frame,
    levels: [],
  });
  if (entry.frame === frame) return entry.levels;
  const levels =
    frame === undefined ? [] : [...entry.levels, levelFraction(frame.rms_dbfs)].slice(-bars);
  setEntry({ frame, levels });
  return levels;
}
