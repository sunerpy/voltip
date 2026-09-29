import { useBackend, useUiState } from "@voltip/ui";
import { useEffect, useState } from "react";
import { type HomeStats, emptyHomeStats, homeStats, startOfDay, statsBoundaries } from "./stats";

/** The home page's numbers: the core's `history_stats` for the local midnights of the last six
 *  weeks (docs/dictation.md §4.5), asked again whenever the history changes (a new take, a
 *  deletion, clearing) or the day turns. The last answer stays on screen while the next is on its
 *  way; zeros before the first. */
export function useHomeStats(now: number): HomeStats {
  const { backend } = useBackend();
  const { history_total: total, history_recent: recent } = useUiState();
  const newest = recent[0]?.id;
  const day = startOfDay(now);
  const [stats, setStats] = useState<HomeStats>(emptyHomeStats);
  useEffect(() => {
    let live = true;
    const boundaries = statsBoundaries(day);
    backend.historyStats(boundaries).then(
      (answer) => {
        if (live) setStats(homeStats(answer, boundaries, day));
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
    // A new take, a deletion or clearing (the total or the newest entry changes) is a reason to
    // ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, total, newest, day]);
  return stats;
}
