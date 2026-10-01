import { type HistoryHits, hitTotals } from "@voltip/shared";
import { useEffect, useMemo, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";

/** How often each dictionary entry (`corrections`) or rule (`rules`) fired over the whole
 *  history: the core's `history_hits` (docs/dictation.md §16.3), asked again whenever the history
 *  changes (a new take, a deletion, clearing). Empty until the first answer. */
export function useHitTotals(kind: "corrections" | "rules"): Map<string, number> {
  const { backend } = useBackend();
  const { history_total: total, history_recent: recent } = useUiState();
  const newest = recent[0]?.id;
  const [hits, setHits] = useState<HistoryHits | undefined>(undefined);
  useEffect(() => {
    let live = true;
    backend.historyHits().then(
      (answer) => {
        if (live) setHits(answer);
      },
      () => {
        if (live) setHits(undefined);
      },
    );
    return () => {
      live = false;
    };
    // A new take, a deletion or clearing (the total or the newest entry changes) is a reason to
    // ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, total, newest]);
  return useMemo(() => hitTotals(hits, kind), [hits, kind]);
}
