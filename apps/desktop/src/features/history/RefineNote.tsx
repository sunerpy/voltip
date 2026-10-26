import { type HistoryEntry, serviceReasonText } from "@voltip/shared";
import { Button, useI18n } from "@voltip/ui";

/** The kinds of failure whose entry keeps the service's explanation (docs/dictation.md §3.6). */
type RequestFailure = "rate_limited" | "quota" | "failed";

function requestFailure(entry: HistoryEntry): RequestFailure | undefined {
  const failure = entry.refine_failure;
  return failure === "rate_limited" || failure === "quota" || failure === "failed"
    ? failure
    : undefined;
}

/** Under an entry whose clean-up request failed (docs/dictation.md §3.6): one sentence for the
 *  kind of failure and what to do about it, then why, with what the service said (§3.12). Nothing
 *  for an entry written before the reason was kept. With `onRetry` it offers 重新润色
 *  (docs/dictation.md §26): the clean-up again, kept as the processed text. */
export function RefineNote({
  entry,
  onRetry,
  retrying = false,
}: {
  entry: HistoryEntry;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  const { t, locale } = useI18n();
  const failure = requestFailure(entry);
  if (failure === undefined || entry.refine_error === undefined) return null;
  return (
    <div
      className="flex flex-wrap items-start gap-x-3 gap-y-2 rounded-6 bg-warning-soft px-3 py-2 text-[12px] leading-5 text-warning"
      data-testid="history-refine-note"
      data-failure={failure}>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <p>{t(`history.refineNote.${failure}`)}</p>
        <p className="break-words text-fg-muted" data-testid="history-refine-detail">
          {t("history.refineNote.reason")}
          <span className="mono" data-user-text>
            {serviceReasonText(entry.refine_error, locale)}
          </span>
        </p>
      </div>
      {onRetry !== undefined && (
        <Button
          size="sm"
          icon="wand"
          disabled={retrying}
          data-testid="history-refine-retry"
          onClick={onRetry}>
          {retrying ? t("history.refineNote.retrying") : t("history.refineNote.retry")}
        </Button>
      )}
    </div>
  );
}
