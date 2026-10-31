import { describe, expect, it } from "vitest";
import {
  type DictionaryEntry,
  type HistoryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_SUGGESTIONS,
  type ReplacementRule,
  historyEntrySchema,
} from "./schema";
import {
  type AcceptedSuggestion,
  acceptSuggestion,
  ruleWarnings,
  suggestVocabulary,
  suggestionDraft,
  suggestionTarget,
} from "./suggestions";

let seq = 0;
const nextId = () => {
  seq += 1;
  return `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}`;
};

function entry(
  term: string,
  heard: string[],
  extra: Partial<DictionaryEntry> = {},
): DictionaryEntry {
  return {
    id: nextId(),
    term,
    heard_as: heard,
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: 1,
    updated_at_ms: 1,
    ...extra,
  };
}

function rule(
  name: string,
  pattern: string,
  replacement: string,
  extra: Partial<ReplacementRule> = {},
): ReplacementRule {
  return {
    id: nextId(),
    name,
    kind: "literal",
    pattern,
    replacement,
    case_sensitive: true,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
    source: { kind: "manual" },
    pending: false,
    ...extra,
  };
}

function take(raw: string, text: string, atMs: number, refined = true): HistoryEntry {
  return historyEntrySchema.parse({
    id: nextId(),
    at_ms: atMs,
    raw_text: raw,
    text,
    refined,
    asr_model: "fake",
    duration_ms: 1000,
    asr_ms: 100,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
  });
}

const TAKES = [
  take("我们用维他命开会", "我们用 Voltip 开会。", 50),
  take("发到非书群", "发到飞书群。", 40),
  take("又是非书", "又是飞书。", 30),
  take("库伯内特斯部署", "Kubernetes 部署", 20),
  take("没有润色的非书", "没有润色的飞书", 10, false),
];

const heardOf = (dictionary: DictionaryEntry[], ignored = []) =>
  suggestVocabulary(dictionary, [], TAKES, ignored).pairs.map((p) => [
    p.heard,
    p.polished,
    p.count,
  ]);

function accepted(result: ReturnType<typeof acceptSuggestion>): AcceptedSuggestion {
  if ("kind" in result) throw new Error(`refused: ${JSON.stringify(result)}`);
  return result;
}

describe("learning suggestions (docs/dictation.md §16.10)", () => {
  it("counts the clean-up's pairs, most frequent first and then the newest", () => {
    const out = suggestVocabulary([], [], TAKES, []);
    expect([out.scanned, out.refined, out.skipped, out.total]).toEqual([5, 4, 0, 3]);
    expect(heardOf([])).toEqual([
      ["非", "飞", 2],
      ["维他命", "Voltip", 1],
      ["库伯内特斯", "Kubernetes", 1],
    ]);
    expect(out.pairs[0]?.last_at_ms).toBe(40);
    expect(out.pairs[0]?.examples.map((e) => e.history_id)).toEqual([TAKES[1]?.id, TAKES[2]?.id]);
    expect(out.pairs[1]?.examples[0]?.snippet).toBe("我们用维他命开会");
    const one = suggestVocabulary([], [], TAKES, [], 1);
    expect([one.total, one.pairs.length]).toEqual([3, 1]);
  });

  it("leaves out what an enabled entry corrects and the ignored pairs, matched exactly", () => {
    const dictionary = [entry("Voltip", ["维他命"]), entry("飞书", ["非"], { enabled: false })];
    const ignored = [{ heard: "库伯内特斯", polished: "Kubernetes", at_ms: 1 }];
    expect(suggestVocabulary(dictionary, [], TAKES, ignored).pairs.map((p) => p.heard)).toEqual([
      "非",
    ]);
    const otherCase = [{ heard: "库伯内特斯", polished: "kubernetes", at_ms: 1 }];
    expect(suggestVocabulary([], [], TAKES, otherCase).total).toBe(3);
  });

  it("counts a take too long to compare as skipped", () => {
    const long = take("字".repeat(1001), "子".repeat(1001), 5);
    expect(suggestVocabulary([], [], [long], [])).toMatchObject({ skipped: 1, total: 0 });
  });

  it("stops comparing older takes once the comparison budget is spent, as the core does", () => {
    // Each polished take costs (3 + 1) × (1 + 1) = 8 cells; a take not polished costs nothing.
    const takes = [
      take("维他命", "Voltip", 50),
      take("没有润色", "没有润色", 45, false),
      take("非书", "飞书", 40),
      take("维他命", "Voltip", 30),
    ];
    const out = suggestVocabulary([], [], takes, [], MAX_SUGGESTIONS, 20);
    expect([out.scanned, out.refined, out.total]).toEqual([3, 2, 2]);
    expect(out.pairs.map((p) => [p.heard, p.count])).toEqual([
      ["维他命", 1],
      ["非", 1],
    ]);
    expect(suggestVocabulary([], [], takes, []).scanned).toBe(4);
  });

  it("snippets cut the recognised text around what was heard", () => {
    const raw = `${"前".repeat(20)}维他命${"后".repeat(20)}`;
    const text = `${"前".repeat(20)}Voltip${"后".repeat(20)}`;
    const [pair] = suggestVocabulary([], [], [take(raw, text, 1)], []).pairs;
    expect(pair?.examples[0]?.snippet).toBe(`…${"前".repeat(12)}维他命${"后".repeat(12)}…`);
  });
});

describe("加入词典 on a suggestion", () => {
  it("makes an entry from the history, or adds to the one with the spelling", () => {
    const kube = entry("kubernetes", ["酷伯"]);
    const created = accepted(acceptSuggestion([kube], [], "维他命", "Voltip", "h1", 100, nextId));
    expect(created.before).toBeUndefined();
    expect(created.after).toMatchObject({
      term: "Voltip",
      heard_as: ["维他命"],
      enabled: true,
      source: { kind: "history", history_id: "h1" },
      created_at_ms: 100,
    });
    expect(created.dictionary).toHaveLength(2);
    const added = accepted(
      acceptSuggestion([kube], [], "库伯内特斯", "Kubernetes", "h2", 200, nextId),
    );
    expect(added.before).toBe(kube);
    expect(added.after).toMatchObject({
      term: "kubernetes",
      heard_as: ["酷伯", "库伯内特斯"],
      updated_at_ms: 200,
    });
    expect(suggestionTarget([kube], "KUBERNETES")).toBe(kube);
  });

  it("refuses, naming what is in the way", () => {
    const many = entry(
      "Voltip",
      Array.from({ length: MAX_HEARD_AS }, (_, i) => `沃${i}`),
    );
    const off = entry("飞书", [], { enabled: false });
    expect(acceptSuggestion([many], [], "维他命", "Voltip", "h", 2, nextId)).toEqual({
      kind: "entry",
      entry_id: many.id,
      term: "Voltip",
      clash: "too_many",
    });
    expect(acceptSuggestion([off], [], "非书", "飞书", "h", 2, nextId)).toMatchObject({
      clash: "disabled",
    });
    const term = entry("维他命", []);
    const taken = entry("Vitamin", ["维他名"], { enabled: false });
    const wolt = entry("Wolt", ["voltip"]);
    expect(acceptSuggestion([term], [], "维他命", "Vitamin C", "h", 2, nextId)).toMatchObject({
      clash: "heard_is_term",
    });
    expect(acceptSuggestion([taken], [], "维他名", "Vitamin C", "h", 2, nextId)).toMatchObject({
      clash: "heard_taken",
    });
    expect(acceptSuggestion([wolt], [], "沃提普", "Voltip", "h", 2, nextId)).toMatchObject({
      clash: "term_taken",
    });
    const voltip = entry("Voltip", ["维他命"]);
    expect(acceptSuggestion([voltip], [], "维他命", "Voltip", "h", 2, nextId)).toMatchObject({
      clash: "heard_taken",
    });
    const full = Array.from({ length: MAX_DICTIONARY_ENTRIES }, (_, i) => entry(`词${i}`, []));
    expect(acceptSuggestion(full, [], "维他命", "Voltip", "h", 2, nextId)).toEqual({
      kind: "full",
      max: 500,
    });
    expect(acceptSuggestion([], [], "同", "同", "h", 2, nextId)).toMatchObject({ kind: "failed" });
  });

  it("refuses when a rule that runs rewrites the spelling", () => {
    const back = rule("改回去", "Voltip", "维他命");
    expect(acceptSuggestion([], [back], "维他命", "Voltip", "h", 2, nextId)).toEqual({
      kind: "rule",
      rule_id: back.id,
      name: "改回去",
      becomes: "维他命",
    });
    const off = { ...back, enabled: false };
    expect("kind" in acceptSuggestion([], [off], "维他命", "Voltip", "h", 2, nextId)).toBe(false);
  });

  it("backtests the entry it would save", () => {
    const voltip = entry("Voltip", ["沃提普"], { enabled: false });
    expect(suggestionDraft([voltip], "维他命", "voltip")).toEqual({
      kind: "dictionary",
      id: voltip.id,
      entry: { term: "Voltip", heard_as: ["沃提普", "维他命"], enabled: false },
    });
    expect(suggestionDraft([voltip], "沃提普", "Voltip").entry.heard_as).toEqual(["沃提普"]);
    expect(suggestionDraft([], "非", "飞")).toEqual({
      kind: "dictionary",
      id: null,
      entry: { term: "飞", heard_as: ["非"], enabled: true },
    });
  });
});

describe("the rules page's hints", () => {
  it("names duplicates, inverses and rules that undo an entry", () => {
    const a = rule("非书", "非书", "飞书");
    const b = rule("非书 2", "非书", "飞书 Lark");
    const c = rule("飞书 → 非书", "飞书", "非书");
    const d = rule("Voltip", "Voltip", "维他命");
    const voltip = entry("Voltip", ["维他命"]);
    expect(ruleWarnings([a, b, c, d], [voltip])).toEqual([
      { kind: "duplicate", rule_id: a.id, other_id: b.id, other_name: b.name },
      { kind: "inverse", rule_id: a.id, other_id: c.id, other_name: c.name },
      { kind: "duplicate", rule_id: b.id, other_id: a.id, other_name: a.name },
      { kind: "inverse", rule_id: c.id, other_id: a.id, other_name: a.name },
      { kind: "undoes_entry", rule_id: d.id, entry_id: voltip.id, term: "Voltip", heard: "维他命" },
    ]);
  });

  it("leaves out near misses and what does not run", () => {
    expect(ruleWarnings([rule("Foo", "Foo", "x"), rule("foo", "foo", "x")], [])).toEqual([]);
    expect(
      ruleWarnings(
        [rule("Foo", "Foo", "x"), rule("Foo 2", "Foo", "x", { case_sensitive: false })],
        [],
      ),
    ).toEqual([]);
    expect(
      ruleWarnings([rule("r", "非书", "飞书", { kind: "regex" }), rule("l", "飞书", "非书")], []),
    ).toEqual([]);
    expect(
      ruleWarnings([rule("a", "非书", "飞书"), rule("b", "飞书", "非书", { enabled: false })], []),
    ).toEqual([]);
    expect(
      ruleWarnings([rule("a", "非书", "飞书"), rule("b", "非书", "飞书", { pending: true })], []),
    ).toEqual([]);
    expect(
      ruleWarnings(
        [rule("v", "Voltip", "维他命")],
        [entry("Voltip", ["维他命"], { enabled: false })],
      ),
    ).toEqual([]);
    expect(ruleWarnings([rule("v", "Voltip", "VOLTIP")], [entry("Voltip", ["维他命"])])).toEqual(
      [],
    );
    const same = ruleWarnings([rule("same", "same", "same"), rule("same 2", "same", "same")], []);
    expect(same.map((w) => w.kind)).toEqual(["duplicate", "duplicate"]);
  });
});
