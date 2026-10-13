import { formatCount } from "@voltip/shared";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { IconButton } from "./IconButton";

export interface PaginationProps {
  /** The page shown, from 1. */
  page: number;
  /** How many pages there are, at least 1. */
  pages: number;
  /** Asked with another page within 1…`pages`; never with the page shown. */
  onPage: (page: number) => void;
  className?: string;
}

/** A place in the row of page buttons: a page number, or a gap (…) standing for two or more. */
export type PageItem = number | "gap";

/** `from`, `from + 1`, …, `to`. */
function run(from: number, to: number): number[] {
  return Array.from({ length: to - from + 1 }, (_, i) => from + i);
}

/** The page buttons for `page` of `pages`: all of them up to seven; past that the first and the
 *  last page, the current one with a page on each side, and a gap for what lies between. Always
 *  seven places then, so the buttons do not move under the pointer as the page changes, and a gap
 *  never hides a single page (that page is shown instead). */
export function pageItems(page: number, pages: number): PageItem[] {
  const last = Math.max(1, pages);
  const at = Math.min(Math.max(1, page), last);
  if (last <= 7) return run(1, last);
  if (at <= 4) return [...run(1, 5), "gap", last];
  if (at >= last - 3) return [1, "gap", ...run(last - 4, last)];
  return [1, "gap", at - 1, at, at + 1, "gap", last];
}

/** The ends turned off at the first or the last page: dimmed and inert, but still in the tab order
 *  (`aria-disabled`, not `disabled`), so a keyboard that pressed 下一页 up to the last page keeps
 *  its place instead of dropping the focus to the page. */
const INERT =
  "aria-disabled:opacity-40 aria-disabled:hover:bg-transparent aria-disabled:hover:text-fg-muted";

/** Page buttons for a long list (the desktop history page, user request 2026-10-13): first,
 *  previous, the page numbers with gaps, next and last, in the interface language. The current page
 *  is marked `aria-current="page"` in the soft accent tint; the first and last buttons give way in a
 *  container narrower than 19rem, where the first and last page numbers still are. */
export function Pagination({ page, pages, onPage, className }: PaginationProps) {
  const t = useT();
  const last = Math.max(1, pages);
  const go = (target: number) => {
    if (target >= 1 && target <= last && target !== page) onPage(target);
  };
  const end = (
    icon: "chevronsLeft" | "chevronLeft" | "chevronRight" | "chevronsRight",
    label: string,
    target: number,
    narrowHidden: boolean,
  ) => {
    const off = target < 1 || target > last || target === page;
    return (
      <IconButton
        icon={icon}
        label={label}
        size={28}
        aria-disabled={off || undefined}
        onClick={() => {
          go(target);
        }}
        className={cx(INERT, narrowHidden && "@max-[19rem]:hidden")}
      />
    );
  };
  return (
    <nav aria-label={t("ui.pagination.label")} className={cx("@container", className)}>
      <div className="flex flex-wrap items-center justify-center gap-0.5">
        {end("chevronsLeft", t("ui.pagination.first"), 1, true)}
        {end("chevronLeft", t("ui.pagination.previous"), page - 1, false)}
        {pageItems(page, last).map((item, i) =>
          item === "gap" ? (
            <span
              key={i < 3 ? "gap-start" : "gap-end"}
              aria-hidden="true"
              data-page-gap=""
              className="mono inline-flex h-7 w-5 items-center justify-center text-[12px] text-fg-subtle select-none">
              …
            </span>
          ) : (
            <button
              key={item}
              type="button"
              aria-label={t("ui.pagination.page", { n: formatCount(item) })}
              aria-current={item === page ? "page" : undefined}
              onClick={() => {
                go(item);
              }}
              className={cx(
                "mono inline-flex h-7 min-w-7 shrink-0 items-center justify-center rounded-control px-1.5 text-[12px] tabular-nums transition-colors",
                item === page
                  ? "bg-accent-soft font-medium text-accent-text"
                  : "text-fg-muted hover:bg-inset hover:text-fg",
              )}>
              {formatCount(item)}
            </button>
          ),
        )}
        {end("chevronRight", t("ui.pagination.next"), page + 1, false)}
        {end("chevronsRight", t("ui.pagination.last"), last, true)}
      </div>
    </nav>
  );
}
