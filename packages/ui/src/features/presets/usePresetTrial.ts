import { type PresetId, type PresetTryOutcome, errorText } from "@voltip/shared";
import { useCallback, useEffect, useRef, useState } from "react";
import { useBackend } from "../../backend/BackendProvider";
import { useT } from "../../i18n/I18nProvider";

/** How long the editor waits for a `preset_try` answer before it gives up (the core always
 *  answers, with a failure at the clean-up's own deadline at the latest). */
export const PRESET_TRY_WAIT_MS = 60_000;

/** Request ids of this window's trials; the core echoes them in `preset_try`. */
let lastTrial = 0;

export interface PresetTrial {
  pending: boolean;
  /** The answer to the latest run; cleared by `reset`. */
  outcome: PresetTryOutcome | undefined;
  /** Run `text` through the current clean-up with a saved preset or an instruction being edited. */
  run: (source: { preset: PresetId } | { prompt: string }, text: string) => void;
  reset: () => void;
}

/** 试运行 (`presets_try`): sends the request and listens for the `preset_try` event with its id;
 *  an answer to an earlier run is ignored. */
export function usePresetTrial(): PresetTrial {
  const { backend } = useBackend();
  const t = useT();
  const [pending, setPending] = useState(false);
  const [outcome, setOutcome] = useState<PresetTryOutcome | undefined>(undefined);
  const current = useRef<number | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const settle = useCallback((answer: PresetTryOutcome) => {
    clearTimeout(timer.current);
    current.current = undefined;
    setPending(false);
    setOutcome(answer);
  }, []);

  useEffect(
    () =>
      backend.on((event) => {
        if (event.type === "preset_try" && event.id === current.current) settle(event.outcome);
      }),
    [backend, settle],
  );
  useEffect(
    () => () => {
      clearTimeout(timer.current);
    },
    [],
  );

  const run = useCallback(
    (source: { preset: PresetId } | { prompt: string }, text: string) => {
      lastTrial += 1;
      const id = lastTrial;
      current.current = id;
      setPending(true);
      setOutcome(undefined);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        if (current.current === id)
          settle({ status: "failed", reason: t("presets.editor.trial.timeout") });
      }, PRESET_TRY_WAIT_MS);
      backend
        .invoke("presets_try", {
          id,
          preset: "preset" in source ? source.preset : null,
          prompt: "prompt" in source ? source.prompt : null,
          text,
        })
        .catch((e: unknown) => {
          if (current.current === id) settle({ status: "failed", reason: errorText(e) });
        });
    },
    [backend, settle, t],
  );

  const reset = useCallback(() => {
    clearTimeout(timer.current);
    current.current = undefined;
    setPending(false);
    setOutcome(undefined);
  }, []);

  return { pending, outcome, run, reset };
}
