import { formatCount } from "@voltip/shared";
import { Pagination, Select, useI18n } from "@voltip/ui";
import { HISTORY_PAGE_SIZES, type HistoryPageSize, isHistoryPageSize } from "./page-size";

export interface HistoryPagerProps {
  page: number;
  pages: number;
  /** Entries matching the filter and the search. */
  matching: number;
  /** A search is on: the count reads 找到 N 条 instead of 共 N 条. */
  searching: boolean;
  size: HistoryPageSize;
  onPage: (page: number) => void;
  onSize: (size: HistoryPageSize) => void;
}

/** The foot of the history log (user request 2026-10-13): where the page is (第 2 / 7 页 · 共 320
 *  条, or 找到 N 条 for a search), the page buttons when there is more than one page, and the page
 *  size. In a narrow log the count and the size share a line and the buttons take the next one; a
 *  log 36rem wide or more puts all three on one line. */
export function HistoryPager({
  page,
  pages,
  matching,
  searching,
  size,
  onPage,
  onSize,
}: HistoryPagerProps) {
  const { t } = useI18n();
  return (
    <div className="@container mt-auto border-t border-border pt-3" data-testid="history-pager">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <span className="mono text-[11px] text-fg-muted" data-testid="history-page-count">
          {t(searching ? "history.pager.found" : "history.pager.count", {
            n: matching,
            count: formatCount(matching),
            page: formatCount(page),
            pages: formatCount(pages),
          })}
        </span>
        <span className="ml-auto flex items-center gap-1.5 text-[12px] text-fg-muted @[36rem]:order-last">
          <span aria-hidden="true">{t("history.pager.sizeLabel")}</span>
          <Select
            size="sm"
            aria-label={t("history.pager.size")}
            value={String(size)}
            options={HISTORY_PAGE_SIZES.map((n) => ({
              value: String(n),
              label: t("history.pager.sizeOption", { n }),
            }))}
            onChange={(value) => {
              const next = Number(value);
              if (isHistoryPageSize(next)) onSize(next);
            }}
          />
        </span>
        {pages > 1 && (
          <Pagination
            page={page}
            pages={pages}
            onPage={onPage}
            className="w-full @[36rem]:w-auto @[36rem]:flex-1"
          />
        )}
      </div>
    </div>
  );
}
