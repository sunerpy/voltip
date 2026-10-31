import { Icon, cx } from "@voltip/ui";

export type Tone = "info" | "warn" | "danger" | "busy";

/** One line under a suggestion or an ignored pair: an icon for its tone, the text (it names the
 *  user's words), and optionally a link to what it is about. */
export function Line({
  tone,
  text,
  link,
}: {
  tone: Tone;
  text: string;
  link?: { label: string; onClick: () => void };
}) {
  const color =
    tone === "danger" ? "text-danger" : tone === "warn" ? "text-warning" : "text-fg-muted";
  return (
    <div className="flex items-start gap-2.5" data-tone={tone}>
      {tone === "busy" ? (
        <span
          aria-hidden
          className="mt-0.5 inline-block size-3.5 shrink-0 animate-spin rounded-pill border-2 border-primary/30 border-t-primary"
        />
      ) : (
        <Icon
          name={tone === "info" ? "info" : "alert"}
          size={14}
          className={cx("mt-0.5 shrink-0", color)}
        />
      )}
      <p className={cx("min-w-0", tone === "warn" || tone === "danger" ? color : "text-fg")}>
        <span data-user-text>{text}</span>
        {link !== undefined && (
          <button
            type="button"
            className="ml-1 font-medium text-accent-text hover:text-accent-text-hover"
            onClick={link.onClick}>
            {link.label}
          </button>
        )}
      </p>
    </div>
  );
}

/** A pair's cell: what the recogniser wrote, muted, then what the polished text has. */
export function Pair({
  heard,
  polished,
  testId,
}: {
  heard: string;
  polished: string;
  testId: string;
}) {
  return (
    <span className="flex min-w-0 items-center gap-1.5" data-testid={testId}>
      <span data-user-text className="min-w-0 truncate text-fg-muted">
        {heard}
      </span>
      <span className="shrink-0 text-fg-subtle">→</span>
      <span data-user-text className="min-w-0 truncate font-medium text-fg">
        {polished}
      </span>
    </span>
  );
}

/** The band under a table row that holds its lines (a message, details). */
export const ROW_BAND = "flex flex-col gap-2 bg-inset px-3 py-2.5 text-[12px] leading-[18px]";
