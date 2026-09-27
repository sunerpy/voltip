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

// A one-second clock as an external store (the same shape as `useNow`): pure during render,
// ticking only while something is subscribed.
export const TICK_MS = 1000;
const tickListeners = new Set<() => void>();
let tickCached = Date.now();
let tickTimer: ReturnType<typeof setInterval> | undefined;

function subscribeTick(listener: () => void): () => void {
  tickListeners.add(listener);
  if (tickTimer === undefined) {
    tickCached = Date.now();
    tickTimer = setInterval(() => {
      tickCached = Date.now();
      for (const l of tickListeners) l();
    }, TICK_MS);
  }
  return () => {
    tickListeners.delete(listener);
    if (tickListeners.size === 0 && tickTimer !== undefined) {
      clearInterval(tickTimer);
      tickTimer = undefined;
    }
  };
}

function tickSnapshot(): number {
  return tickCached;
}

function subscribeNothing(): () => void {
  return () => undefined;
}

/** Milliseconds now: a one-second clock while `active` (the `正在听… 00:03` readout), the shared
 *  30-second clock otherwise (day grouping, statistics). */
export function useTickingNow(active: boolean): number {
  const coarse = useNow() * 1000;
  const fine = useSyncExternalStore(
    active ? subscribeTick : subscribeNothing,
    tickSnapshot,
    tickSnapshot,
  );
  return active ? fine : coarse;
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
