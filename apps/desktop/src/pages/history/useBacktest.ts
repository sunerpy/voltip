import { type BacktestDraft, type VocabularyBacktest, errorText } from "@voltip/shared";
import { useBackend, useUiState } from "@voltip/ui";
import { useEffect, useRef, useState } from "react";

/** Quiet time after the last keystroke before the core is asked again (§16.9 3.3). */
export const BACKTEST_DEBOUNCE_MS = 200;

/** Where the card's effect line is: nothing to ask, being asked, answered, or refused. */
export type BacktestState =
  | { kind: "idle" }
  | { kind: "computing" }
  | { kind: "ok"; backtest: VocabularyBacktest }
  | { kind: "error"; message: string };

const IDLE: BacktestState = { kind: "idle" };
const COMPUTING: BacktestState = { kind: "computing" };

/** What `draft` would change in the recent takes, leaving out `historyId` when given
 *  (`vocabulary_backtest`, docs/dictation.md §16.9; the dictionary's suggestions leave none out,
 *  §16.10): asked 200 ms after the draft last changed, and again when the dictionary or the history
 *  changes; `computing` until the answer for this draft is in. `undefined` asks nothing. */
export function useBacktest(draft: BacktestDraft | undefined, historyId?: string): BacktestState {
  const { backend } = useBackend();
  const { dictionary, history_total: total } = useUiState();
  const [answer, setAnswer] = useState<{ key: string; state: BacktestState } | undefined>();
  // The draft by value: the card builds a new object on every render.
  const key = draft === undefined ? undefined : JSON.stringify(draft);
  const latest = useRef(draft);
  useEffect(() => {
    latest.current = draft;
  });
  useEffect(() => {
    if (key === undefined) return undefined;
    let alive = true;
    const handle = setTimeout(() => {
      const asked = latest.current;
      if (asked === undefined) return;
      backend.vocabularyBacktest({ draft: asked, historyId }).then(
        (backtest) => {
          if (alive) setAnswer({ key, state: { kind: "ok", backtest } });
        },
        (e: unknown) => {
          if (alive) setAnswer({ key, state: { kind: "error", message: errorText(e) } });
        },
      );
    }, BACKTEST_DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(handle);
    };
    // A changed dictionary or history is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, key, historyId, dictionary, total]);
  if (key === undefined) return IDLE;
  return answer?.key === key ? answer.state : COMPUTING;
}
