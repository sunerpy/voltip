// Pure helpers of the history pages and the desktop's home page, shared by the desktop and the
// phone (user decision 2026-10-01: the phone has the desktop's history): the home statistics from
// the core's `history_stats` (docs/dictation.md §4.5), the 6-week heatmap, the list filters as
// `history_query` arguments, the day grouping and the labels. Every calendar boundary is the local
// one, computed with `Date` so the numbers match what the user sees on the clock; `now` is
// injected so tests stay deterministic. Moved here from
// `apps/desktop/src/features/history/stats.ts`, which re-exports it.
import {
  DEFAULT_LOCALE,
  type Locale,
  type TFunction,
  formatDateTime,
  translate,
  zhT,
} from "./i18n";
import {
  HISTORY_LIMIT,
  type HistoryEntry,
  type HistoryQueryArgs,
  type HistoryStats,
  type HistoryStatsBucket,
} from "./schema";

/** The history retention choices the settings offer (all inside `HISTORY_MIN_KEEP..=HISTORY_LIMIT`),
 *  on the desktop and the phone. */
export const HISTORY_KEEP_OPTIONS = [500, 2000, 5000, 10_000, HISTORY_LIMIT] as const;

/** Saved time = speaking time × this (docs/dictation.md §4.5): in Ruan et al. 2016
 *  (arXiv:1608.07323) speech input was about 2.9 times as fast as typing on a phone (English 153
 *  vs 52 words per minute, Chinese 123 vs 43). */
export const SAVED_TIME_FACTOR = 1.9;

export interface HistoryBucket {
  count: number;
  /** Characters recognised (the transcript). */
  rawChars: number;
  /** Characters the clean-up changed. */
  correctedChars: number;
  /** Total recording time in this bucket. */
  spokenMs: number;
  /** `spokenMs × SAVED_TIME_FACTOR`, rounded. */
  savedMs: number;
  /** Mean ASR + refine latency, rounded; `undefined` when the bucket is empty. */
  latencyMs: number | undefined;
}

/** What the home page shows: dictations only (the core leaves out voice edits and what a phone
 *  typed or sent from its clipboard). */
export interface HomeStats {
  today: HistoryBucket;
  /** Since local Monday 00:00. */
  week: HistoryBucket;
  /** Since the 1st of the local month. */
  month: HistoryBucket;
  total: HistoryBucket;
  /** `values[week][weekday]` for the last 6 weeks (oldest column first), levels 0..3. */
  heatmap: number[][];
}

export type HistoryFilter = "today" | "week" | "month" | "all" | "starred" | "failed";
export const HISTORY_FILTERS: readonly HistoryFilter[] = [
  "all",
  "today",
  "week",
  "month",
  "starred",
  "failed",
];
export function historyFilterLabel(filter: HistoryFilter, t: TFunction = zhT.t): string {
  return t(`history.filter.${filter}`);
}

/** Filter labels in the default locale; the page reads `historyFilterLabel(filter, t)`. */
export const HISTORY_FILTER_LABELS: Readonly<Record<HistoryFilter, string>> = {
  all: historyFilterLabel("all"),
  today: historyFilterLabel("today"),
  week: historyFilterLabel("week"),
  month: historyFilterLabel("month"),
  starred: historyFilterLabel("starred"),
  failed: historyFilterLabel("failed"),
};

export function isHistoryFilter(value: string | undefined): value is HistoryFilter {
  return value !== undefined && (HISTORY_FILTERS as readonly string[]).includes(value);
}

const DAY_MS = 86_400_000;
const HEATMAP_WEEKS = 6;

/** Local midnight of the day containing `ms`. */
export function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Local midnight of the Monday starting the week that contains `ms`. */
export function startOfWeek(ms: number): number {
  const d = new Date(startOfDay(ms));
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d.getTime();
}

/** Local midnight of the 1st of the month containing `ms`. */
export function startOfMonth(ms: number): number {
  const d = new Date(startOfDay(ms));
  d.setDate(1);
  return d.getTime();
}

/** Whole local days between two instants (DST-safe: rounds the 23 / 25 h days). */
export function dayDiff(later: number, earlier: number): number {
  return Math.round((startOfDay(later) - startOfDay(earlier)) / DAY_MS);
}

export function textChars(text: string): number {
  return Array.from(text).length;
}

export function entryLatencyMs(entry: HistoryEntry): number {
  return entry.asr_ms + (entry.refine_ms ?? 0);
}

/** The local midnights `history_stats` is asked for: from the Monday five weeks before this
 *  week's to tomorrow's (37 to 43 of them: one bucket per day, today's last). */
export function statsBoundaries(now: number): number[] {
  const out: number[] = [];
  const day = new Date(startOfWeek(now));
  day.setDate(day.getDate() - 7 * (HEATMAP_WEEKS - 1));
  const tomorrow = new Date(startOfDay(now));
  tomorrow.setDate(tomorrow.getDate() + 1);
  while (day.getTime() <= tomorrow.getTime()) {
    out.push(day.getTime());
    day.setDate(day.getDate() + 1);
  }
  return out;
}

function toBucket(parts: readonly HistoryStatsBucket[]): HistoryBucket {
  const sum = (key: keyof HistoryStatsBucket) => parts.reduce((n, p) => n + p[key], 0);
  const count = sum("count");
  const spokenMs = sum("spoken_ms");
  return {
    count,
    rawChars: sum("raw_chars"),
    correctedChars: sum("corrected_chars"),
    spokenMs,
    savedMs: Math.round(spokenMs * SAVED_TIME_FACTOR),
    latencyMs: count === 0 ? undefined : Math.round(sum("latency_ms") / count),
  };
}

/** The home page's numbers from the core's answer for `boundaries = statsBoundaries(now)`:
 *  today, this week and this month are sums of daily buckets, the heatmap is one cell per day. */
export function homeStats(
  stats: HistoryStats,
  boundaries: readonly number[],
  now: number,
): HomeStats {
  const since = (from: number) => stats.buckets.filter((_, i) => (boundaries[i] ?? 0) >= from);
  const heatmap = Array.from({ length: HEATMAP_WEEKS }, () => Array.from({ length: 7 }, () => 0));
  stats.buckets.forEach((bucket, i) => {
    const column = heatmap[Math.floor(i / 7)];
    if (column) column[i % 7] = Math.min(3, bucket.count);
  });
  return {
    today: toBucket(since(startOfDay(now))),
    week: toBucket(since(startOfWeek(now))),
    month: toBucket(since(startOfMonth(now))),
    total: toBucket([stats.total]),
    heatmap,
  };
}

/** Nothing counted yet (before the first answer, or on an empty history). */
export function emptyHomeStats(): HomeStats {
  return homeStats({ buckets: [], total: toBucketPart() }, [], 0);
}

function toBucketPart(): HistoryStatsBucket {
  return { count: 0, raw_chars: 0, corrected_chars: 0, spoken_ms: 0, latency_ms: 0 };
}

/** A home tile or sidebar filter, and the search text, as `history_query` arguments. */
export function historyQueryArgs(
  filter: HistoryFilter,
  query: string,
  now: number,
): Omit<HistoryQueryArgs, "offset" | "limit"> {
  const q = query.trim() === "" ? {} : { query };
  switch (filter) {
    case "all":
      return q;
    case "today":
      return { ...q, sinceMs: startOfDay(now) };
    case "week":
      return { ...q, sinceMs: startOfWeek(now) };
    case "month":
      return { ...q, sinceMs: startOfMonth(now) };
    case "starred":
      return { ...q, starred: true };
    case "failed":
      return { ...q, failed: true };
  }
}

/** `14:32:07` in local time. */
export function clockLabel(ms: number): string {
  const d = new Date(ms);
  return [d.getHours(), d.getMinutes(), d.getSeconds()]
    .map((n) => String(n).padStart(2, "0"))
    .join(":");
}

/** `14:32` in local time. */
export function shortClockLabel(ms: number): string {
  return clockLabel(ms).slice(0, 5);
}

/** `今天 · 9月24日星期四` / `Today · Thu, Sep 24` (the history page's day headers): the date
 *  through `Intl.DateTimeFormat` in the locale, today / yesterday from the dictionary. */
export function dayLabel(ms: number, now: number, locale: Locale = DEFAULT_LOCALE): string {
  const date = formatDateTime(locale, ms, {
    month: locale === "zh-CN" ? "long" : "short",
    day: "numeric",
    weekday: locale === "zh-CN" ? "long" : "short",
  });
  const diff = dayDiff(now, ms);
  if (diff === 0) return `${translate(locale, "time.today")} · ${date}`;
  if (diff === 1) return `${translate(locale, "time.yesterday")} · ${date}`;
  return date;
}

/** `今天 · 09-24 周四` / `Today · 09-24 Thu` (the session panel header). */
export function todayLabel(now: number, locale: Locale = DEFAULT_LOCALE): string {
  const d = new Date(now);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const weekday = formatDateTime(locale, now, { weekday: "short" });
  return `${translate(locale, "time.today")} · ${mm}-${dd} ${weekday}`;
}

/** Time column of the recent table (104 px mono): the clock today, `昨天 18:05` yesterday where
 *  the locale has a word short enough (`time.yesterdayCompact`; English has none, "Yesterday 18:05"
 *  overflowed the cell), the date (`9-22 18:05`) otherwise. */
export function recentTimeLabel(ms: number, now: number, locale: Locale = DEFAULT_LOCALE): string {
  const diff = dayDiff(now, ms);
  if (diff === 0) return clockLabel(ms);
  const yesterday = translate(locale, "time.yesterdayCompact");
  if (diff === 1 && yesterday !== "") return `${yesterday} ${shortClockLabel(ms)}`;
  const d = new Date(ms);
  return `${d.getMonth() + 1}-${String(d.getDate()).padStart(2, "0")} ${shortClockLabel(ms)}`;
}

/** Newest-first groups by local day for the history list. */
export function groupByDay(
  entries: readonly HistoryEntry[],
  now: number,
  locale: Locale = DEFAULT_LOCALE,
): { day: string; items: HistoryEntry[] }[] {
  const out: { day: string; key: number; items: HistoryEntry[] }[] = [];
  for (const e of entries) {
    const key = startOfDay(e.at_ms);
    const last = out.at(-1);
    if (last?.key === key) last.items.push(e);
    else out.push({ day: dayLabel(e.at_ms, now, locale), key, items: [e] });
  }
  return out.map(({ day, items }) => ({ day, items }));
}
