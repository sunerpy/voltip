import {
  CORRECTION_PIECE_CHARS,
  correctedChars,
  countsForStats,
  historyHitsOf,
  historyPageOf,
  historyStatsOf,
  matchesHistoryQuery,
} from "./history-queries";
import { HISTORY_QUERY_LIMIT, HISTORY_STATS_BOUNDARIES, type HistoryEntry } from "./schema";

const NOW = new Date(2026, 8, 24, 12, 0, 0).getTime();
const HOUR = 3_600_000;

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

describe("history queries of the mock backend (the core's semantics)", () => {
  it("searches text, raw text and model ids case-insensitively", () => {
    const e = entry({ at_ms: NOW, text: "把 fetchUser 改成 async", raw_text: "fetch user" });
    expect(matchesHistoryQuery(e, "")).toBe(true);
    expect(matchesHistoryQuery(e, "FETCHUSER")).toBe(true);
    expect(matchesHistoryQuery(e, "fetch user")).toBe(true);
    expect(matchesHistoryQuery(e, "qwen3-asr")).toBe(true);
    expect(matchesHistoryQuery(e, "qwen3.8")).toBe(true);
    expect(matchesHistoryQuery(e, "zzz")).toBe(false);
    expect(matchesHistoryQuery({ ...e, refine_model: undefined }, "qwen3.8")).toBe(false);
  });

  it("regression: searches the app and the scene of a take with a context (docs/dictation.md section 18.6)", () => {
    const e = entry({ at_ms: NOW, text: "好的", raw_text: "好的" });
    const inSlack = {
      ...e,
      app: { id: "com.tinyspeck.slackmacgap", name: "Slack" },
      scene: { id: "00000000-0000-4000-a000-000000000001", name: "聊天" },
    };
    expect(matchesHistoryQuery(inSlack, "slack")).toBe(true);
    expect(matchesHistoryQuery(inSlack, "tinyspeck")).toBe(true);
    expect(matchesHistoryQuery(inSlack, "聊天")).toBe(true);
    expect(matchesHistoryQuery(e, "slack")).toBe(false);
    expect(matchesHistoryQuery({ ...inSlack, scene: undefined }, "聊天")).toBe(false);
  });

  it("finds a built-in scene by its name in either language", () => {
    const coding = entry({
      at_ms: NOW,
      scene: { id: "00000000-0000-4000-a000-000000000002", name: "编程开发", builtin: "coding" },
    });
    expect(matchesHistoryQuery(coding, "coding")).toBe(true);
    expect(matchesHistoryQuery(coding, "编程开发")).toBe(true);
    expect(matchesHistoryQuery(coding, "office writing")).toBe(false);
  });

  it("counts dictations only, and the characters the clean-up changed", () => {
    expect(countsForStats(entry({ at_ms: NOW }))).toBe(true);
    expect(countsForStats(entry({ at_ms: NOW, kind: "edit" }))).toBe(false);
    const phone = (kind: "take" | "typed" | "clipboard") =>
      entry({ at_ms: NOW, origin: { device: "Pixel 8", kind } });
    expect(countsForStats(phone("take"))).toBe(true);
    expect(countsForStats(phone("typed"))).toBe(false);
    expect(countsForStats(phone("clipboard"))).toBe(false);
    // The same cases as the core's `corrected_chars` test.
    expect(correctedChars("嗯那个明天开会", "明天开会。")).toBe(4);
    expect(correctedChars("hello world", "Hello, world.")).toBe(3);
    expect(correctedChars("相同", "相 同")).toBe(0);
    expect(correctedChars("", "新加的")).toBe(3);
    const long = "语音听写".repeat(1500);
    const changed = `${long.slice(0, 3000)}改${long.slice(3001)}`;
    expect(long.length).toBeGreaterThan(2 * CORRECTION_PIECE_CHARS);
    expect(correctedChars(long, changed)).toBe(1);
  });

  it("filters, searches and pages newest first, with the counts of every page", () => {
    const rows = [
      entry({ at_ms: NOW - HOUR, starred: true, text: "Hello World" }),
      entry({ at_ms: NOW, outcome: { kind: "failed", reason: "x" }, text: "失败的" }),
      entry({ at_ms: NOW - 2 * HOUR, outcome: { kind: "clipboard", reason: "x" } }),
    ];
    const texts = (args: Parameters<typeof historyPageOf>[1]) =>
      historyPageOf(rows, args).entries.map((e) => e.at_ms);
    expect(texts({ limit: 10 })).toEqual([NOW, NOW - HOUR, NOW - 2 * HOUR]);
    expect(texts({ limit: 10, sinceMs: NOW - HOUR })).toEqual([NOW, NOW - HOUR]);
    expect(texts({ limit: 10, starred: true })).toEqual([NOW - HOUR]);
    expect(texts({ limit: 10, failed: true })).toEqual([NOW, NOW - 2 * HOUR]);
    expect(texts({ limit: 10, query: " hello " })).toEqual([NOW - HOUR]);
    expect(historyPageOf(rows, { limit: 1, offset: 1 })).toEqual({
      entries: [rows[0]],
      matching: 3,
      total: 3,
    });
    for (const limit of [0, HISTORY_QUERY_LIMIT + 1, 1.5]) {
      expect(() => historyPageOf(rows, { limit })).toThrow(/limit/);
    }
  });

  it("adds up dictations between local midnights (23 h and 25 h days too) and in total", () => {
    const boundaries = [0, 24 * HOUR, 47 * HOUR, 72 * HOUR];
    const dictation = (at_ms: number, raw_text: string, text: string) =>
      entry({ at_ms, raw_text, text, duration_ms: 2000, asr_ms: 300, refine_ms: 200 });
    const rows = [
      dictation(HOUR, "嗯明天开会", "明天开会。"),
      dictation(46 * HOUR, "你好", "你好。"),
      dictation(47 * HOUR, "早上好", "早上好。"),
      dictation(80 * HOUR, "以后的", "以后的。"),
      { ...dictation(2 * HOUR, "编辑", "编辑后"), kind: "edit" as const },
    ];
    const day = (count: number, raw_chars: number, corrected_chars: number) => ({
      count,
      raw_chars,
      corrected_chars,
      spoken_ms: 2000 * count,
      latency_ms: 500 * count,
    });
    // The same numbers as the core's `stats_add_up_dictations_between_the_boundaries…` test.
    expect(historyStatsOf(rows, boundaries)).toEqual({
      buckets: [day(1, 5, 2), day(1, 2, 1), day(1, 3, 1)],
      total: day(4, 13, 5),
    });
    for (const bad of [[], [5], [5, 5], [9, 3]]) {
      expect(() => historyStatsOf(rows, bad)).toThrow(/boundaries/);
    }
    const tooMany = Array.from({ length: HISTORY_STATS_BOUNDARIES + 1 }, (_, i) => i);
    expect(() => historyStatsOf(rows, tooMany)).toThrow(/boundaries/);
  });

  it("adds up the hits of every entry per dictionary entry and rule", () => {
    const fired = (corrections: number, rules: number) => ({
      corrections: [{ id: "word", count: corrections }],
      rules: [{ id: "rule", count: rules }],
    });
    const rows = [
      entry({ at_ms: 1, vocabulary: fired(2, 1) }),
      entry({ at_ms: 2, vocabulary: fired(3, 0) }),
      entry({ at_ms: 3 }),
    ];
    expect(historyHitsOf(rows)).toEqual({ dictionary: { word: 5 }, rules: { rule: 1 } });
    expect(historyHitsOf([])).toEqual({ dictionary: {}, rules: {} });
  });
});
