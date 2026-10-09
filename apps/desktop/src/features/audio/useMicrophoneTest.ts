import { useUiState } from "@voltip/ui";
import { useCallback, useEffect, useState } from "react";

/** How long one 测试麦克风 run keeps the microphone open before it closes by itself. */
export const MIC_TEST_MS = 15_000;
/** How often the countdown refreshes. */
const TICK_MS = 250;

export interface MicrophoneTest {
  testing: boolean;
  /** Whole seconds left while testing (0 otherwise). */
  remaining: number;
  start: () => void;
  stop: () => void;
}

/** A 测试麦克风 run: the caller meters the microphone while `testing` (user feedback 2026-09-28:
 *  nothing listens to the microphone while idle), for `durationMs` or until `stop`. */
export function useMicrophoneTest(durationMs: number = MIC_TEST_MS): MicrophoneTest {
  const [endsAt, setEndsAt] = useState<number | undefined>(undefined);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (endsAt === undefined) return undefined;
    const timer = setInterval(() => {
      const t = Date.now();
      setNow(t);
      if (t >= endsAt) setEndsAt(undefined);
    }, TICK_MS);
    return () => {
      clearInterval(timer);
    };
  }, [endsAt]);
  const start = useCallback(() => {
    const t = Date.now();
    setNow(t);
    setEndsAt(t + durationMs);
  }, [durationMs]);
  const stop = useCallback(() => {
    setEndsAt(undefined);
  }, []);
  const testing = endsAt !== undefined;
  return {
    testing,
    remaining: testing ? Math.max(0, Math.ceil((endsAt - now) / 1000)) : 0,
    start,
    stop,
  };
}

/** The microphone the settings choose (`settings.microphone`), `undefined` for the default input. */
export function useChosenMicrophone(): string | undefined {
  return useUiState().settings.microphone ?? undefined;
}
