import { createTranslator } from "./i18n";
import { formatDuration } from "./labels";
import { MockBackend } from "./mock-backend";
import type { HistoryEntry } from "./schema";
import {
  HISTORY_FILTER_LABELS,
  type HistoryFilter,
  SAVED_TIME_FACTOR,
  clockLabel,
  dayDiff,
  dayLabel,
  emptyHomeStats,
  entryLatencyMs,
  groupByDay,
  historyFilterLabel,
  historyQueryArgs,
  homeStats,
  isHistoryFilter,
  recentTimeLabel,
  startOfDay,
  startOfMonth,
  startOfWeek,
  statsBoundaries,
  textChars,
  todayLabel,
} from "./history-stats";

/** Local Thursday 2026-09-24 12:00 — noon keeps every offset below inside a predictable day. */
const NOW = new Date(2026, 8, 24, 12, 0, 0).getTime();
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

function entry(overrides: Partial<HistoryEntry> & { at_ms: number }): HistoryEntry {
  return {
    id: `id-${overrides.at_ms}`,
    raw_text: "raw",
    text: "十个字十个字十个字十",
    refined: true,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    refine_model: "qwen/qwen3.8-27b",
    duration_ms: 6000,
    asr_ms: 400,
    refine_ms: 300,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...overrides,
  };
}

const ENTRIES: HistoryEntry[] = [
  entry({ at_ms: NOW - HOUR, starred: true }),
  entry({ at_ms: NOW - 3 * HOUR, text: "abc", refined: false, refine_ms: undefined, asr_ms: 800 }),
  entry({ at_ms: NOW - DAY, outcome: { kind: "clipboard", reason: "焦点丢失" } }),
  // Monday of this week is 2026-09-21; two days ago is Tuesday, still this week and month.
  entry({ at_ms: NOW - 2 * DAY, outcome: { kind: "failed", reason: "ASR 500" } }),
  // Last week (Thursday 2026-09-17): in the month, not the week.
  entry({ at_ms: NOW - 7 * DAY }),
  // 2026-08-30: neither this week nor this month, but within six weeks.
  entry({ at_ms: NOW - 25 * DAY }),
  // Far outside the heatmap window.
  entry({ at_ms: NOW - 60 * DAY }),
];

describe("history stats", () => {
  /** What the home page shows for `ENTRIES`: the core's answer (the mock has its semantics). */
  async function statsOf(entries: HistoryEntry[]) {
    const backend = new MockBackend({ history: entries });
    const boundaries = statsBoundaries(NOW);
    const stats = homeStats(await backend.historyStats(boundaries), boundaries, NOW);
    backend.destroy();
    return stats;
  }

  it("buckets today / this week / this month / total by the local calendar", async () => {
    const stats = await statsOf(ENTRIES);
    expect(stats.today.count).toBe(2);
    expect(stats.today.rawChars).toBe(3 + 3);
    expect(stats.today.spokenMs).toBe(12_000);
    expect(stats.today.savedMs).toBe(Math.round(12_000 * SAVED_TIME_FACTOR));
    // (400 + 300 + 800) / 2 = 750
    expect(stats.today.latencyMs).toBe(750);
    expect(stats.week.count).toBe(4);
    expect(stats.month.count).toBe(5);
    expect(stats.total.count).toBe(7);
    expect(stats.total.rawChars).toBe(3 * 7);
    // "raw" → the ten-character text: every character changed, and seven more added.
    expect(stats.today.correctedChars).toBe(10 + 3);
    expect(emptyHomeStats().today).toEqual({
      count: 0,
      rawChars: 0,
      correctedChars: 0,
      spokenMs: 0,
      savedMs: 0,
      latencyMs: undefined,
    });
    // Monday five weeks back to tomorrow, local midnights (a Thursday: 35 + 3 + 1 days).
    const boundaries = statsBoundaries(NOW);
    expect(boundaries).toHaveLength(40);
    expect(new Date(boundaries[0] ?? 0).getDay()).toBe(1);
    expect(boundaries.at(-1)).toBe(startOfDay(NOW) + 24 * HOUR);
  });

  it("builds the 6-week × 7-day heatmap from the daily buckets, quantised to four levels", async () => {
    const grid = (await statsOf(ENTRIES)).heatmap;
    expect(grid).toHaveLength(6);
    for (const col of grid) expect(col).toHaveLength(7);
    const thisWeek = grid[5];
    // Thursday = row 3 (Mon 0 … Sun 6): two entries today, one yesterday (Wed), one Tuesday.
    expect(thisWeek?.[3]).toBe(2);
    expect(thisWeek?.[2]).toBe(1);
    expect(thisWeek?.[1]).toBe(1);
    expect(grid[4]?.[3]).toBe(1);
    // 25 days back is Sunday 2026-08-30: week column 5 - 4 = 1, row 6.
    expect(grid[1]?.[6]).toBe(1);
    expect(grid.flat().reduce((a, b) => a + b, 0)).toBe(6);
    const busy = Array.from({ length: 5 }, (_, i) => entry({ at_ms: NOW - i * 60_000 }));
    expect((await statsOf(busy)).heatmap[5]?.[3]).toBe(3);
  });

  it("filters by tile, star and outcome; recognises filter names", async () => {
    const backend = new MockBackend({ history: ENTRIES });
    const ids = async (f: HistoryFilter) =>
      (await backend.historyQuery({ ...historyQueryArgs(f, "", NOW), limit: 100 })).entries.map(
        (e) => e.id,
      );
    expect(await ids("all")).toHaveLength(7);
    expect(await ids("today")).toHaveLength(2);
    expect(await ids("week")).toHaveLength(4);
    expect(await ids("month")).toHaveLength(5);
    expect(await ids("starred")).toEqual([`id-${NOW - HOUR}`]);
    expect(await ids("failed")).toHaveLength(2);
    expect(historyQueryArgs("today", " abc ", NOW)).toEqual({
      query: " abc ",
      sinceMs: startOfDay(NOW),
    });
    expect(historyQueryArgs("all", "  ", NOW)).toEqual({});
    expect(isHistoryFilter("today")).toBe(true);
    expect(isHistoryFilter("r1")).toBe(false);
    expect(isHistoryFilter(undefined)).toBe(false);
    backend.destroy();
  });

  it("labels days, clocks and spoken time in local time", () => {
    expect(clockLabel(NOW)).toBe("12:00:00");
    expect(dayLabel(NOW - HOUR, NOW)).toBe("今天 · 9月24日星期四");
    expect(dayLabel(NOW - DAY, NOW)).toBe("昨天 · 9月23日星期三");
    expect(dayLabel(NOW - 2 * DAY, NOW)).toBe("9月22日星期二");
    expect(todayLabel(NOW)).toBe("今天 · 09-24 周四");
    // English goes through Intl.DateTimeFormat("en-US") with Today / Yesterday from the dictionary.
    expect(dayLabel(NOW - HOUR, NOW, "en")).toBe("Today · Thu, Sep 24");
    expect(dayLabel(NOW - DAY, NOW, "en")).toBe("Yesterday · Wed, Sep 23");
    expect(dayLabel(NOW - 2 * DAY, NOW, "en")).toBe("Tue, Sep 22");
    expect(todayLabel(NOW, "en")).toBe("Today · 09-24 Thu");
    // Regression (English screenshot 2026-09-25): "Yesterday 12:00" overflowed the 104 px time
    // column and rendered as "Yesterday …"; English uses the date form, which fits like the rest.
    expect(recentTimeLabel(NOW - DAY, NOW, "en")).toBe("9-23 12:00");
    expect(recentTimeLabel(NOW - DAY, NOW, "en").length).toBeLessThanOrEqual(10);
    expect(groupByDay([], NOW, "en")).toEqual([]);
    expect(historyFilterLabel("starred")).toBe("已收藏");
    expect(historyFilterLabel("starred", createTranslator("en").t)).toBe("Starred");
    expect(HISTORY_FILTER_LABELS.failed).toBe("未插入");
    expect(recentTimeLabel(NOW - HOUR, NOW)).toBe("11:00:00");
    expect(recentTimeLabel(NOW - DAY, NOW)).toBe("昨天 12:00");
    expect(recentTimeLabel(NOW - 2 * DAY, NOW)).toBe("9-22 12:00");
    expect(formatDuration(227_000)).toBe("3 分 47 秒");
    expect(formatDuration(0)).toBe("0 秒");
    expect(textChars("👍a中")).toBe(3);
    expect(entryLatencyMs(entry({ at_ms: NOW, refine_ms: undefined }))).toBe(400);
  });

  it("groups newest-first rows by local day and computes calendar boundaries", () => {
    const groups = groupByDay(ENTRIES.slice(0, 4), NOW);
    expect(groups.map((g) => [g.day, g.items.length])).toEqual([
      ["今天 · 9月24日星期四", 2],
      ["昨天 · 9月23日星期三", 1],
      ["9月22日星期二", 1],
    ]);
    expect(startOfDay(NOW)).toBe(new Date(2026, 8, 24).getTime());
    expect(startOfWeek(NOW)).toBe(new Date(2026, 8, 21).getTime());
    expect(startOfWeek(new Date(2026, 8, 20, 23).getTime())).toBe(new Date(2026, 8, 14).getTime());
    expect(startOfMonth(NOW)).toBe(new Date(2026, 8, 1).getTime());
    expect(dayDiff(NOW, NOW - 3 * DAY)).toBe(3);
    expect(dayDiff(NOW - HOUR, NOW)).toBe(0);
  });
});
