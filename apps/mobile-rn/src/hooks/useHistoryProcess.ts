// Copied from packages/ui/src/features/history/useHistoryProcess.ts: the RN app cannot import `@voltip/ui` (React 19.3 vs
// React Native's 19.2, docs/mobile-rn.md §5). Keep the two in step until one shell remains.
import { type HistoryEntry, type ProcessedText, textChars } from "@voltip/shared";
import { useEffect, useRef, useState } from "react";
import { useBackend } from "../backend/BackendProvider";

/** docs/dictation.md §22: a take longer than two minutes, or a text past the 2000 characters the
 *  clean-up takes at once, is a long entry — the one the tools below are for. */
export function isLongEntry(entry: HistoryEntry): boolean {
  return (
    entry.kind === "dictation" && (entry.duration_ms > 120_000 || textChars(entry.text) > 2000)
  );
}

/** Where 用 AI 预设处理 is on a history page (the desktop's or the phone's). */
export type ProcessView =
  | { state: "idle" }
  | { state: "running"; done: number; total: number }
  | { state: "failed"; reason: string }
  | { state: "cancelled" }
  | { state: "done"; processed: ProcessedText };

// Request ids only have to differ from the ones still running in this core.
let nextRequest = Date.now();

const IDLE: ProcessView = { state: "idle" };

/** `history_process` for entry `entryId` (docs/dictation.md §22): the request this page sent and
 *  its answers. Another entry, or leaving the page, cancels a request still running. */
export function useHistoryProcess(entryId: string) {
  const { backend } = useBackend();
  const [current, setCurrent] = useState<{ entryId: string; view: ProcessView }>({
    entryId,
    view: IDLE,
  });
  const request = useRef<{ id: number; entryId: string } | undefined>(undefined);
  useEffect(
    () => () => {
      if (request.current?.entryId === entryId) {
        void backend.invoke("history_process_cancel", { requestId: request.current.id });
        request.current = undefined;
      }
    },
    [backend, entryId],
  );
  useEffect(
    () =>
      backend.on((event) => {
        const sent = request.current;
        if (event.type !== "history_process" || event.request_id !== sent?.id) return;
        const answer = event.state;
        if (answer.state !== "running") request.current = undefined;
        setCurrent({
          entryId: sent.entryId,
          view:
            answer.state === "running"
              ? { state: "running", done: answer.done, total: answer.total }
              : answer.state === "done"
                ? { state: "done", processed: answer.processed }
                : answer,
        });
      }),
    [backend],
  );
  return {
    view: current.entryId === entryId ? current.view : IDLE,
    start: (preset: string) => {
      nextRequest += 1;
      request.current = { id: nextRequest, entryId };
      setCurrent({ entryId, view: { state: "running", done: 0, total: 0 } });
      void backend.invoke("history_process", { requestId: nextRequest, id: entryId, preset });
    },
    cancel: () => {
      if (request.current !== undefined)
        void backend.invoke("history_process_cancel", { requestId: request.current.id });
    },
  };
}

const two = (n: number) => String(n).padStart(2, "0");

/** `Voltip 2026-09-30 15.30`: the local time of `atMs` as an export's file name offers it. */
export function exportName(atMs: number): string {
  const d = new Date(atMs);
  return `Voltip ${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}.${two(d.getMinutes())}`;
}
