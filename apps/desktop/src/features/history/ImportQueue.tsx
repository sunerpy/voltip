import {
  type ImportJob,
  extensionOf,
  formatTakeLength,
  importProgress,
  importStateText,
  isImportRunning,
  storageSize,
} from "@voltip/shared";
import { Button, Card, Icon, Progress, useBackend, useI18n, useUiState } from "@voltip/ui";

/** The containers whose files are video: the queue shows them with the video icon. */
const VIDEO: ReadonlySet<string> = new Set(["mp4", "m4v", "mov", "mkv", "webm"]);

/** The import queue at the top of the history page (docs/dictation.md §31): each file, where it
 *  is and its progress, 取消 while it runs or waits, 重试 and 移除 once it failed, 查看记录 and 移除
 *  once it is in the history. The list lives until Voltip closes; the original files stay. */
export function ImportQueue({ onView }: { onView: (entry: string) => void }) {
  const { t } = useI18n();
  const { imports } = useUiState();
  if (!imports.available || imports.jobs.length === 0) return null;
  const waiting = imports.jobs.filter((job) => job.state.kind === "queued").length;
  return (
    <Card padding="none" className="flex flex-col gap-1 px-4 py-3" data-testid="import-queue">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span className="text-[14px] font-medium text-fg">{t("history.imports.queue.title")}</span>
        {waiting > 0 && (
          <span className="text-[12px] text-fg-muted" data-testid="import-waiting">
            {t("history.imports.queue.waiting", { n: waiting })}
          </span>
        )}
        <span className="ml-auto text-[11px] text-fg-subtle">
          {t("history.imports.queue.note")}
        </span>
      </div>
      <ul
        className="flex max-h-[232px] flex-col overflow-y-auto"
        aria-label={t("history.imports.queue.title")}>
        {imports.jobs.map((job) => (
          <ImportRow key={job.id} job={job} onView={onView} />
        ))}
      </ul>
    </Card>
  );
}

function ImportRow({ job, onView }: { job: ImportJob; onView: (entry: string) => void }) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const { state } = job;
  const running = isImportRunning(state);
  const progress = importProgress(state);
  const facts = [
    storageSize(job.size),
    ...(job.duration_ms === undefined ? [] : [formatTakeLength(job.duration_ms, locale)]),
  ].join(" · ");
  const send = (command: "import_cancel" | "import_retry" | "import_remove") => {
    void backend.invoke(command, { id: job.id });
  };
  return (
    <li
      className="flex items-center gap-3 border-t border-border py-2 first:border-t-0"
      data-testid="import-job"
      data-state={state.kind}
      data-paused={job.paused}>
      <Icon
        name={VIDEO.has(extensionOf(job.name)) ? "video" : "wave"}
        size={14}
        className="shrink-0 text-fg-subtle"
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="truncate text-[13px] text-fg" title={job.name} data-user-text>
            {job.name}
          </span>
          <span className="mono shrink-0 text-[11px] text-fg-subtle">{facts}</span>
        </div>
        <div
          className={`text-[12px] ${state.kind === "failed" ? "text-danger" : "text-fg-muted"}`}
          data-testid="import-job-state">
          {importStateText(state, t, locale)}
          {job.paused && (
            <span className="text-fg-subtle" data-testid="import-paused">
              {` · ${t("history.imports.queue.paused")}`}
            </span>
          )}
        </div>
        {running && (
          <Progress
            value={progress ?? 0}
            indeterminate={progress === undefined}
            size={2}
            className="mt-1.5 w-[240px] max-w-full"
          />
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {(running || state.kind === "queued") && (
          <Button
            size="sm"
            variant="text"
            data-testid="import-cancel"
            onClick={() => {
              send("import_cancel");
            }}>
            {t("history.imports.queue.cancel")}
          </Button>
        )}
        {state.kind === "failed" && (
          <Button
            size="sm"
            variant="text"
            icon="refresh"
            title={state.resumable ? t("history.imports.queue.retryResume") : undefined}
            data-testid="import-retry"
            onClick={() => {
              send("import_retry");
            }}>
            {t("history.imports.queue.retry")}
          </Button>
        )}
        {state.kind === "done" && (
          <Button
            size="sm"
            variant="text"
            data-testid="import-view"
            onClick={() => {
              onView(state.entry);
            }}>
            {t("history.imports.queue.view")}
          </Button>
        )}
        {(state.kind === "failed" || state.kind === "done") && (
          <Button
            size="sm"
            variant="text"
            data-testid="import-remove"
            onClick={() => {
              send("import_remove");
            }}>
            {t("history.imports.queue.remove")}
          </Button>
        )}
      </div>
    </li>
  );
}
