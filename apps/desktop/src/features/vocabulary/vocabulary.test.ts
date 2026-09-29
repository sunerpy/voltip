import {
  type DictionaryEntry,
  type HistoryEntry,
  type ReplacementRule,
  createTranslator,
} from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import {
  dictionaryDraftProblem,
  errorText,
  hitTotals,
  moved,
  ruleDraftProblem,
  splitHeardAs,
} from "./vocabulary";

const EN = createTranslator("en").t;

function row(vocabulary?: HistoryEntry["vocabulary"]): HistoryEntry {
  return {
    id: "h",
    at_ms: 1,
    raw_text: "",
    text: "",
    refined: false,
    asr_model: "m",
    duration_ms: 1,
    asr_ms: 1,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...(vocabulary === undefined ? {} : { vocabulary }),
  };
}

const voltip: DictionaryEntry = {
  id: "e1",
  term: "Voltip",
  heard_as: [],
  enabled: true,
  source: { kind: "manual" },
  created_at_ms: 1,
  updated_at_ms: 1,
};

const rule: ReplacementRule = {
  id: "r1",
  name: "git push",
  kind: "literal",
  pattern: "p",
  replacement: "",
  case_sensitive: true,
  enabled: true,
  created_at_ms: 1,
  updated_at_ms: 1,
};

describe("vocabulary page helpers", () => {
  it("sums the hits of the history per id and kind (the core's history_hits)", async () => {
    const history = [
      row({ corrections: [{ id: "e1", count: 2 }], rules: [{ id: "r1", count: 1 }] }),
      row(),
      row({
        corrections: [
          { id: "e1", count: 1 },
          { id: "e2", count: 4 },
        ],
        rules: [],
      }),
    ];
    const backend = new MockBackend({ history });
    const hits = await backend.historyHits();
    expect([...hitTotals(hits, "corrections")]).toEqual([
      ["e1", 3],
      ["e2", 4],
    ]);
    expect([...hitTotals(hits, "rules")]).toEqual([["r1", 1]]);
    expect(hitTotals(undefined, "rules").size).toBe(0);
    backend.destroy();
  });

  it("splits the heard-as field on every documented separator", () => {
    expect(splitHeardAs("a, b，c、d · e;f；g\nh")).toEqual([
      "a",
      "b",
      "c",
      "d",
      "e",
      "f",
      "g",
      "h",
    ]);
    expect(splitHeardAs(" 听 写 ,, ")).toEqual(["听 写"]);
    expect(splitHeardAs("")).toEqual([]);
  });

  it("reports the rejection text of both backends", () => {
    // docs/frontend.md §8: the core's machine prefix is for the log, not the reader.
    expect(errorText("dictionary: x")).toBe("x");
    expect(errorText(new Error("rules: y"))).toBe("y");
    expect(errorText("没有前缀的原因")).toBe("没有前缀的原因");
  });

  it("checks a dictionary draft while typing in both locales", () => {
    expect(dictionaryDraftProblem("Teams", ["听写"], [voltip])).toBeUndefined();
    expect(dictionaryDraftProblem("  ", [], [])).toEqual({ term: "不能为空" });
    expect(dictionaryDraftProblem("x".repeat(65), [], [])).toEqual({
      term: "最多 64 个字符（当前 65）",
    });
    expect(dictionaryDraftProblem("VOLTIP", [], [voltip])).toEqual({ term: "词典里已有这个写法" });
    expect(
      dictionaryDraftProblem(
        "a",
        Array.from({ length: 11 }, (_, i) => `v${i}`),
        [],
      ),
    ).toEqual({
      heard: "最多 10 个曾听成（当前 11）",
    });
    expect(dictionaryDraftProblem("a", ["y".repeat(65)], [], EN)).toEqual({
      heard: "Up to 64 characters (currently 65)",
    });
    expect(dictionaryDraftProblem("", ["y".repeat(65)], [], EN)).toEqual({
      term: "Cannot be empty",
      heard: "Up to 64 characters (currently 65)",
    });
  });

  it("checks a rule draft while typing in both locales", () => {
    expect(ruleDraftProblem("new", "p", [rule])).toBeUndefined();
    expect(ruleDraftProblem(" ", "p", [])).toEqual({ name: "名称不能为空" });
    expect(ruleDraftProblem(" git push ", "p", [rule])).toEqual({ name: "已有同名规则" });
    expect(ruleDraftProblem("x", "", [], EN)).toEqual({ pattern: "The pattern cannot be empty" });
  });

  it("moves one id up or down and leaves the ends alone", () => {
    expect(moved(["a", "b", "c"], 1, -1)).toEqual(["b", "a", "c"]);
    expect(moved(["a", "b", "c"], 1, 1)).toEqual(["a", "c", "b"]);
    expect(moved(["a", "b"], 0, -1)).toEqual(["a", "b"]);
    expect(moved(["a", "b"], 1, 1)).toEqual(["a", "b"]);
  });
});
