import {
  MAX_PAIR_DIFF_CELLS,
  type TextMark,
  type WordSegmenter,
  diffItems,
  diffRuns,
  excerptSegments,
  fixScope,
  fixStatus,
  fixStatusLines,
  fixStatusTone,
  fixTarget,
  planDictionary,
  planRule,
  polishPairs,
  prefillFor,
  prefillPair,
  ruleNameFor,
  shortForm,
  snippetParts,
  takePolishPairs,
  tapPiece,
  textMarks,
  trimRange,
  wordAt,
  wordPieces,
  wordSegmenter,
} from "./quick-fix";
import { createTranslator } from "./i18n";
import {
  type DictionaryEntry,
  type HistoryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_RULE_NAME_CHARS,
  type ReplacementRule,
} from "./schema";

/** The board's sample take (Penpot 「快速纠错」 01): two wordings the polish fixed, one it kept. */
const RAW =
  "下午用维他命测了千问三的流式识别，延迟比上周低了一半，结果发到非书群了，记得把配置同步到酷伯内特斯集群。";
const TEXT =
  "下午用 Voltip 测了千问三的流式识别，延迟比上周低了一半，结果发到非书群了，记得把配置同步到 Kubernetes 集群。";
const TAKE = { raw_text: RAW, text: TEXT };
const PAIRS = polishPairs(RAW, TEXT);
/** Every replacement of a take, as `takePolishPairs` leaves them when the polish made them all. */
const pairsOf = (take: { raw_text: string; text: string }) => polishPairs(take.raw_text, take.text);

let seq = 0;
function entry(
  term: string,
  heard: string[],
  extra: Partial<DictionaryEntry> = {},
): DictionaryEntry {
  seq += 1;
  return {
    id: `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}`,
    term,
    heard_as: heard,
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: 1,
    updated_at_ms: 1,
    ...extra,
  };
}

function rule(name: string): ReplacementRule {
  seq += 1;
  return {
    id: `00000000-0000-4000-9000-${String(seq).padStart(12, "0")}`,
    name,
    kind: "literal",
    pattern: "x",
    replacement: "y",
    case_sensitive: true,
    enabled: true,
    source: { kind: "manual" },
    pending: false,
    created_at_ms: 1,
    updated_at_ms: 1,
  };
}

const at = (text: string, word: string, from = 0) => {
  const start = text.indexOf(word, from);
  return [start, start + word.length] as const;
};

describe("trimRange", () => {
  it("drops white space and punctuation at both ends, by code point, and nothing else", () => {
    const text = " ，维他命。 x";
    expect(trimRange(text, 0, 6)).toEqual({ start: 2, end: 5 });
    expect(trimRange(text, 6, 0)).toEqual({ start: 2, end: 5 });
    expect(trimRange(text, -3, 99)).toEqual({ start: 2, end: 8 });
    expect(trimRange("，。 ", 0, 3)).toBeUndefined();
    expect(trimRange("「𠀀」", 0, 4)).toEqual({ start: 1, end: 3 });
    expect(trimRange("a.b", 0, 3)).toEqual({ start: 0, end: 3 });
  });
});

describe("the polish's replacements (polishPairs, diffItems)", () => {
  it("pairs a deletion with the addition after it, trimmed, with the offsets in both texts", () => {
    const pairs = polishPairs(RAW, TEXT);
    expect(pairs.map((p) => [p.heard, p.polished])).toEqual([
      ["维他命", "Voltip"],
      ["酷伯内特斯", "Kubernetes"],
    ]);
    const [first] = pairs;
    expect(first && RAW.slice(first.start, first.end)).toBe("维他命");
    expect(first && TEXT.slice(first.polishedStart, first.polishedEnd)).toBe("Voltip");
    expect(diffRuns(RAW, TEXT)[0]).toEqual({
      kind: "same",
      text: "下午用",
      rawStart: 0,
      textStart: 0,
    });
  });

  it("leaves out punctuation-only changes, lone deletions and additions, and white space changes", () => {
    expect(polishPairs("你好，世界", "你好。世界")).toEqual([]);
    expect(polishPairs("嗯我们开会", "我们开会")).toEqual([]);
    expect(polishPairs("我们开会", "我们开会。")).toEqual([]);
    expect(polishPairs("Qwen3 ASR", "Qwen3ASR")).toEqual([
      expect.objectContaining({ heard: "Qwen3 ASR", polished: "Qwen3ASR" }),
    ]);
    expect(polishPairs("用 Voltip 了", "用Voltip了")).toEqual([]);
  });

  it("gives up on a diff past the cap rather than stalling", () => {
    const side = Math.ceil(Math.sqrt(MAX_PAIR_DIFF_CELLS)) + 1;
    expect(polishPairs("维".repeat(side), "他".repeat(side))).toEqual([]);
    expect(polishPairs("维".repeat(10), "他".repeat(10))).toHaveLength(1);
  });

  it("groups each replacement of the diff view and keeps the other runs as they are", () => {
    const raw = "嗯下午用维他命测，好";
    const text = "下午用 Voltip 测。好";
    const items = diffItems(raw, text, polishPairs(raw, text));
    expect(items.map((i) => i.kind)).toEqual(["del", "same", "pair", "same", "del", "add", "same"]);
    const pair = items[2];
    expect(
      pair?.kind === "pair" && [pair.del, pair.add, pair.pair.heard, pair.pair.polished],
    ).toEqual(["维他命", " Voltip ", "维他命", "Voltip"]);
  });
});

describe("marks on the original text (textMarks)", () => {
  it("marks the polish's replacements and the enabled entries' forms; a form wins over a replacement", () => {
    const marks = textMarks(RAW, PAIRS, []);
    expect(marks.map((m) => [m.kind, m.text, m.pair?.polished])).toEqual([
      ["polish", "维他命", "Voltip"],
      ["polish", "酷伯内特斯", "Kubernetes"],
    ]);
    const voltip = entry("Voltip", ["维他命"]);
    const off = entry("飞书", ["非书"], { enabled: false });
    const known = textMarks(RAW, PAIRS, [voltip, off]);
    expect(known.map((m) => [m.kind, m.text])).toEqual([
      ["known", "维他命"],
      ["polish", "酷伯内特斯"],
    ]);
    expect(known[0]?.entry).toBe(voltip);
    expect(known[0] && [known[0].start, known[0].end]).toEqual(at(RAW, "维他命"));
  });
});

describe("the polish's own replacements (takePolishPairs)", () => {
  // The final text is the polished text after the dictionary and the rules (§16.9 3.1): a change
  // the vocabulary made is not the polish's, whatever the diff of the two texts shows.
  const literal = (pattern: string, replacement: string, extra: Partial<ReplacementRule> = {}) => ({
    ...rule(`${pattern} → ${replacement}`),
    pattern,
    replacement,
    ...extra,
  });
  const fired = (corrections: { id: string }[], rules: { id: string }[]) => ({
    corrections: corrections.map(({ id }) => ({ id, count: 1 })),
    rules: rules.map(({ id }) => ({ id, count: 1 })),
  });

  it("regression: a take that was not polished has none, so no 润色时已改正 mark and no pre-filled spelling", () => {
    // Codex review 2026-10-13: with the polish off, a rule's 非书 → 飞书 showed as a replacement
    // the polish made and filled the card's right spelling 「来自润色结果」.
    const feishu = literal("非书", "飞书");
    const take = {
      raw_text: "结果发到非书群了",
      text: "结果发到飞书群了",
      refined: false,
      vocabulary: fired([], [feishu]),
    };
    const pairs = takePolishPairs(take, [], [feishu]);
    expect(pairs).toEqual([]);
    expect(textMarks(take.raw_text, pairs, [])).toEqual([]);
    expect(diffItems(take.raw_text, take.text, pairs).some((i) => i.kind === "pair")).toBe(false);
    const [s, e] = at(take.raw_text, "非书");
    const target = fixTarget(take, "raw", s, e, pairs);
    expect(target?.pair).toBeUndefined();
    expect(prefillFor(target ?? fail())).toEqual({ kind: "dictionary", heard: "非书", term: "" });
    // Not polished and nothing fired: still none.
    expect(takePolishPairs({ ...take, vocabulary: undefined }, [], [])).toEqual([]);
  });

  it("regression: in a polished take a rule's change is left out and the polish's own stays", () => {
    const feishu = literal("非书", "飞书");
    const take = {
      raw_text: "结果发到非书群了，用维他命测",
      text: "结果发到飞书群了，用 Voltip 测。",
      refined: true,
      vocabulary: fired([], [feishu]),
    };
    // Han text diffs by the character: the rule's change is 非 → 飞, which the rule's pattern
    // never matches alone.
    expect(polishPairs(take.raw_text, take.text).map((p) => p.heard)).toEqual(["非", "维他命"]);
    const pairs = takePolishPairs(take, [], [feishu]);
    expect(pairs.map((p) => [p.heard, p.polished])).toEqual([["维他命", "Voltip"]]);
    expect(textMarks(take.raw_text, pairs, []).map((m) => [m.kind, m.text])).toEqual([
      ["polish", "维他命"],
    ]);
    // A rule turned off since still made that change; a rule that did not fire made none.
    expect(takePolishPairs(take, [], [{ ...feishu, enabled: false }])).toEqual(pairs);
    expect(takePolishPairs({ ...take, vocabulary: fired([], []) }, [], [feishu])).toHaveLength(2);
    expect(takePolishPairs({ ...take, vocabulary: undefined }, [], [feishu])).toHaveLength(2);
  });

  it("leaves out a correction's change, even of an entry turned off since", () => {
    const voltip = entry("Voltip", ["维他命"], { enabled: false });
    const take = {
      raw_text: "用维他命测",
      text: "用 Voltip 测。",
      refined: true,
      vocabulary: fired([voltip], []),
    };
    expect(takePolishPairs(take, [voltip], [])).toEqual([]);
    expect(takePolishPairs(take, [], [])).toHaveLength(1);
  });

  it("leaves out a rule's change that needs the text around it", () => {
    // `(\d+)块` never matches the pair's own wording 块: the whole original text shows the rule
    // made the same replacement there.
    const yuan = literal("(\\d+)块", "${1}元", { kind: "regex" });
    const take = {
      raw_text: "一共30块钱",
      text: "一共30元钱。",
      refined: true,
      vocabulary: fired([], [yuan]),
    };
    expect(polishPairs(take.raw_text, take.text).map((p) => [p.heard, p.polished])).toEqual([
      ["块", "元"],
    ]);
    expect(takePolishPairs(take, [], [yuan])).toEqual([]);
  });
});

describe("the selection (fixTarget)", () => {
  it("in the original: trimmed, grown to the whole replacement it lies in, a dictionary entry by default", () => {
    const [s, e] = at(RAW, "他命");
    const target = fixTarget(TAKE, "raw", s, e, PAIRS);
    expect(target).toMatchObject({
      view: "raw",
      text: "维他命",
      polishOnly: false,
      kind: "dictionary",
    });
    expect(target?.pair?.polished).toBe("Voltip");
    expect(prefillFor(target ?? fail())).toEqual({
      kind: "dictionary",
      heard: "维他命",
      term: "Voltip",
      termFrom: "polish",
    });
    const [fs, fe] = at(RAW, "非书群了，");
    const fei = fixTarget(TAKE, "raw", fs, fe, PAIRS);
    expect(fei).toMatchObject({ text: "非书群了", kind: "dictionary" });
    expect(fei?.pair).toBeUndefined();
    expect(prefillFor(fei ?? fail())).toEqual({ kind: "dictionary", heard: "非书群了", term: "" });
    // Partly over a replacement: not grown.
    const [ps, pe] = at(RAW, "用维他");
    expect(fixTarget(TAKE, "raw", ps, pe, PAIRS)).toMatchObject({ text: "用维他" });
    expect(fixTarget(TAKE, "raw", 16, 17, PAIRS)).toBeUndefined();
  });

  it("in the polished text: words the original has are a dictionary entry, words only the polish wrote a rule", () => {
    const [s, e] = at(TEXT, "非书");
    expect(fixTarget(TAKE, "polished", s, e, PAIRS)).toMatchObject({
      text: "非书",
      polishOnly: false,
      kind: "dictionary",
    });
    const [vs, ve] = at(TEXT, "olt");
    const voltip = fixTarget(TAKE, "polished", vs, ve, PAIRS);
    expect(voltip).toMatchObject({ text: "Voltip", polishOnly: true, kind: "rule" });
    expect(prefillFor(voltip ?? fail())).toEqual({ kind: "rule", heard: "Voltip", term: "" });
    // Over an addition that is not a replacement: polish only.
    const added = { raw_text: "我们开会", text: "我们下午开会" };
    expect(fixTarget(added, "polished", 1, 4, pairsOf(added))).toMatchObject({
      text: "们下午",
      polishOnly: true,
      kind: "rule",
    });
    // An added space or comma does not make the words the polish's.
    const spaced = { raw_text: "用Voltip测", text: "用 Voltip，测" };
    expect(fixTarget(spaced, "polished", 0, 11, pairsOf(spaced))).toMatchObject({
      polishOnly: false,
      kind: "dictionary",
    });
  });

  it("a selection over 64 characters or across lines cannot be a dictionary form", () => {
    const long = { raw_text: "长".repeat(70), text: "长".repeat(70) };
    expect(fixTarget(long, "raw", 0, 65, [])).toMatchObject({ block: "tooLong", kind: "rule" });
    expect(fixTarget(long, "raw", 0, 64, [])).toMatchObject({ kind: "dictionary" });
    expect(fixTarget(long, "raw", 0, 64, [])?.block).toBeUndefined();
    const lines = { raw_text: "第一行\n第二行", text: "第一行\n第二行" };
    expect(fixTarget(lines, "raw", 0, 7, [])).toMatchObject({ block: "oneLine", kind: "rule" });
  });

  it("the diff view's question pre-fills either spelling", () => {
    const [pair] = PAIRS;
    expect(prefillPair(pair ?? fail(), "polished")).toEqual({
      kind: "dictionary",
      heard: "维他命",
      term: "Voltip",
      termFrom: "polish",
    });
    expect(prefillPair(pair ?? fail(), "raw")).toEqual({
      kind: "dictionary",
      heard: "",
      term: "维他命",
      termFrom: "raw",
    });
  });
});

describe("risk and the checks before saving", () => {
  it("a one-character form or up to two ASCII letters is short", () => {
    expect(shortForm(["维他命", "维"])).toEqual({ form: "维", chars: 1, latin: false });
    expect(shortForm([" ab "])).toEqual({ form: "ab", chars: 2, latin: true });
    expect(shortForm(["A"])).toEqual({ form: "A", chars: 1, latin: true });
    expect(shortForm(["の"])).toEqual({ form: "の", chars: 1, latin: false });
    expect(shortForm(["abc", "12", "维他"])).toBeUndefined();
    expect(shortForm([])).toBeUndefined();
  });

  it("a new entry is ready; the fields' own problems come from the dictionary page's checks", () => {
    const plan = planDictionary(" Voltip ", "维他命, 沃尔提普 · 维他命", []);
    expect(plan).toEqual({
      heardAs: ["维他命", "沃尔提普"],
      added: ["维他命", "沃尔提普"],
      draft: { term: "Voltip", heard_as: ["维他命", "沃尔提普"], enabled: true },
      ready: true,
    });
    expect(planDictionary("", "维他命", []).problem?.term).toBe("不能为空");
    expect(planDictionary("", "维他命", []).ready).toBe(false);
    expect(planDictionary("", "维他命", []).conflict).toBeUndefined();
    // Before a right spelling is typed, a form the dictionary has is said at once (5.2).
    const voltip = entry("Voltip", ["沃尔提普"]);
    expect(planDictionary("", "voltip", [voltip]).conflict).toEqual({
      kind: "exists",
      entry: voltip,
    });
    expect(planDictionary("", "沃尔提普", [voltip]).conflict).toEqual({
      kind: "taken",
      heard: "沃尔提普",
      owner: voltip,
    });
    expect(planDictionary("", "长".repeat(65), [voltip]).problem?.heard).toMatch(/64/);
    expect(planDictionary("", "长".repeat(65), [voltip]).conflict).toBeUndefined();
    const en = createTranslator("en").t;
    expect(planDictionary("x".repeat(65), "", [], en).problem?.term).toMatch(/64/);
  });

  it("a right spelling the dictionary has already merges the forms into that entry", () => {
    const voltip = entry("Voltip", ["沃尔提普", "沃提普"], { enabled: false });
    const plan = planDictionary("voltip", "维他命", [voltip]);
    expect(plan.merge).toBe(voltip);
    expect(plan.draft).toEqual({
      term: "Voltip",
      heard_as: ["沃尔提普", "沃提普", "维他命"],
      enabled: false,
    });
    expect(plan.ready).toBe(true);
    // Only the new forms are added; nothing new is a clash.
    const some = planDictionary("Voltip", "维他命 · 沃提普", [voltip]);
    expect(some.draft.heard_as).toEqual(["沃尔提普", "沃提普", "维他命"]);
    expect([some.heardAs, some.added]).toEqual([["维他命", "沃提普"], ["维他命"]]);
    expect(planDictionary("Voltip", "沃提普", [voltip]).conflict).toEqual({
      kind: "taken",
      heard: "沃提普",
      owner: voltip,
    });
    expect(planDictionary("Voltip", " ", [voltip]).conflict).toEqual({
      kind: "exists",
      entry: voltip,
    });
    const full = entry(
      "Full",
      Array.from({ length: MAX_HEARD_AS }, (_, i) => `f${i}`),
    );
    expect(planDictionary("Full", "新的", [full]).conflict).toEqual({
      kind: "tooMany",
      entry: full,
      max: MAX_HEARD_AS,
    });
  });

  it("refuses what the core refuses: a form another entry has or spells, the same spelling, a spelling that is a form, a full dictionary", () => {
    const vitamin = entry("维生素", ["维他命"]);
    const qwen = entry("Qwen", []);
    const dictionary = [vitamin, qwen];
    expect(planDictionary("Voltip", "维他命", dictionary)).toMatchObject({
      conflict: { kind: "taken", heard: "维他命", owner: vitamin },
      ready: false,
    });
    expect(planDictionary("Voltip", "qwen", dictionary).conflict).toEqual({
      kind: "isTerm",
      heard: "qwen",
      owner: qwen,
    });
    expect(planDictionary("Voltip", "Voltip", []).conflict).toEqual({
      kind: "same",
      heard: "Voltip",
    });
    expect(planDictionary("Voltip", "voltip", []).ready).toBe(true);
    expect(planDictionary("维他命", "", dictionary).conflict).toEqual({
      kind: "termTaken",
      term: "维他命",
      owner: vitamin,
    });
    const many = Array.from({ length: MAX_DICTIONARY_ENTRIES }, (_, i) => entry(`词${i}`, []));
    expect(planDictionary("新词", "", many).conflict).toEqual({
      kind: "full",
      max: MAX_DICTIONARY_ENTRIES,
    });
    expect(planDictionary("词1", "听成", many).ready).toBe(true);
  });

  it("names a rule after its find and replace, short and unique", () => {
    expect(ruleNameFor(" 非书 ", "飞书", [])).toBe("非书 → 飞书");
    expect(ruleNameFor("非书", "", [])).toBe("非书 →");
    expect(ruleNameFor("非书", "飞书", [rule("非书 → 飞书"), rule("非书 → 飞书 (2)")])).toBe(
      "非书 → 飞书 (3)",
    );
    const long = ruleNameFor("长".repeat(80), "短", []);
    expect(Array.from(long)).toHaveLength(MAX_RULE_NAME_CHARS);
    const taken = ruleNameFor("长".repeat(80), "短", [rule(long)]);
    expect(Array.from(taken)).toHaveLength(MAX_RULE_NAME_CHARS);
    expect(taken.endsWith(" (2)")).toBe(true);
  });

  it("checks a rule draft like the rules page", () => {
    const fields = {
      find: "非书",
      replace: "飞书",
      name: " 非书 → 飞书 ",
      kind: "literal" as const,
      caseSensitive: true,
    };
    expect(planRule(fields, [])).toEqual({
      draft: {
        name: "非书 → 飞书",
        kind: "literal",
        pattern: "非书",
        replacement: "飞书",
        case_sensitive: true,
        enabled: true,
      },
      ready: true,
    });
    expect(planRule(fields, [rule("非书 → 飞书")]).problem?.name).toBe("已有同名规则");
    expect(planRule({ ...fields, find: "" }, []).problem?.pattern).toBe("匹配内容不能为空");
  });
});

describe("the card's status rows (fixStatus)", () => {
  const backtest = (scanned: number, matched: number, kept: number) => ({
    scanned,
    matched,
    kept,
    examples: [],
  });
  const voltip = entry("Voltip", ["沃尔提普"]);

  it("a clash or a refusal is shown alone", () => {
    const conflict = { kind: "same" as const, heard: "x" };
    expect(fixStatus({ conflict, heard: "x", backtest: backtest(5, 1, 1) })).toEqual([
      { kind: "conflict", conflict },
    ]);
    expect(fixStatus({ error: "rules: 正则无法编译", heard: "x", merge: voltip })).toEqual([
      { kind: "error", message: "rules: 正则无法编译" },
    ]);
  });

  it("otherwise the effect and one risk, or the merge and the effect, at most two rows", () => {
    expect(fixStatus({ heard: "维他命", backtest: "computing" })).toEqual([{ kind: "computing" }]);
    expect(fixStatus({ heard: "维他命", backtest: backtest(500, 3, 0) })).toEqual([
      { kind: "impact", scanned: 500, matched: 3 },
    ]);
    expect(
      fixStatus({ heard: "维他命", backtest: backtest(500, 3, 2), short: shortForm(["维"]) }),
    ).toEqual([
      { kind: "impact", scanned: 500, matched: 3 },
      { kind: "kept", kept: 2 },
    ]);
    expect(
      fixStatus({ heard: "维", backtest: backtest(500, 0, 0), short: shortForm(["维"]) }),
    ).toEqual([
      { kind: "impact", scanned: 500, matched: 0 },
      { kind: "short", short: { form: "维", chars: 1, latin: false } },
    ]);
    expect(fixStatus({ heard: "维他命", merge: voltip, backtest: backtest(500, 3, 0) })).toEqual([
      { kind: "merge", entry: voltip, heard: "维他命" },
      { kind: "impact", scanned: 500, matched: 3 },
    ]);
    expect(fixStatus({ heard: "维", merge: voltip, backtest: backtest(500, 3, 1) })).toEqual([
      { kind: "impact", scanned: 500, matched: 3 },
      { kind: "kept", kept: 1 },
    ]);
    // Nothing else in the history: no effect row.
    expect(
      fixStatus({
        heard: "维",
        merge: voltip,
        backtest: backtest(0, 0, 0),
        short: shortForm(["维"]),
      }),
    ).toEqual([
      { kind: "merge", entry: voltip, heard: "维" },
      { kind: "short", short: { form: "维", chars: 1, latin: false } },
    ]);
    expect(fixStatus({ heard: "维他命" })).toEqual([]);
  });

  it("words the rows, links the effect to its examples and a clash to the dictionary", () => {
    const owner = entry("维生素", ["维他命"]);
    const lines = (rows: Parameters<typeof fixStatusLines>[0], isRule = false, examples = 2) =>
      fixStatusLines(rows, { heard: "维他命", rule: isRule, examples });
    expect(
      lines([{ kind: "conflict", conflict: { kind: "taken", heard: "维他命", owner } }]),
    ).toEqual([
      { tone: "error", text: "「维他命」已是词条「维生素」的曾听成写法。", link: "openDictionary" },
    ]);
    expect(
      lines([{ kind: "conflict", conflict: { kind: "isTerm", heard: "维他命", owner } }]),
    ).toEqual([
      {
        tone: "error",
        text: "「维他命」是另一个词条的正确写法，不能作为曾听成写法。",
        link: "openDictionary",
      },
    ]);
    expect(
      lines([{ kind: "conflict", conflict: { kind: "termTaken", term: "维他命", owner } }]),
    ).toEqual([
      {
        tone: "error",
        text: "「维他命」已是词条「维生素」的曾听成写法，不能作为正确写法。",
        link: "openDictionary",
      },
    ]);
    expect(lines([{ kind: "conflict", conflict: { kind: "exists", entry: voltip } }])).toEqual([
      { tone: "error", text: "「Voltip」已是词典中的正确写法。", link: "viewEntry" },
    ]);
    expect(
      lines([
        { kind: "conflict", conflict: { kind: "same", heard: "x" } },
        { kind: "conflict", conflict: { kind: "tooMany", entry: voltip, max: MAX_HEARD_AS } },
        { kind: "conflict", conflict: { kind: "full", max: MAX_DICTIONARY_ENTRIES } },
        { kind: "error", message: "rules: 正则无法编译" },
      ]),
    ).toEqual([
      { tone: "error", text: "曾听成写法与正确写法相同。" },
      { tone: "error", text: `该词条的曾听成写法已达 ${MAX_HEARD_AS} 个。` },
      { tone: "error", text: `词典最多 ${MAX_DICTIONARY_ENTRIES} 个词条，已达到上限。` },
      { tone: "error", text: "rules: 正则无法编译" },
    ]);
    expect(
      lines([
        { kind: "merge", entry: voltip, heard: "维他命" },
        { kind: "computing" },
        { kind: "impact", scanned: 500, matched: 3 },
      ]),
    ).toEqual([
      {
        tone: "info",
        text: "「Voltip」已在词典中，将把「维他命」加入它的曾听成写法（现有 1 个）。",
      },
      { tone: "busy", text: "正在计算影响范围…" },
      {
        tone: "info",
        text: "除本条外，最近 500 条记录中有 3 条含「维他命」。",
        link: "show",
      },
    ]);
    // With a kept-forms row the link moves to it; without examples there is none.
    expect(
      lines([
        { kind: "impact", scanned: 500, matched: 3 },
        { kind: "kept", kept: 1 },
      ]),
    ).toEqual([
      { tone: "info", text: "除本条外，最近 500 条记录中有 3 条含「维他命」。" },
      {
        tone: "warn",
        text: "其中 1 条的最终文本仍为「维他命」，可能是正确用法。",
        link: "show",
      },
    ]);
    expect(
      lines(
        [
          { kind: "impact", scanned: 500, matched: 0 },
          { kind: "short", short: { form: "维", chars: 1, latin: false } },
        ],
        false,
        0,
      ),
    ).toEqual([
      { tone: "info", text: "除本条外，最近 500 条记录中都没有「维他命」。" },
      { tone: "warn", text: "写法只有 1 个字，容易误改其他词语。" },
    ]);
    expect(
      lines(
        [
          { kind: "impact", scanned: 499, matched: 1 },
          { kind: "short", short: { form: "ab", chars: 2, latin: true } },
        ],
        true,
      ),
    ).toEqual([
      { tone: "info", text: "除本条外，最近 499 条记录中有 1 条会被改动。", link: "show" },
      { tone: "warn", text: "写法只有 2 个字母，容易误改其他词语。" },
    ]);
    expect(lines([{ kind: "impact", scanned: 499, matched: 0 }], true, 0)).toEqual([
      { tone: "info", text: "除本条外，最近 499 条记录都不会被改动。" },
    ]);
    // In English, through the translator it is given.
    const en = createTranslator("en").t;
    expect(
      fixStatusLines([{ kind: "computing" }], { heard: "x", rule: false, examples: 0 }, en),
    ).toEqual([{ tone: "busy", text: "Checking the effect…" }]);
  });

  it("the box takes its worst row's tone", () => {
    expect(fixStatusTone([])).toBe("info");
    expect(fixStatusTone([{ tone: "busy", text: "" }])).toBe("info");
    expect(
      fixStatusTone([
        { tone: "info", text: "" },
        { tone: "warn", text: "" },
      ]),
    ).toBe("warn");
    expect(
      fixStatusTone([
        { tone: "warn", text: "" },
        { tone: "error", text: "" },
      ]),
    ).toBe("error");
  });

  it("cuts the rule preview to the changes and twelve characters around them", () => {
    const after = TEXT.replace("非书", "飞书");
    expect(excerptSegments(TEXT, after)).toEqual([
      { kind: "same", text: "…比上周低了一半，结果发到" },
      { kind: "del", text: "非" },
      { kind: "add", text: "飞" },
      { kind: "same", text: "书群了，记得把配置同步到…" },
    ]);
    expect(excerptSegments("非书", "飞书")).toEqual([
      { kind: "del", text: "非" },
      { kind: "add", text: "飞" },
      { kind: "same", text: "书" },
    ]);
    expect(excerptSegments("same", "same")).toEqual([]);
    // Past the diff cap: one stretch from the first difference to the last.
    const long = "维".repeat(1001);
    expect(excerptSegments(`${long}x`, `${long}y`)).toEqual([
      { kind: "same", text: `…${"维".repeat(12)}` },
      { kind: "del", text: "x" },
      { kind: "add", text: "y" },
    ]);
    expect(excerptSegments(`${long}x`, long)).toEqual([
      { kind: "same", text: `…${"维".repeat(12)}` },
      { kind: "del", text: "x" },
    ]);
  });

  it("splits an example around what the draft changes", () => {
    expect(snippetParts("…今天用维他命跑了", "…今天用Voltip跑了")).toEqual([
      "…今天用",
      "维他命",
      "跑了",
    ]);
    expect(snippetParts("非书群", "飞书群")).toEqual(["", "非", "书群"]);
    expect(snippetParts("same", "same")).toEqual(["same", "", ""]);
  });
});

describe("where the quick fix applies (fixScope)", () => {
  const take = (extra: Partial<HistoryEntry>): Pick<HistoryEntry, "kind" | "origin"> => ({
    kind: "dictation",
    ...extra,
  });
  const phone = (kind: NonNullable<HistoryEntry["origin"]>["kind"]) =>
    take({ origin: { device: "Pixel 8", kind } });

  it("the desktop fixes its own takes and the phone's takes it recognised, and explains the phone's own", () => {
    expect(fixScope(take({}), "desktop")).toEqual({ kind: "fix" });
    expect(fixScope(phone("take"), "desktop")).toEqual({ kind: "fix" });
    expect(fixScope(phone("standalone"), "desktop")).toEqual({ kind: "note", note: "phone" });
    expect(fixScope(phone("typed"), "desktop")).toEqual({ kind: "none" });
    expect(fixScope(phone("clipboard"), "desktop")).toEqual({ kind: "none" });
    expect(fixScope(phone("sent"), "desktop")).toEqual({ kind: "none" });
    expect(fixScope(take({ kind: "edit" }), "desktop")).toEqual({ kind: "none" });
  });

  it("the phone fixes its own takes and sends the ones the computer recognised there", () => {
    expect(fixScope(take({}), "phone")).toEqual({ kind: "fix" });
    expect(fixScope(phone("sent"), "phone")).toEqual({ kind: "note", note: "computer" });
    expect(fixScope(phone("standalone"), "phone")).toEqual({ kind: "none" });
  });
});

describe("words (wordPieces, wordAt, tapPiece)", () => {
  const mark: TextMark = { kind: "polish", start: 3, end: 6, text: "维他命" };

  it("with a segmenter: words as it finds them, each mark one piece", () => {
    const segmenter = wordSegmenter();
    expect(segmenter).not.toBeNull();
    const pieces = wordPieces("下午用维他命测了 Voltip，好", [mark], segmenter);
    const words = pieces.filter((p) => p.word).map((p) => p.text);
    expect(words).toContain("维他命");
    expect(words).toContain("Voltip");
    expect(pieces.find((p) => p.text === "维他命")?.mark).toBe(mark);
    expect(pieces.find((p) => p.text === "，")?.word).toBe(false);
    expect(pieces.map((p) => p.text).join("")).toBe("下午用维他命测了 Voltip，好");
  });

  it("without one (Hermes): Han and kana by the character, other words whole", () => {
    const pieces = wordPieces("用维他命测 good-idea's v2，テスト", [], null);
    expect(pieces.map((p) => [p.text, p.word])).toEqual([
      ["用", true],
      ["维", true],
      ["他", true],
      ["命", true],
      ["测", true],
      [" ", false],
      ["good-idea's", true],
      [" ", false],
      ["v2", true],
      ["，", false],
      ["テ", true],
      ["ス", true],
      ["ト", true],
    ]);
    const marked = wordPieces("下午用维他命测", [mark], null);
    expect(marked.map((p) => p.text)).toEqual(["下", "午", "用", "维他命", "测"]);
    // A mark that starts inside an earlier one is skipped.
    expect(
      wordPieces("下午用维他命测", [mark, { ...mark, start: 4 }], null).map((p) => p.text),
    ).toEqual(["下", "午", "用", "维他命", "测"]);
  });

  it("the word at an offset, on every platform the same", () => {
    const fake: WordSegmenter = {
      segment: (text) => [
        { segment: text.slice(0, 2), index: 0, isWordLike: true },
        { segment: text.slice(2, 3), index: 2, isWordLike: false },
        { segment: text.slice(3), index: 3 },
      ],
    };
    expect(wordAt("下午，开会", 1, fake)).toEqual({ start: 0, end: 2 });
    expect(wordAt("下午，开会", 2, fake)).toBeUndefined();
    expect(wordAt("下午，开会", 4, fake)).toEqual({ start: 3, end: 5 });
    expect(wordAt("Voltip 测", 2, null)).toEqual({ start: 0, end: 6 });
    expect(wordAt("Voltip 测", 6, null)).toBeUndefined();
    expect(wordAt("Voltip 测", 99, null)).toBeUndefined();
  });

  it("no Intl.Segmenter (Hermes): no segmenter, and the words fall back to the script rules", () => {
    const real = Object.getOwnPropertyDescriptor(Intl, "Segmenter");
    Reflect.deleteProperty(Intl, "Segmenter");
    try {
      expect(wordSegmenter()).toBeNull();
      expect(wordPieces("测good").map((p) => p.text)).toEqual(["测", "good"]);
    } finally {
      if (real !== undefined) Reflect.defineProperty(Intl, "Segmenter", real);
    }
    expect(wordSegmenter()).not.toBeNull();
  });

  it("taps select a word, extend to another, clear inside the selection; a mark selects itself whole", () => {
    const pieces = wordPieces("下午用维他命测了", [mark], null);
    const piece = (text: string) => pieces.find((p) => p.text === text) ?? fail();
    let selection = tapPiece(undefined, piece("午"));
    expect(selection).toEqual({ start: 1, end: 2 });
    selection = tapPiece(selection, piece("测"));
    expect(selection).toEqual({ start: 1, end: 7 });
    expect(tapPiece(selection, piece("用"))).toBeUndefined();
    expect(tapPiece(selection, piece("下"))).toEqual({ start: 0, end: 7 });
    expect(tapPiece(selection, piece("维他命"))).toEqual({ start: 3, end: 6 });
    expect(tapPiece({ start: 3, end: 6 }, piece("维他命"))).toBeUndefined();
    const comma = { start: 0, end: 1, text: "，", word: false };
    expect(tapPiece(selection, comma)).toBe(selection);
  });
});

function fail(): never {
  throw new Error("unexpected undefined");
}
