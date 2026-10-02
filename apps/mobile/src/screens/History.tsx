import {
  HISTORY_FILTERS,
  type HistoryBucket,
  type HistoryFilter,
  formatCount,
  formatDuration,
  groupByDay,
  historyFilterLabel,
  mirrorStateText,
  outcomeLabel,
  sceneLabel,
  shortClockLabel,
} from "@voltip/shared";
import {
  Button,
  EmptyState,
  Icon,
  Input,
  Segmented,
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
import { mirrorEntryParam } from "./MirrorEntry";

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

/** Which history the page shows: this phone's (`""`) or a computer's copy (its key). */
const THIS_PHONE = "";

/** 记录 on the phone (docs/dictation.md §20.7; user decision 2026-10-01: the phone has the
 *  desktop's history): what the phone recognised itself, searched and filtered by the core
 *  (`history_query`, as on the desktop), newest first and grouped by day, with the counts of today,
 *  this week, this month and all time (`history_stats`). An entry opens its own page.
 *
 *  A switch at the top (user decision 2026-10-02) shows a paired computer's history instead: the
 *  phone's copy of it (§20.8), read-only, with the copy's state above the list and no counts. The
 *  computer chosen is the screen's `param`, so an entry opened from it comes back to it, and the
 *  tab bar returns to this phone. */
export function History() {
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const desktop = state.mirrors.some((m) => m.desktop === shell.param) ? shell.param : undefined;
  const copy = state.mirrors.find((m) => m.desktop === desktop);
  const total = copy === undefined ? state.history_total : copy.entries;
  const retention = state.settings.history;
  const now = useNow();
  const stats = useHomeStats(now);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const search = useDebounced(query, SEARCH_DEBOUNCE_MS);
  const list = useHistoryList(filter, search, now, desktop);
  const groups = useMemo(() => groupByDay(list.entries, now, locale), [list.entries, now, locale]);
  const filterLabel = (f: HistoryFilter) =>
    f === "failed" ? t("mobile.history.failed") : historyFilterLabel(f, t);

  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-history">
      {state.mirrors.length > 0 && (
        <div className="-mx-1 overflow-x-auto px-1">
          <Segmented<string>
            label={t("mirror.source")}
            size="sm"
            value={desktop ?? THIS_PHONE}
            onChange={(next) => {
              if (next === THIS_PHONE) shell.go("history");
              else shell.go("history", next);
            }}
            options={[
              { value: THIS_PHONE, label: t("mirror.thisPhone") },
              ...state.mirrors.map((m) => ({ value: m.desktop, label: m.name })),
            ]}
          />
        </div>
      )}
      {copy === undefined ? (
        <>
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
        </>
      ) : (
        <p
          className="px-1 text-[12px] text-fg-muted"
          data-testid="phone-history-mirror-state"
          data-state={copy.state}>
          {mirrorStateText(copy, Math.floor(now / 1000), locale)}
        </p>
      )}
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
        <EmptyState
          compact
          title={copy === undefined ? t("history.empty.none") : t("mirror.empty")}>
          {copy === undefined ? t("mobile.history.emptyBody") : t("mirror.emptyBody")}
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
            : filter === "starred" && copy === undefined
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
                        if (desktop === undefined) shell.go("entry", entry.id);
                        else shell.go("mirrorEntry", mirrorEntryParam(desktop, entry.id));
                      }}>
                      <span className="line-clamp-2 text-[14px] leading-5 text-fg" data-user-text>
                        {entry.text.trim().length > 0 ? entry.text : entry.raw_text}
                      </span>
                      <span className="flex items-center gap-2 text-[12px] text-fg-muted">
                        <span className="mono">{shortClockLabel(entry.at_ms)}</span>
                        <span>{outcome.text}</span>
                        {entry.origin !== undefined && (
                          <span className="truncate" data-user-text data-origin={entry.origin.kind}>
                            {t(`history.origin.${entry.origin.kind}`, {
                              device: entry.origin.device,
                            })}
                          </span>
                        )}
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
