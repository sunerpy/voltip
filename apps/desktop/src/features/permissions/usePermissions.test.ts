import {
  PERMISSION_POLL_INTERVAL_MS,
  type PermissionReport,
  notApplicablePermissions,
} from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, renderHook, waitFor } from "@testing-library/react";
import { INITIAL_POLL, foldRead, usePermissions } from "./usePermissions";

const granted: PermissionReport = {
  platform: "macos",
  microphone: "granted",
  accessibility: "granted",
};

describe("foldRead (voltip_platform::Poller, TypeScript side)", () => {
  it("errors extend the streak and stop at three; a success resets it", () => {
    const err = { ok: false as const, error: "x" };
    let s = foldRead(INITIAL_POLL, err);
    s = foldRead(s, err);
    expect([s.consecutiveErrors, s.stopped]).toEqual([2, false]);
    // A success in between resets the streak: two more errors do not stop it.
    s = foldRead(s, { ok: true, report: granted });
    expect(s).toEqual({ report: granted, error: undefined, consecutiveErrors: 0, stopped: false });
    s = foldRead(foldRead(s, err), err);
    expect(s.stopped).toBe(false);
    s = foldRead(s, err);
    expect([s.consecutiveErrors, s.stopped, s.error]).toEqual([3, true, "x"]);
    // The last good report survives the errors, so the table keeps showing it.
    expect(s.report).toEqual(granted);
  });

  it("an unchanged answer keeps the same state object (no re-render every second)", () => {
    const s = foldRead(INITIAL_POLL, { ok: true, report: granted });
    expect(foldRead(s, { ok: true, report: { ...granted } })).toBe(s);
    const changed = foldRead(s, { ok: true, report: { ...granted, accessibility: "denied" } });
    expect(changed).not.toBe(s);
    expect(changed.report?.accessibility).toBe("denied");
    // After an error the same report is a real change (the error clears).
    const afterError = foldRead(foldRead(s, { ok: false, error: "x" }), {
      ok: true,
      report: granted,
    });
    expect(afterError.error).toBeUndefined();
    expect(afterError.consecutiveErrors).toBe(0);
  });
});

describe("usePermissions", () => {
  it("reads nothing while inactive and starts reading once active", async () => {
    const backend = new MockBackend({ permissions: notApplicablePermissions("linux") });
    const spy = vi.spyOn(backend, "permissionsStatus");
    const { result, rerender } = renderHook(({ on }) => usePermissions(backend, on), {
      initialProps: { on: false },
    });
    expect(spy).not.toHaveBeenCalled();
    expect(result.current.report).toBeUndefined();
    rerender({ on: true });
    await waitFor(() => {
      expect(result.current.report).toEqual(notApplicablePermissions("linux"));
    });
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("keeps polling every second while active and stops when the step leaves", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend({ permissions: granted });
      const spy = vi.spyOn(backend, "permissionsStatus");
      const flush = (ms: number) =>
        act(async () => {
          await vi.advanceTimersByTimeAsync(ms);
        });
      const { rerender, unmount } = renderHook(({ on }) => usePermissions(backend, on), {
        initialProps: { on: true },
      });
      await flush(0);
      expect(spy).toHaveBeenCalledTimes(1);
      await flush(PERMISSION_POLL_INTERVAL_MS - 1);
      expect(spy).toHaveBeenCalledTimes(1);
      await flush(1);
      expect(spy).toHaveBeenCalledTimes(2);
      await flush(PERMISSION_POLL_INTERVAL_MS);
      expect(spy).toHaveBeenCalledTimes(3);
      rerender({ on: false });
      await flush(PERMISSION_POLL_INTERVAL_MS * 5);
      expect(spy).toHaveBeenCalledTimes(3);
      unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a failed request surfaces its message; plain and object rejections are readable too", async () => {
    const backend = new MockBackend({ permissions: granted });
    const { result } = renderHook(() => usePermissions(backend, true));
    await waitFor(() => {
      expect(result.current.report).toEqual(granted);
    });
    backend.permissionsRequest = () => Promise.reject(new Error("tcc refused"));
    await act(async () => {
      await result.current.request("accessibility");
    });
    expect(result.current.error).toBe("tcc refused");
    backend.permissionsRequest = () => Promise.reject("plain");
    await act(async () => {
      await result.current.request("microphone");
    });
    expect(result.current.error).toBe("plain");
    backend.permissionsStatus = () => Promise.reject({ code: 7 });
    act(() => {
      result.current.recheck();
    });
    await waitFor(() => {
      expect(result.current.error).toBe('{"code":7}');
    });
    expect(result.current.consecutiveErrors).toBe(1);
  });

  it("a successful request re-reads at once instead of waiting for the next tick", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend({
        permissions: { ...granted, accessibility: "denied" },
      });
      const spy = vi.spyOn(backend, "permissionsStatus");
      const { result } = renderHook(() => usePermissions(backend, true));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(result.current.report?.accessibility).toBe("denied");
      expect(spy).toHaveBeenCalledTimes(1);
      await act(async () => {
        await result.current.request("accessibility");
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      // No timer advanced: the request itself triggered the second read.
      expect(spy).toHaveBeenCalledTimes(2);
      expect(result.current.report?.accessibility).toBe("granted");
      expect(backend.permissionRequests).toEqual(["accessibility"]);
    } finally {
      vi.useRealTimers();
    }
  });
});
