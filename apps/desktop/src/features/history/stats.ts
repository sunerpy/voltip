// Pure helpers over `UiState.history` (`HistoryEntry[]`, newest first): the home statistics,
// the 6-week heatmap, the day grouping of the history page and the list filters. Every calendar
// boundary is the local one, computed with `Date` so the numbers match what the user sees on the
// clock; `now` is injected so tests stay deterministic.
import {
  DEFAULT_LOCALE,
  type HistoryEntry,
  LOCALES,
  type Locale,
  type TFunction,
  formatDateTime,
  sceneLabel,
  translate,
  zhT,
} from "@voltip/shared";

export interface HistoryBucket {
  count: number;
  /** Code points of the inserted text (what landed in the target app). */
  chars: number;
  /** Total recording time in this bucket. */
  spokenMs: number;
  /** Mean ASR + refine latency, rounded; `undefined` when the bucket is empty. */
  latencyMs: number | undefined;
}

export interface HistoryStats {
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

function bucket(entries: readonly HistoryEntry[]): HistoryBucket {
  let chars = 0;
  let spokenMs = 0;
  let latency = 0;
  for (const e of entries) {
    chars += textChars(e.text);
    spokenMs += e.duration_ms;
    latency += entryLatencyMs(e);
  }
  return {
    count: entries.length,
    chars,
    spokenMs,
    latencyMs: entries.length === 0 ? undefined : Math.round(latency / entries.length),
  };
}

/** Counts per weekday over the last six weeks, quantised to the heatmap's four levels. */
export function heatmapOf(entries: readonly HistoryEntry[], now: number): number[][] {
  const thisWeek = startOfWeek(now);
  const grid = Array.from({ length: HEATMAP_WEEKS }, () => Array.from({ length: 7 }, () => 0));
  for (const e of entries) {
    const weeksAgo = Math.round(dayDiff(thisWeek, startOfWeek(e.at_ms)) / 7);
    if (weeksAgo < 0 || weeksAgo >= HEATMAP_WEEKS) continue;
    const column = grid[HEATMAP_WEEKS - 1 - weeksAgo];
    const weekday = (new Date(e.at_ms).getDay() + 6) % 7;
    if (column) column[weekday] = (column[weekday] ?? 0) + 1;
  }
  return grid.map((col) => col.map((n) => Math.min(3, n)));
}

export function historyStats(entries: readonly HistoryEntry[], now: number): HistoryStats {
  const today = startOfDay(now);
  const week = startOfWeek(now);
  const month = startOfMonth(now);
  return {
    today: bucket(entries.filter((e) => e.at_ms >= today)),
    week: bucket(entries.filter((e) => e.at_ms >= week)),
    month: bucket(entries.filter((e) => e.at_ms >= month)),
    total: bucket(entries),
    heatmap: heatmapOf(entries, now),
  };
}

/** Rows matching a home tile / sidebar filter. */
export function filterHistory(
  entries: readonly HistoryEntry[],
  filter: HistoryFilter,
  now: number,
): HistoryEntry[] {
  switch (filter) {
    case "all":
      return [...entries];
    case "today":
      return entries.filter((e) => e.at_ms >= startOfDay(now));
    case "week":
      return entries.filter((e) => e.at_ms >= startOfWeek(now));
    case "month":
      return entries.filter((e) => e.at_ms >= startOfMonth(now));
    case "starred":
      return entries.filter((e) => e.starred);
    case "failed":
      return entries.filter((e) => e.outcome.kind !== "inserted");
  }
}

/** Case-insensitive search over the inserted text, the raw ASR text, the model ids, — for a take
 *  with a context (docs/dictation.md §18.6) — the app's name and id and the scene's name (a
 *  built-in scene's in both languages, §18.10), and — for a voice edit (§19.5) — its instruction and
 *  original selection. */
export function matchesHistoryQuery(entry: HistoryEntry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return true;
  const builtinNames =
    entry.scene?.builtin === undefined
      ? []
      : LOCALES.map((locale) => sceneLabel({ name: "", builtin: entry.scene?.builtin }, locale));
  return [
    ...builtinNames,
    entry.text,
    entry.raw_text,
    entry.asr_model,
    entry.refine_model ?? "",
    entry.app?.name ?? "",
    entry.app?.id ?? "",
    entry.scene?.name ?? "",
    entry.edit?.instruction ?? "",
    entry.edit?.selection ?? "",
  ].some((s) => s.toLowerCase().includes(q));
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

/** `3:47` minutes:seconds for spoken time (the 今日会话 panel). */
export function spokenLabel(ms: number): string {
  const total = Math.round(Math.max(0, ms) / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
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
