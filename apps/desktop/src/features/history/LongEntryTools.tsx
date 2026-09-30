import {
  BUILTIN_PRESETS,
  type ExportFormat,
  type HistoryEntry,
  type ProcessedText,
  presetLabel,
} from "@voltip/shared";
import { Button, Progress, Select, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useEffect, useRef, useState } from "react";
import { useShell } from "../../app/shell-context";
import { textChars } from "./stats";

/** docs/dictation.md §22: a take longer than two minutes, or a text past the 2000 characters the
 *  clean-up takes at once, is a long entry — the one the tools below are for. */
export function isLongEntry(entry: HistoryEntry): boolean {
  return (
    entry.kind === "dictation" && (entry.duration_ms > 120_000 || textChars(entry.text) > 2000)
  );
}

/** Where 用 AI 预设处理 is on this page. */
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

/** 用 AI 预设处理 and the two exports of a long entry (docs/dictation.md §22). */
export function LongEntryTools({
  entry,
  process,
}: {
  entry: HistoryEntry;
  process: ReturnType<typeof useHistoryProcess>;
}) {
  const shell = useShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const { settings, presets } = useUiState();
  const [preset, setPreset] = useState<string>(settings.engines.refine_preset);
  const view = process.view;
  const processed = view.state === "done" ? view.processed : entry.processed;
  const running = view.state === "running";
  const hasSegments = (entry.segments ?? []).some((s) => s.text.trim().length > 0);
  const options = [
    ...BUILTIN_PRESETS.map((id) => ({ value: id, label: presetLabel(id, presets, locale) })),
    ...presets.map((p) => ({ value: p.id, label: p.name })),
  ];
  const save = async (format: ExportFormat) => {
    const outcome = await backend.historyExport(entry.id, format, exportName(entry.at_ms));
    if (outcome.kind === "cancelled") return;
    shell.toast(
      outcome.kind === "saved"
        ? { message: t("history.long.saved", { path: outcome.path }), duration: 4000 }
        : {
            message: t(`history.long.exportFailed.${outcome.code}`, { detail: outcome.detail }),
            duration: 4000,
            tone: "danger",
          },
    );
  };
  return (
    <div className="flex flex-col gap-3 rounded-10 hairline p-3" data-testid="history-long-tools">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[13px] font-medium text-fg">{t("history.long.title")}</span>
        <Select
          aria-label={t("history.long.preset")}
          size="sm"
          value={preset}
          options={options}
          disabled={running}
          data-testid="history-long-preset"
          onChange={setPreset}
        />
        {running ? (
          <Button
            size="sm"
            variant="outline"
            data-testid="history-long-cancel"
            onClick={process.cancel}>
            {t("common.cancel")}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="primary"
            data-testid="history-long-start"
            onClick={() => {
              process.start(preset);
            }}>
            {processed === undefined ? t("history.long.start") : t("history.long.again")}
          </Button>
        )}
      </div>
      {running && (
        <div className="flex items-center gap-3" data-testid="history-long-progress">
          <Progress
            value={view.total === 0 ? 0 : view.done / view.total}
            indeterminate={view.total === 0}
            size={2}
            className="w-[160px]"
          />
          <span className="mono text-[11px] text-fg-muted">
            {t("history.long.running", { done: view.done, total: view.total })}
          </span>
        </div>
      )}
      {view.state === "failed" && (
        <p className="text-[12px] text-danger" data-testid="history-long-failed">
          {t("history.long.failed", { reason: view.reason })}
        </p>
      )}
      {view.state === "cancelled" && (
        <p className="text-[12px] text-fg-muted" data-testid="history-long-cancelled">
          {t("history.long.cancelled")}
        </p>
      )}
      <p className="text-[11px] leading-4 text-fg-subtle">
        {t("history.long.note")}
        {settings.engines.llm_provider === "builtin" && ` ${t("history.long.builtinNote")}`}
      </p>
      <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
        <Button
          size="sm"
          icon="download"
          disabled={!hasSegments}
          title={hasSegments ? undefined : t("history.long.exportFailed.empty")}
          data-testid="history-export-srt"
          onClick={() => {
            void save("srt");
          }}>
          {t("history.long.exportSrt")}
        </Button>
        <Button
          size="sm"
          icon="download"
          data-testid="history-export-txt"
          onClick={() => {
            void save("txt");
          }}>
          {t("history.long.exportTxt")}
        </Button>
        {processed !== undefined && (
          <span className="text-[11px] text-fg-muted" data-testid="history-export-txt-note">
            {t("history.long.txtUsesProcessed")}
          </span>
        )}
      </div>
    </div>
  );
}
