// The onboarding permission step's poller (docs/dictation.md §15.1): while the step is on screen
// it re-reads `permissions_status` every second; three consecutive failed reads stop the loop and
// surface the error until the user asks to check again. The policy mirrors
// `voltip_platform::Poller` (interval and error limit come from the shared schema); the transport
// is whatever `Backend` the app runs on, so the mock backend drives the tests.
import {
  type Backend,
  PERMISSIONS,
  PERMISSION_POLL_INTERVAL_MS,
  PERMISSION_POLL_MAX_ERRORS,
  type Permission,
  type PermissionReport,
} from "@voltip/shared";
import { useCallback, useEffect, useState } from "react";

export interface PermissionsPoll {
  /** The last report that arrived; `undefined` until the first read succeeds. */
  report: PermissionReport | undefined;
  /** Message of the last failed read; cleared by the next success. */
  error: string | undefined;
  /** Failed reads in a row (resets on success and on `recheck`). */
  consecutiveErrors: number;
  /** `true` once the error limit stopped the loop; `recheck` restarts it. */
  stopped: boolean;
  /** Read again now and restart the loop if it had stopped. */
  recheck: () => void;
  /** Ask the OS for one permission, then read again right away. */
  request: (permission: Permission) => Promise<void>;
}

export interface PollState {
  report: PermissionReport | undefined;
  error: string | undefined;
  consecutiveErrors: number;
  stopped: boolean;
}

export const INITIAL_POLL: PollState = {
  report: undefined,
  error: undefined,
  consecutiveErrors: 0,
  stopped: false,
};

function sameReport(a: PermissionReport | undefined, b: PermissionReport): boolean {
  return a !== undefined && a.platform === b.platform && PERMISSIONS.every((p) => a[p] === b[p]);
}

function messageOf(e: unknown): string {
  if (e instanceof Error) return e.message;
  return typeof e === "string" ? e : JSON.stringify(e);
}

/** One read folded into the state: a success resets the streak, an error extends it and stops the
 *  loop at the limit (the `Poller` table on the Rust side). */
export function foldRead(
  prev: PollState,
  read: { ok: true; report: PermissionReport } | { ok: false; error: string },
): PollState {
  if (read.ok) {
    // An unchanged answer keeps the state object, so the page does not re-render every second.
    const settled = prev.error === undefined && prev.consecutiveErrors === 0 && !prev.stopped;
    if (settled && sameReport(prev.report, read.report)) return prev;
    return { report: read.report, error: undefined, consecutiveErrors: 0, stopped: false };
  }
  const consecutiveErrors = prev.consecutiveErrors + 1;
  return {
    ...prev,
    error: read.error,
    consecutiveErrors,
    stopped: consecutiveErrors >= PERMISSION_POLL_MAX_ERRORS,
  };
}

/** Poll `backend.permissionsStatus()` while `active`; idle (no timer, no reads) otherwise. */
export function usePermissions(backend: Backend, active: boolean): PermissionsPoll {
  const [state, setState] = useState<PollState>(INITIAL_POLL);
  // Each `recheck` / `request` starts a new run: the effect restarts its loop, and a tick that
  // belongs to an older run never schedules a successor. `undefined` while the step is off screen.
  const [run, setRun] = useState(0);
  const pollKey = active ? run : undefined;

  useEffect(() => {
    if (pollKey === undefined) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Local streak for the running loop: the state is folded from it, never read back, so a
    // read that lands after the component moved on cannot resurrect a stopped loop.
    let errors = 0;
    const tick = async () => {
      let stopped = false;
      try {
        const report = await backend.permissionsStatus();
        if (!alive) return;
        errors = 0;
        setState((prev) => foldRead(prev, { ok: true, report }));
      } catch (e: unknown) {
        if (!alive) return;
        errors += 1;
        stopped = errors >= PERMISSION_POLL_MAX_ERRORS;
        setState((prev) => foldRead(prev, { ok: false, error: messageOf(e) }));
      }
      if (alive && !stopped) {
        timer = setTimeout(() => {
          void tick();
        }, PERMISSION_POLL_INTERVAL_MS);
      }
    };
    void tick();
    return () => {
      alive = false;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [backend, pollKey]);

  const recheck = useCallback(() => {
    setState((prev) => ({ ...prev, consecutiveErrors: 0, stopped: false }));
    setRun((r) => r + 1);
  }, []);

  const request = useCallback(
    async (permission: Permission) => {
      try {
        await backend.permissionsRequest(permission);
      } catch (e: unknown) {
        setState((prev) => ({ ...prev, error: messageOf(e) }));
        return;
      }
      // The answer shows up in the next read; do not wait a full second for it.
      setState((prev) => ({ ...prev, consecutiveErrors: 0, stopped: false }));
      setRun((r) => r + 1);
    },
    [backend],
  );

  return { ...state, recheck, request };
}
