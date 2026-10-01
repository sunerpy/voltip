import { BUILTIN_PRESETS, type ExportFormat, type HistoryEntry, presetLabel } from "@voltip/shared";
import {
  Button,
  Progress,
  Select,
  exportName,
  type useHistoryProcess,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useShell } from "../../app/shell-context";

// Moved to `@voltip/ui` (packages/ui/src/features/history/useHistoryProcess.ts), shared with the
// phone's history.
export { type ProcessView, exportName, isLongEntry, useHistoryProcess } from "@voltip/ui";

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
    // `shared` is the phone's answer.
    if (outcome.kind === "cancelled" || outcome.kind === "shared") return;
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
