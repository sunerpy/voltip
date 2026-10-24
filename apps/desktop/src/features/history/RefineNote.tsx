import { type HistoryEntry, serviceReasonText } from "@voltip/shared";
import { useI18n } from "@voltip/ui";

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
 *  for an entry written before the reason was kept. */
export function RefineNote({ entry }: { entry: HistoryEntry }) {
  const { t, locale } = useI18n();
  const failure = requestFailure(entry);
  if (failure === undefined || entry.refine_error === undefined) return null;
  return (
    <div
      className="flex flex-col gap-1 rounded-6 bg-warning-soft px-3 py-2 text-[12px] leading-5 text-warning"
      data-testid="history-refine-note"
      data-failure={failure}>
      <p>{t(`history.refineNote.${failure}`)}</p>
      <p className="break-words text-fg-muted" data-testid="history-refine-detail">
        {t("history.refineNote.reason")}
        <span className="mono" data-user-text>
          {serviceReasonText(entry.refine_error, locale)}
        </span>
      </p>
    </div>
  );
}
