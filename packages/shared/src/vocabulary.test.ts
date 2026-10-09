import {
  type DictionaryEntry,
  MAX_RULES,
  MAX_TEXT_BYTES,
  MAX_TOML_BYTES,
  NIL_ID,
  type ReplacementRule,
  type RuleDraft,
} from "./schema";
import {
  RULES_TOML_VERSION,
  Vocabulary,
  VocabularyError,
  checkDictionary,
  checkRules,
  compileRegex,
  expandReplacement,
  exportRulesToml,
  isWordChar,
  parseRulesToml,
  previewVocabulary,
  utf8Length,
  validateDictionaryDraft,
  validateRuleDraft,
} from "./vocabulary";

let seq = 0;
function entry(term: string, heardAs: string[], enabled = true): DictionaryEntry {
  seq += 1;
  return {
    id: `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}`,
    term,
    heard_as: heardAs,
    enabled,
    source: { kind: "manual" },
    created_at_ms: 1,
    updated_at_ms: 1,
  };
}

function rule(
  name: string,
  kind: RuleDraft["kind"],
  pattern: string,
  replacement: string,
  extra: Partial<ReplacementRule> = {},
): ReplacementRule {
  seq += 1;
  return {
    id: `00000000-0000-4000-9000-${String(seq).padStart(12, "0")}`,
    name,
    kind,
    pattern,
    replacement,
    case_sensitive: true,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
    ...extra,
  };
}

function draftOf(r: ReplacementRule): RuleDraft {
  return {
    name: r.name,
    kind: r.kind,
    pattern: r.pattern,
    replacement: r.replacement,
    case_sensitive: r.case_sensitive,
    enabled: r.enabled,
  };
}

function refusal(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error("expected a refusal");
}

describe("dictionary validation (mirrors voltip_core::vocabulary)", () => {
  it("normalises a draft: trims and drops empty variants and collapses duplicates ignoring ASCII case", () => {
    expect(
      validateDictionaryDraft({
        term: "  Voltip ",
        heard_as: [" 沃提普", "", "  ", "VOLT IP", "volt ip"],
        enabled: false,
      }),
    ).toEqual({ term: "Voltip", heard_as: ["沃提普", "VOLT IP"], enabled: false });
  });

  it("refuses a draft that is wrong on its own with the core words", () => {
    const cases: [Parameters<typeof validateDictionaryDraft>[0], string][] = [
      [{ term: " ", heard_as: [], enabled: true }, "dictionary: 正确写法不能为空"],
      [{ term: "x".repeat(65), heard_as: [], enabled: true }, "最多 64 个字符（当前 65）"],
      [{ term: "a\u0007b", heard_as: [], enabled: true }, "不能包含换行或控制字符"],
      [{ term: "a", heard_as: ["y".repeat(65)], enabled: true }, "误识别写法最多 64 个字符"],
      [{ term: "Teams", heard_as: ["Teams"], enabled: true }, "误识别写法「Teams」和正确写法相同"],
      [
        { term: "t", heard_as: Array.from({ length: 11 }, (_, i) => `v${i}`), enabled: true },
        "一个词条最多 10 个误识别写法（当前 11）",
      ],
    ];
    for (const [draft, needle] of cases) {
      const message = refusal(() => validateDictionaryDraft(draft));
      expect({
        message,
        ok: message.startsWith("dictionary: ") && message.includes(needle),
      }).toEqual({
        message,
        ok: true,
      });
    }
    expect(refusal(() => validateDictionaryDraft({ term: "", heard_as: [], enabled: true }))).toBe(
      "dictionary: 正确写法不能为空",
    );
    expect(new VocabularyError("x").name).toBe("VocabularyError");
  });

  it("checks the whole list and names the entry that was there first", () => {
    const voltip = entry("Voltip", ["沃提普"]);
    expect(() => {
      checkDictionary([voltip, entry("Teams", ["听写"])]);
    }).not.toThrow();
    const cases: [DictionaryEntry[], string][] = [
      [[voltip, entry("voltip", [])], "词典里已有「Voltip」"],
      [[voltip, entry("沃提普", [])], "「沃提普」已是「Voltip」的误识别写法，不能再作为正确写法"],
      [
        [voltip, entry("World", ["VOLTIP"])],
        "「VOLTIP」是词条「Voltip」的正确写法，不能再作为「World」的误识别写法",
      ],
      [[voltip, entry("World", ["沃提普"])], "「沃提普」已是「Voltip」的误识别写法"],
      [Array.from({ length: 501 }, (_, i) => entry(`t${i}`, [])), "词典最多 500 条"],
    ];
    for (const [list, needle] of cases)
      expect(refusal(() => checkDictionary(list))).toBe(`dictionary: ${needle}`);
  });
});

describe("rule validation (mirrors voltip_core::vocabulary)", () => {
  it("compiles regexes the core would and refuses what the Rust dialect lacks", () => {
    expect(compileRegex("(?<n>\\d+)", true).flags).toBe("gu");
    expect(compileRegex("abc", false).flags).toBe("giu");
    expect(refusal(() => compileRegex("(", true))).toMatch(/^正则无法编译：/);
    expect(refusal(() => compileRegex("a(?=b)", true))).toBe("正则无法编译：不支持环视与反向引用");
    expect(refusal(() => compileRegex("(a)\\1", true))).toBe("正则无法编译：不支持环视与反向引用");
  });

  it("validates a rule draft on its own and keeps the pattern and replacement as typed", () => {
    const ok = validateRuleDraft({
      name: "  PR 编号 ",
      kind: "regex",
      pattern: " \\bpr (\\d+)",
      replacement: "PR #$1 ",
      case_sensitive: false,
      enabled: true,
    });
    expect(ok).toEqual({
      name: "PR 编号",
      kind: "regex",
      pattern: " \\bpr (\\d+)",
      replacement: "PR #$1 ",
      case_sensitive: false,
      enabled: true,
    });
    const base: RuleDraft = {
      name: "r",
      kind: "literal",
      pattern: "p",
      replacement: "",
      case_sensitive: true,
      enabled: true,
    };
    const cases: [RuleDraft, string][] = [
      [{ ...base, name: "" }, "rules: 规则名称不能为空"],
      [{ ...base, pattern: "" }, "rules: 规则「r」的匹配内容不能为空"],
      [
        { ...base, pattern: "p".repeat(257) },
        "rules: 规则「r」的匹配内容最多 256 个字符（当前 257）",
      ],
      [
        { ...base, replacement: "q".repeat(257) },
        "rules: 规则「r」的替换内容最多 256 个字符（当前 257）",
      ],
    ];
    for (const [draft, message] of cases)
      expect(refusal(() => validateRuleDraft(draft))).toBe(message);
    expect(refusal(() => validateRuleDraft({ ...base, kind: "regex", pattern: "[" }))).toMatch(
      /^rules: 规则「r」：正则无法编译：/,
    );
    // A literal pattern is never compiled: `(` is plain text there.
    expect(validateRuleDraft({ ...base, pattern: "(" }).pattern).toBe("(");
  });

  it("checks the whole rule list: the cap and unique names", () => {
    expect(() => {
      checkRules([rule("a", "literal", "x", ""), rule("b", "literal", "x", "")]);
    }).not.toThrow();
    expect(
      refusal(() => checkRules([rule("a", "literal", "x", ""), rule("a", "regex", "y", "")])),
    ).toBe("rules: 已有名为「a」的规则");
    const many = Array.from({ length: MAX_RULES + 1 }, (_, i) => rule(`r${i}`, "literal", "p", ""));
    expect(refusal(() => checkRules(many))).toBe("rules: 规则最多 200 条");
  });
});

describe("matching (docs/dictation.md section 16.2)", () => {
  it("word characters are letters and digits and underscore outside the unspaced scripts", () => {
    const word = ["a", "Z", "7", "_", "é", "ß", "Ж"];
    const other = ["开", "あ", "カ", "한", "ไ", "𠀀", "，", " ", "-", "#"];
    expect(word.filter((c) => !isWordChar(c))).toEqual([]);
    expect(other.filter((c) => isWordChar(c))).toEqual([]);
  });

  it("corrects the sample mis-hearings with boundaries and ASCII case folding and longest match first", () => {
    const teams = entry("Teams", ["听写", "teems"]);
    const cat = entry("cat", ["kat"]);
    const good = entry("good idea", ["谷歌IDR", "谷歌"]);
    const off = entry("Off", ["关掉"], false);
    const vocab = Vocabulary.compile([teams, cat, good, off], []);
    // Longest first (`谷歌IDR` over `谷歌`), ASCII case folded (`TEEMS`), CJK has no boundary.
    const step = vocab.correct("我想用TEEMS和听写，谷歌IDR 真好，关掉它");
    expect(step.text).toBe("我想用Teams和Teams，good idea 真好，关掉它");
    expect(step.hits).toEqual([
      { id: teams.id, count: 2 },
      { id: good.id, count: 1 },
    ]);
    expect(step.error).toBeUndefined();
    // Boundaries for spaced scripts: `kat` never hits inside `katana`, but does between Han.
    expect(vocab.correct("katana 用kat命令 kat.").text).toBe("katana 用cat命令 cat.");
    // A match already spelled like the term is not a change; replaced text is never rescanned.
    const again = Vocabulary.compile([entry("A", ["乙"]), entry("乙乙", ["甲"])], []);
    expect(again.correct("甲乙").text).toBe("乙乙A");
    expect(Vocabulary.compile([teams], []).correct("Teams").hits).toEqual([]);
    // Nothing enabled: the text passes through.
    expect(Vocabulary.compile([off], []).correct("关掉")).toEqual({ text: "关掉", hits: [] });
    // Surrogate pairs are whole characters for the boundary rule.
    expect(Vocabulary.compile([entry("X", ["ab"])], []).correct("𠀀ab𠀀").text).toBe("𠀀X𠀀");
  });

  it("falls back to the input when the input or the output would pass the text limit", () => {
    const big = "a".repeat(MAX_TEXT_BYTES + 1);
    const vocab = Vocabulary.compile([entry("z", ["a"])], [rule("r", "literal", "开", "开开")]);
    expect(vocab.correct(big)).toEqual({
      text: big,
      hits: [],
      error: "文本超过 64 KiB，跳过词典纠正",
    });
    expect(vocab.applyRules(big).error).toBe("文本超过 64 KiB，跳过替换规则");
    // 11 000 × 3 bytes fits; doubled it does not (Han needs no boundary, so every one matches).
    const half = "开".repeat(11_000);
    const grow = Vocabulary.compile([entry("开开", ["开"])], []);
    expect(grow.correct(half)).toEqual({
      text: half,
      hits: [],
      error: "词典纠正让文本超过 64 KiB，本次不纠正",
    });
    expect(vocab.applyRules(half)).toEqual({
      text: half,
      hits: [],
      error: "规则「r」让文本超过 64 KiB，本次不应用替换规则",
    });
    expect(utf8Length("开a")).toBe(4);
  });

  it("runs the rules in order each on the output of the one before with the Rust replacement syntax", () => {
    const rules = [
      rule("pr", "regex", "\\bpr (\\d+)", "PR #$1", { case_sensitive: false }),
      rule("named", "regex", "(?<who>[A-Z][a-z]+) said", "${who}: $$ $who $1a $ $-"),
      rule("filler", "regex", "(嗯|啊)+[，,]?", ""),
      rule("empty", "regex", "x*", "Y"),
      rule("git", "literal", "给他push", "git push"),
      rule("chain", "literal", "git push", "git push --force-with-lease"),
      rule("ci", "literal", "hello", "Hello", { case_sensitive: false }),
      rule("off", "literal", "Hello", "nope", { enabled: false }),
      rule("broken", "regex", "(", "never"),
    ];
    const vocab = Vocabulary.compile([], rules);
    const step = vocab.applyRules("嗯，Ada said PR 12 给他push HELLO");
    expect(step.text).toBe("Ada: $ Ada  $ $- PR #12 git push --force-with-lease Hello");
    expect(step.hits.map((h) => rules.find((r) => r.id === h.id)?.name)).toEqual([
      "pr",
      "named",
      "filler",
      "git",
      "chain",
      "ci",
    ]);
    // A match whose expansion is the matched text itself is no change.
    const same = rule("same", "regex", "(PR)", "$1");
    expect(Vocabulary.compile([], [same]).applyRules("PR 1")).toEqual({ text: "PR 1", hits: [] });
    expect(Vocabulary.compile([], []).applyRules("same")).toEqual({ text: "same", hits: [] });
  });

  it("expands group references like the regex crate", () => {
    const match = /(?<a>x)(y)?/u.exec("x");
    if (match === null) throw new Error("no match");
    expect(expandReplacement("[$1][$2][${a}][$a][$9][${1}][$$][$][$}]", match)).toBe(
      "[x][][x][x][][x][$][$][$}]",
    );
    expect(expandReplacement("plain", match)).toBe("plain");
  });
});

describe("preview (vocabulary_preview)", () => {
  const dictionary = [entry("good idea", ["谷歌IDR"])];
  const saved = rule("app", "literal", "app", "App");

  it("runs the dictionary then the rules with a draft replacing its rule or appended under the nil id", () => {
    const plain = previewVocabulary(dictionary, [saved], "一个谷歌IDR的app");
    expect(plain).toEqual({
      corrected: "一个good idea的app",
      output: "一个good idea的App",
      corrections: [{ id: dictionary[0]?.id, count: 1 }],
      rules: [{ id: saved.id, count: 1 }],
    });
    const edited = previewVocabulary(dictionary, [saved], "app", {
      id: saved.id,
      rule: { ...draftOf(saved), replacement: "APP" },
    });
    expect(edited.output).toBe("APP");
    expect(edited.rules).toEqual([{ id: saved.id, count: 1 }]);
    const added = previewVocabulary(dictionary, [saved], "app", {
      rule: { ...draftOf(saved), name: "new", pattern: "App", replacement: "Ap" },
    });
    expect(added.output).toBe("Ap");
    expect(added.rules).toEqual([
      { id: saved.id, count: 1 },
      { id: NIL_ID, count: 1 },
    ]);
    const unknownId = "11111111-1111-4111-8111-111111111111";
    expect(
      previewVocabulary([], [], "a", {
        id: unknownId,
        rule: { ...draftOf(saved), pattern: "a", replacement: "b" },
      }).rules,
    ).toEqual([{ id: unknownId, count: 1 }]);
  });

  it("refuses an invalid draft or an oversized text and reports a step that fell back", () => {
    expect(
      refusal(() =>
        previewVocabulary([], [], "a", {
          id: null,
          rule: { ...draftOf(saved), kind: "regex", pattern: "(" },
        }),
      ),
    ).toMatch(/^rules: 规则「app」：正则无法编译/);
    expect(refusal(() => previewVocabulary([], [], "a".repeat(MAX_TEXT_BYTES + 1)))).toBe(
      "rules: 试写文本最多 64 KiB",
    );
    const grow = previewVocabulary([], [rule("g", "literal", "开", "开开")], "开".repeat(11_000));
    expect(grow.error).toBe("规则「g」让文本超过 64 KiB，本次不应用替换规则");
  });
});

describe("TOML exchange format (docs/dictation.md section 16.5)", () => {
  const rules = [
    rule("git push", "literal", "给他push", "git push"),
    rule("PR 编号", "regex", "\\bpr (\\d+)", "PR #$1", { case_sensitive: false }),
  ];

  it("exports exactly what the core exports and round-trips every string form", () => {
    expect(exportRulesToml(rules)).toBe(
      [
        "# Voltip 替换规则 · docs/dictation.md §16",
        "version = 1",
        "",
        "[[rule]]",
        'name = "git push"',
        'kind = "literal"',
        'pattern = "给他push"',
        'replacement = "git push"',
        "case_sensitive = true",
        "enabled = true",
        "",
        "[[rule]]",
        'name = "PR 编号"',
        'kind = "regex"',
        "pattern = '\\bpr (\\d+)'",
        'replacement = "PR #$1"',
        "case_sensitive = false",
        "enabled = true",
        "",
      ].join("\n"),
    );
    expect(exportRulesToml([])).toBe(
      `# Voltip 替换规则 · docs/dictation.md §16\nversion = ${RULES_TOML_VERSION}\n`,
    );
    const tricky = [
      ...rules,
      rule('quote "x"', "literal", "it's", "it is", { enabled: false }),
      rule("both", "regex", "a'b\"c\\d", ""),
      rule("control", "literal", "a\u0001b", "tab\there\nnext\r"),
    ];
    const text = exportRulesToml(tricky);
    expect(text).toContain("name = 'quote \"x\"'");
    expect(text).toContain('pattern = "a\'b\\"c\\\\d"');
    expect(text).toContain('pattern = "a\\u0001b"');
    expect(parseRulesToml(text)).toEqual(tricky.map(draftOf));
  });

  it("parses the forms the Rust exporter writes with comments and CRLF and the documented defaults", () => {
    const text = [
      "# comment",
      "version = 1 # trailing",
      "",
      "[[ rule ]]",
      "name = 'a'",
      'pattern = """',
      'x\\ty"',
      '\\u00e9\\U0001F600"""',
      "",
      "[[rule]]",
      '"name" = "b"',
      "kind = 'regex'",
      "pattern = '''a'b'''",
      'replacement = "\\b\\f\\r\\n\\\\\\""',
      "case_sensitive = false",
      "enabled = false",
    ].join("\r\n");
    expect(parseRulesToml(text)).toEqual([
      {
        name: "a",
        kind: "literal",
        pattern: 'x\ty"\r\né😀',
        replacement: "",
        case_sensitive: true,
        enabled: true,
      },
      {
        name: "b",
        kind: "regex",
        pattern: "a'b",
        replacement: '\b\f\r\n\\"',
        case_sensitive: false,
        enabled: false,
      },
    ]);
    expect(parseRulesToml("version = 1\n")).toEqual([]);
    expect(parseRulesToml("version = +1")).toEqual([]);
  });

  it("refuses a bad import with its position and accepts nothing from it", () => {
    const cases: [string, string][] = [
      [
        'version = 1\n[[rule]]\nname = "a"\npattern = "x\n',
        "TOML parse error at line 4: newline in a single-line string",
      ],
      [
        'version = 1\n[[rule]]\nname = "a"\npattern = "x',
        "TOML parse error at line 4: unterminated string",
      ],
      [
        'version = 1\n[[rule]]\nname = "a"\npatern = "x"\n',
        "line 4: unknown field `patern`, expected one of `name`",
      ],
      ["version = 2\n", "rules: 不支持的 version = 2（应为 1）"],
      ['version = "1"\n', "line 1: invalid type for `version`, expected an integer"],
      ['[[rule]]\nname = "a"\npattern = "x"\n', "missing field `version`"],
      ['version = 1\n[[rule]]\nname = "a"\n', "line 2: missing field `pattern`"],
      [
        'version = 1\n[[rule]]\nname = 1\npattern = "x"\n',
        "line 3: invalid type for `name`, expected a string",
      ],
      [
        'version = 1\n[[rule]]\nname = "a"\npattern = "x"\nenabled = "yes"\n',
        "line 5: invalid type for `enabled`, expected a boolean",
      ],
      [
        'version = 1\n[[rule]]\nname = "a"\nkind = "glob"\npattern = "x"\n',
        "line 4: unknown variant `glob`, expected `literal` or `regex`",
      ],
      ["version = 1\nextra = 2\n", "line 2: unknown field `extra`, expected `version` or `rule`"],
      ["version = 1\nversion = 1\n", "line 2: duplicate key `version`"],
      ["version = 1\n[rule]\n", "line 2: only [[rule]] tables are supported"],
      ['version = 1\n[[rule]]\nname = "a" pattern = "x"\n', "line 3: unexpected `p` after a value"],
      ['version = 1\n[[rule]]\nname = "\\q"\n', "line 3: invalid escape `\\q`"],
      ['version = 1\n[[rule]]\nname = "\\u12"\n', "line 3: invalid escape `\\u`"],
      ["version = 1\n[[rule]]\nname = [1]\n", "line 3: a value is expected"],
      ["version = 1\n[[rule]]\nname = maybe\n", "line 3: invalid value `maybe`"],
      ["version = 1\n[[rule]]\nname\n", "line 3: `=` is expected after `name`"],
      ["version = 1\n= 2\n", "line 2: a key is expected"],
      [
        'version = 1\n[[rule]]\nname = "ok"\npattern = "x"\n[[rule]]\nname = "bad"\nkind = "regex"\npattern = "("\n',
        "rules: 第 2 条（bad）：规则「bad」：正则无法编译",
      ],
      [
        'version = 1\n[[rule]]\nname = "a"\npattern = "x"\n[[rule]]\nname = " a "\npattern = "y"\n',
        "rules: 第 2 条：名称「a」在文件里重复",
      ],
    ];
    const misses = cases.flatMap(([text, needle]) => {
      const message = refusal(() => parseRulesToml(text));
      return message.startsWith("rules: ") && message.includes(needle)
        ? []
        : [`${text} → ${message}`];
    });
    expect(misses).toEqual([]);
    const many = [
      "version = 1",
      ...Array.from({ length: MAX_RULES + 1 }, (_, i) => `[[rule]]\nname = "r${i}"\npattern = "p"`),
    ].join("\n");
    expect(refusal(() => parseRulesToml(many))).toBe("rules: 文件里有 201 条规则，最多 200 条");
    expect(refusal(() => parseRulesToml("#".repeat(MAX_TOML_BYTES + 1)))).toBe(
      "rules: TOML 文本最多 256 KiB",
    );
  });
});
