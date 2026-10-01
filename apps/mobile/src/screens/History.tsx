import {
  HISTORY_FILTERS,
  type HistoryBucket,
  type HistoryFilter,
  formatCount,
  formatDuration,
  groupByDay,
  historyFilterLabel,
  outcomeLabel,
  sceneLabel,
  shortClockLabel,
} from "@voltip/shared";
import {
  Button,
  EmptyState,
  Icon,
  Input,
  Select,
  useDebounced,
  useHistoryList,
  useHomeStats,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { useMemo, useState } from "react";
import { useMobileShell } from "../app/shell";

/** How long the search waits after the last keystroke, as on the desktop. */
const SEARCH_DEBOUNCE_MS = 200;

function Tile({ label, bucket }: { label: string; bucket: HistoryBucket }) {
  const { t, locale } = useI18n();
  return (
    <div className="flex flex-col gap-0.5 rounded-10 bg-surface p-3 hairline">
      <span className="text-[12px] text-fg-muted">{label}</span>
      <span className="text-[15px] font-semibold text-fg">
        {t("home.session.count", { n: formatCount(bucket.count) })}
      </span>
      <span className="text-[11px] text-fg-subtle">
        {t("home.tiles.saved", { time: formatDuration(bucket.savedMs, locale) })}
      </span>
    </div>
  );
}

/** 记录 on the phone (docs/dictation.md §20.7; user decision 2026-10-01: the phone has the
 *  desktop's history): what the phone recognised itself, searched and filtered by the core
 *  (`history_query`, as on the desktop), newest first and grouped by day, with the counts of today,
 *  this week, this month and all time (`history_stats`). An entry opens its own page. A take sent to
 *  a computer is in that computer's history. */
export function History() {
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const total = state.history_total;
  const retention = state.settings.history;
  const now = useNow();
  const stats = useHomeStats(now);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const search = useDebounced(query, SEARCH_DEBOUNCE_MS);
  const list = useHistoryList(filter, search, now);
  const groups = useMemo(() => groupByDay(list.entries, now, locale), [list.entries, now, locale]);
  const filterLabel = (f: HistoryFilter) =>
    f === "failed" ? t("mobile.history.failed") : historyFilterLabel(f, t);

  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-history">
      <div className="grid grid-cols-2 gap-2" data-testid="phone-history-stats">
        <Tile label={t("home.tiles.today")} bucket={stats.today} />
        <Tile label={t("home.tiles.week")} bucket={stats.week} />
        <Tile label={t("home.tiles.month")} bucket={stats.month} />
        <Tile label={t("home.tiles.total")} bucket={stats.total} />
      </div>
      <button
        type="button"
        className="flex items-center gap-2 px-1 text-left text-[12px] text-fg-muted"
        data-testid="phone-history-retention"
        onClick={() => {
          shell.go("historySettings");
        }}>
        <span className="flex-1">
          {retention.enabled
            ? t("history.banner.retention", { keep: formatCount(retention.keep) })
            : t("history.banner.off", { n: formatCount(total) })}
        </span>
        <Icon name="chevronRight" size={14} className="shrink-0 text-fg-subtle" />
      </button>
      <div className="flex items-end gap-2">
        <Input
          size="sm"
          icon="search"
          className="flex-1"
          aria-label={t("history.search.label")}
          placeholder={t("mobile.history.search")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
          }}
        />
        <Select
          aria-label={t("history.filter.label")}
          size="sm"
          value={filter}
          options={HISTORY_FILTERS.map((f) => ({ value: f, label: filterLabel(f) }))}
          onChange={setFilter}
        />
      </div>
      {total === 0 && (
        <EmptyState compact title={t("history.empty.none")}>
          {t("mobile.history.emptyBody")}
        </EmptyState>
      )}
      {total > 0 && list.settled && list.matching === 0 && (
        <EmptyState
          compact
          title={
            search
              ? t("history.empty.noMatch", { query: search })
              : t("history.empty.noneInFilter", { filter: filterLabel(filter) })
          }
          actions={
            search ? (
              <Button
                size="sm"
                variant="text"
                onClick={() => {
                  setQuery("");
                }}>
                {t("history.empty.clearSearch")}
              </Button>
            ) : undefined
          }>
          {search
            ? t("history.empty.noMatchBody", { n: formatCount(total) })
            : filter === "starred"
              ? t("mobile.history.starHint")
              : t("history.empty.rangeHint")}
        </EmptyState>
      )}
      <ul className="flex flex-col gap-3" aria-label={t("history.log")}>
        {groups.map((group) => (
          <li key={group.day} className="flex flex-col gap-1">
            <div className="flex items-center justify-between px-1 text-[12px] text-fg-muted">
              <span>{group.day}</span>
              <span className="mono">{group.items.length}</span>
            </div>
            <ul className="flex flex-col divide-y divide-border overflow-hidden rounded-10 bg-surface hairline">
              {group.items.map((entry) => {
                const outcome = outcomeLabel(entry.outcome, locale);
                return (
                  <li key={entry.id}>
                    <button
                      type="button"
                      className="flex w-full flex-col gap-1 px-4 py-3 text-left"
                      data-testid="phone-history-row"
                      onClick={() => {
                        shell.go("entry", entry.id);
                      }}>
                      <span className="line-clamp-2 text-[14px] leading-5 text-fg" data-user-text>
                        {entry.text.trim().length > 0 ? entry.text : entry.raw_text}
                      </span>
                      <span className="flex items-center gap-2 text-[12px] text-fg-muted">
                        <span className="mono">{shortClockLabel(entry.at_ms)}</span>
                        <span>{outcome.text}</span>
                        {entry.scene !== undefined && (
                          <span className="truncate" data-user-text>
                            {sceneLabel(entry.scene, locale)}
                          </span>
                        )}
                        {entry.starred && (
                          <Icon
                            name="star"
                            size={12}
                            className="ml-auto shrink-0 text-accent"
                            aria-label={t("history.star")}
                          />
                        )}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </li>
        ))}
      </ul>
      {list.more && (
        <div className="flex flex-col items-center gap-1">
          <Button size="sm" variant="ghost" onClick={list.loadMore}>
            {t("history.loadMore")}
          </Button>
          <span className="text-[11px] text-fg-subtle">
            {t("history.loaded", {
              shown: formatCount(list.entries.length),
              n: formatCount(list.matching),
            })}
          </span>
        </div>
      )}
    </div>
  );
}
