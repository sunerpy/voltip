// The personal dictionary and the replacement rules as the core runs them (docs/dictation.md §16),
// for the in-memory backend: the same limits and refusal texts, the literal matcher (ASCII case
// folding, word boundaries only for scripts written with spaces, leftmost-longest without
// rescanning replaced text), the rule chain, the preview and the TOML exchange format. Regex rules
// run on the JavaScript engine here, so the Rust `regex` dialect of the core stays the reference:
// the desktop app always asks the core (`vocabulary_preview`), never this module.
import {
  type DictionaryDraft,
  type DictionaryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_PATTERN_CHARS,
  MAX_REPLACEMENT_CHARS,
  MAX_RULE_NAME_CHARS,
  MAX_RULES,
  MAX_TERM_CHARS,
  MAX_TEXT_BYTES,
  MAX_TOML_BYTES,
  NIL_ID,
  type PreviewDraft,
  type ReplacementRule,
  type RuleDraft,
  type RuleKind,
  type VocabularyHit,
  type VocabularyPreview,
} from "./schema";

/** A refusal carrying the core's text (`dictionary: …` / `rules: …`). */
export class VocabularyError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "VocabularyError";
  }
}

const dictErr = (message: string) => new VocabularyError(`dictionary: ${message}`);
const rulesErr = (message: string, cause?: unknown) =>
  new VocabularyError(`rules: ${message}`, cause === undefined ? undefined : { cause });

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Characters (Unicode scalar values), as Rust's `chars().count()`. */
function charCount(text: string): number {
  return Array.from(text).length;
}

/** UTF-8 length in bytes: the unit of `MAX_TEXT_BYTES`. */
export function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** A–Z folded to a–z and nothing else, so offsets stay the same. */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

function eqIgnoreAsciiCase(a: string, b: string): boolean {
  return asciiLower(a) === asciiLower(b);
}

/** A trimmed single-line value of 1..=`max` characters. */
function cleanLine(
  value: string,
  what: string,
  max: number,
  fail: (message: string) => VocabularyError,
): string {
  const clean = value.trim();
  if (clean.length === 0) throw fail(`${what}不能为空`);
  const chars = charCount(clean);
  if (chars > max) throw fail(`${what}最多 ${max} 个字符（当前 ${chars}）`);
  if (/\p{Cc}/u.test(clean)) throw fail(`${what}不能包含换行或控制字符`);
  return clean;
}

/** A draft on its own, normalised (trimmed, empty variants dropped, duplicates collapsed ignoring
 *  ASCII case), or a `dictionary: …` refusal. */
export function validateDictionaryDraft(draft: DictionaryDraft): DictionaryDraft {
  const term = cleanLine(draft.term, "正确写法", MAX_TERM_CHARS, dictErr);
  const heardAs: string[] = [];
  for (const raw of draft.heard_as) {
    if (raw.trim().length === 0) continue;
    const variant = cleanLine(raw, "误识别写法", MAX_TERM_CHARS, dictErr);
    if (variant === term) throw dictErr(`误识别写法「${variant}」和正确写法相同`);
    if (!heardAs.some((h) => eqIgnoreAsciiCase(h, variant))) heardAs.push(variant);
  }
  if (heardAs.length > MAX_HEARD_AS) {
    throw dictErr(`一个词条最多 ${MAX_HEARD_AS} 个误识别写法（当前 ${heardAs.length}）`);
  }
  return { term, heard_as: heardAs, enabled: draft.enabled };
}

/** The whole list: at most `MAX_DICTIONARY_ENTRIES`, unique terms, a mis-hearing belongs to one
 *  entry and is nobody's term. Each entry is checked against the earlier ones, so a refusal names
 *  the entry that was there first. */
export function checkDictionary(entries: readonly DictionaryEntry[]): void {
  if (entries.length > MAX_DICTIONARY_ENTRIES) {
    throw dictErr(`词典最多 ${MAX_DICTIONARY_ENTRIES} 条`);
  }
  entries.forEach((a, i) => {
    for (const b of entries.slice(0, i)) {
      if (eqIgnoreAsciiCase(a.term, b.term)) throw dictErr(`词典里已有「${b.term}」`);
      if (b.heard_as.some((h) => eqIgnoreAsciiCase(h, a.term))) {
        throw dictErr(`「${a.term}」已是「${b.term}」的误识别写法，不能再作为正确写法`);
      }
      for (const variant of a.heard_as) {
        if (eqIgnoreAsciiCase(b.term, variant)) {
          throw dictErr(
            `「${variant}」是词条「${b.term}」的正确写法，不能再作为「${a.term}」的误识别写法`,
          );
        }
        if (b.heard_as.some((h) => eqIgnoreAsciiCase(h, variant))) {
          throw dictErr(`「${variant}」已是「${b.term}」的误识别写法`);
        }
      }
    }
  });
}

/** What the Rust `regex` crate refuses and JavaScript would accept: look-around, backreferences. */
const UNSUPPORTED_REGEX = /\(\?<?[=!]|\\[1-9]|\\k</;

/** A regex rule's pattern as a global, Unicode JavaScript expression; throws `正则无法编译：…`. */
export function compileRegex(pattern: string, caseSensitive: boolean): RegExp {
  if (UNSUPPORTED_REGEX.test(pattern)) {
    throw new Error("正则无法编译：不支持环视与反向引用");
  }
  try {
    return new RegExp(pattern, caseSensitive ? "gu" : "giu");
  } catch (e) {
    throw new Error(`正则无法编译：${messageOf(e)}`, { cause: e });
  }
}

/** A rule draft on its own (name trimmed, pattern and replacement as typed, a regex compiled), or
 *  a `rules: …` refusal. */
export function validateRuleDraft(draft: RuleDraft): RuleDraft {
  const name = cleanLine(draft.name, "规则名称", MAX_RULE_NAME_CHARS, rulesErr);
  if (draft.pattern.length === 0) throw rulesErr(`规则「${name}」的匹配内容不能为空`);
  const patternChars = charCount(draft.pattern);
  if (patternChars > MAX_PATTERN_CHARS) {
    throw rulesErr(
      `规则「${name}」的匹配内容最多 ${MAX_PATTERN_CHARS} 个字符（当前 ${patternChars}）`,
    );
  }
  const replacementChars = charCount(draft.replacement);
  if (replacementChars > MAX_REPLACEMENT_CHARS) {
    throw rulesErr(
      `规则「${name}」的替换内容最多 ${MAX_REPLACEMENT_CHARS} 个字符（当前 ${replacementChars}）`,
    );
  }
  if (draft.kind === "regex") {
    try {
      compileRegex(draft.pattern, draft.case_sensitive);
    } catch (e) {
      throw rulesErr(`规则「${name}」：${messageOf(e)}`, e);
    }
  }
  return { ...draft, name };
}

/** The whole rule list: at most `MAX_RULES`, names unique. */
export function checkRules(rules: readonly ReplacementRule[]): void {
  if (rules.length > MAX_RULES) throw rulesErr(`规则最多 ${MAX_RULES} 条`);
  rules.forEach((a, i) => {
    if (rules.slice(0, i).some((b) => b.name === a.name)) {
      throw rulesErr(`已有名为「${a.name}」的规则`);
    }
  });
}

// ---- matching ------------------------------------------------------------------------------------

/** Scripts written without spaces between words: their characters never form a boundary. */
const UNSPACED: readonly (readonly [number, number])[] = [
  [0x0e00, 0x0eff],
  [0x1000, 0x109f],
  [0x1100, 0x11ff],
  [0x1780, 0x17ff],
  [0x2e80, 0x2fdf],
  [0x3005, 0x3007],
  [0x3040, 0x30ff],
  [0x3100, 0x312f],
  [0x3130, 0x318f],
  [0x31a0, 0x31ff],
  [0x3400, 0x4dbf],
  [0x4e00, 0x9fff],
  [0xa960, 0xa97f],
  [0xac00, 0xd7ff],
  [0xf900, 0xfaff],
  [0xff66, 0xff9f],
  [0xffa0, 0xffdc],
  [0x20000, 0x3134f],
];

const WORDISH = /^[\p{Alphabetic}\p{N}_]$/u;

/** `_` or a letter / digit outside the unspaced scripts (the core's `is_word_char`). */
export function isWordChar(c: string): boolean {
  const cp = c.codePointAt(0) ?? 0;
  return WORDISH.test(c) && !UNSPACED.some(([lo, hi]) => cp >= lo && cp <= hi);
}

function charBefore(text: string, at: number): string | undefined {
  if (at === 0) return undefined;
  const low = text.charCodeAt(at - 1);
  return low >= 0xdc00 && low <= 0xdfff && at >= 2
    ? text.slice(at - 2, at)
    : text.slice(at - 1, at);
}

function charAfter(text: string, at: number): string | undefined {
  const cp = text.codePointAt(at);
  return cp === undefined ? undefined : String.fromCodePoint(cp);
}

interface LiteralPattern {
  folded: string;
  target: number;
  /** Needs a boundary before / after (it starts / ends with a word character). */
  before: boolean;
  after: boolean;
}

/** Many literal patterns, each mapped to a target (a dictionary entry, or the one rule). */
class LiteralMatcher {
  private readonly patterns: LiteralPattern[];

  constructor(
    patterns: readonly (readonly [string, number])[],
    private readonly asciiCaseInsensitive: boolean,
  ) {
    this.patterns = patterns
      .filter(([text]) => text.length > 0)
      .map(([text, target]) => {
        const chars = Array.from(text);
        return {
          folded: asciiCaseInsensitive ? asciiLower(text) : text,
          target,
          before: isWordChar(chars[0] ?? ""),
          after: isWordChar(chars.at(-1) ?? ""),
        };
      });
  }

  /** Accepted matches `[start, end, target]`, left to right, non-overlapping: among the matches
   *  that respect their boundaries the leftmost, and at one position the longest. */
  findAll(text: string): [number, number, number][] {
    const hay = this.asciiCaseInsensitive ? asciiLower(text) : text;
    const candidates: [number, number, number][] = [];
    this.patterns.forEach((p, index) => {
      for (let at = hay.indexOf(p.folded); at !== -1; at = hay.indexOf(p.folded, at + 1)) {
        const end = at + p.folded.length;
        const head = charBefore(text, at);
        const tail = charAfter(text, end);
        const blocked =
          (p.before && head !== undefined && isWordChar(head)) ||
          (p.after && tail !== undefined && isWordChar(tail));
        if (!blocked) candidates.push([at, end, index]);
      }
    });
    candidates.sort((a, b) => a[0] - b[0] || b[1] - a[1] || a[2] - b[2]);
    const accepted: [number, number, number][] = [];
    let lastEnd = 0;
    for (const [start, end, index] of candidates) {
      if (start < lastEnd) continue;
      accepted.push([start, end, this.patterns[index]?.target ?? 0]);
      lastEnd = end;
    }
    return accepted;
  }

  /** Every accepted match replaced; the targets of the matches that changed something. */
  replace(text: string, replacement: (target: number) => string): [string, number[]] {
    let out = "";
    let last = 0;
    const changed: number[] = [];
    for (const [start, end, target] of this.findAll(text)) {
      const swap = replacement(target);
      out += text.slice(last, start) + swap;
      if (text.slice(start, end) !== swap) changed.push(target);
      last = end;
    }
    return [out + text.slice(last), changed];
  }
}

/** Rust's `Captures::expand`: `$$`, `$1`, `$name`, `${name}`; an unknown group is empty, a `$`
 *  without a valid reference stays literal. */
export function expandReplacement(replacement: string, match: RegExpExecArray): string {
  let out = "";
  let i = 0;
  while (i < replacement.length) {
    const dollar = replacement.indexOf("$", i);
    if (dollar === -1) {
      out += replacement.slice(i);
      break;
    }
    out += replacement.slice(i, dollar);
    const rest = replacement.slice(dollar + 1);
    if (rest.startsWith("$")) {
      out += "$";
      i = dollar + 2;
      continue;
    }
    const ref = /^\{([^}]*)\}/.exec(rest) ?? /^([A-Za-z0-9_]+)/.exec(rest);
    const name = ref?.[1];
    if (ref === null || name === undefined) {
      out += "$";
      i = dollar + 1;
      continue;
    }
    const group = /^\d+$/.test(name) ? match[Number(name)] : match.groups?.[name];
    out += group ?? "";
    i = dollar + 1 + ref[0].length;
  }
  return out;
}

/** What one vocabulary step did: the text (its input when it fell back), what fired, and why it
 *  fell back. */
export interface VocabularyStep {
  text: string;
  hits: VocabularyHit[];
  error?: string;
}

interface CompiledRule {
  id: string;
  name: string;
  kind: RuleKind;
  literal?: LiteralMatcher;
  regex?: RegExp;
  replacement: string;
}

const tooLong = (text: string) => utf8Length(text) > MAX_TEXT_BYTES;
const KIB = MAX_TEXT_BYTES / 1024;

/** The compiled dictionary and rules one take runs. */
export class Vocabulary {
  private constructor(
    private readonly corrections: LiteralMatcher | undefined,
    private readonly terms: readonly (readonly [string, string])[],
    private readonly rules: readonly CompiledRule[],
  ) {}

  /** The enabled entries and rules; one that does not compile is skipped, never an error. */
  static compile(
    dictionary: readonly DictionaryEntry[],
    rules: readonly ReplacementRule[],
  ): Vocabulary {
    const terms: [string, string][] = [];
    const patterns: [string, number][] = [];
    for (const entry of dictionary.filter((e) => e.enabled)) {
      const target = terms.length;
      terms.push([entry.id, entry.term]);
      for (const variant of entry.heard_as) patterns.push([variant, target]);
    }
    const compiled: CompiledRule[] = [];
    for (const rule of rules.filter((r) => r.enabled)) {
      const base = { id: rule.id, name: rule.name, kind: rule.kind, replacement: rule.replacement };
      if (rule.kind === "literal") {
        compiled.push({
          ...base,
          literal: new LiteralMatcher([[rule.pattern, 0]], !rule.case_sensitive),
        });
        continue;
      }
      try {
        compiled.push({ ...base, regex: compileRegex(rule.pattern, rule.case_sensitive) });
      } catch {
        // A stored rule always compiled when it was saved; skip it rather than fail the take.
      }
    }
    const corrections = patterns.length > 0 ? new LiteralMatcher(patterns, true) : undefined;
    return new Vocabulary(corrections, terms, compiled);
  }

  /** Dictionary corrections: every mis-hearing in one left-to-right pass, longest first. */
  correct(text: string): VocabularyStep {
    const matcher = this.corrections;
    if (matcher === undefined) return { text, hits: [] };
    if (tooLong(text)) return { text, hits: [], error: `文本超过 ${KIB} KiB，跳过词典纠正` };
    const [out, changed] = matcher.replace(text, (target) => this.terms[target]?.[1] ?? "");
    if (tooLong(out)) {
      return { text, hits: [], error: `词典纠正让文本超过 ${KIB} KiB，本次不纠正` };
    }
    const counts = new Map<number, number>();
    for (const target of changed) counts.set(target, (counts.get(target) ?? 0) + 1);
    const byTarget = [...counts.entries()];
    byTarget.sort((a, b) => a[0] - b[0]);
    const hits = byTarget.map(([target, count]) => ({
      id: this.terms[target]?.[0] ?? NIL_ID,
      count,
    }));
    return { text: out, hits };
  }

  /** The rules in order, each on the previous one's output. */
  applyRules(text: string): VocabularyStep {
    if (this.rules.length === 0) return { text, hits: [] };
    if (tooLong(text)) return { text, hits: [], error: `文本超过 ${KIB} KiB，跳过替换规则` };
    let current = text;
    const hits: VocabularyHit[] = [];
    for (const rule of this.rules) {
      const [next, count] = applyRule(rule, current);
      if (tooLong(next)) {
        return {
          text,
          hits: [],
          error: `规则「${rule.name}」让文本超过 ${KIB} KiB，本次不应用替换规则`,
        };
      }
      if (count > 0) {
        hits.push({ id: rule.id, count });
        current = next;
      }
    }
    return { text: current, hits };
  }
}

function applyRule(rule: CompiledRule, text: string): [string, number] {
  if (rule.literal !== undefined) {
    const [out, changed] = rule.literal.replace(text, () => rule.replacement);
    return [out, changed.length];
  }
  const re = rule.regex;
  if (re === undefined) return [text, 0];
  let out = "";
  let last = 0;
  let count = 0;
  for (const match of text.matchAll(re)) {
    const matched = match[0];
    // An empty match would put the replacement between every two characters.
    if (matched.length === 0) continue;
    const expanded = expandReplacement(rule.replacement, match);
    out += text.slice(last, match.index) + expanded;
    last = match.index + matched.length;
    if (expanded !== matched) count += 1;
  }
  return [out + text.slice(last), count];
}

/** `vocabulary_preview`: `text` through the dictionary and the rules, with `draft` standing in for
 *  the rule it names (or appended under the nil id). */
export function previewVocabulary(
  dictionary: readonly DictionaryEntry[],
  rules: readonly ReplacementRule[],
  text: string,
  draft?: PreviewDraft,
): VocabularyPreview {
  if (tooLong(text)) throw rulesErr(`试写文本最多 ${KIB} KiB`);
  let list = [...rules];
  if (draft !== undefined) {
    const rule = validateRuleDraft(draft.rule);
    const asRule = (id: string): ReplacementRule => ({
      ...rule,
      id,
      created_at_ms: 0,
      updated_at_ms: 0,
    });
    const at = list.findIndex((r) => r.id === draft.id);
    if (at >= 0) list = list.map((r, i) => (i === at ? asRule(r.id) : r));
    else list.push(asRule(draft.id ?? NIL_ID));
  }
  const vocabulary = Vocabulary.compile(dictionary, list);
  const corrected = vocabulary.correct(text);
  const ruled = vocabulary.applyRules(corrected.text);
  const error = corrected.error ?? ruled.error;
  return {
    corrected: corrected.text,
    output: ruled.text,
    corrections: corrected.hits,
    rules: ruled.hits,
    ...(error === undefined ? {} : { error }),
  };
}

// ---- TOML exchange format (docs/dictation.md §16.5) ----------------------------------------------

/** `version` of the format. */
export const RULES_TOML_VERSION = 1;
const TOML_HEADER = "# Voltip 替换规则 · docs/dictation.md §16\n";
const RULE_KEYS: readonly string[] = [
  "name",
  "kind",
  "pattern",
  "replacement",
  "case_sensitive",
  "enabled",
];
const TOML_ESCAPES: Readonly<Record<string, string>> = {
  '"': '\\"',
  "\\": "\\\\",
  "\n": "\\n",
  "\t": "\\t",
  "\r": "\\r",
};
const TOML_UNESCAPES: Readonly<Record<string, string>> = {
  b: "\b",
  t: "\t",
  n: "\n",
  f: "\f",
  r: "\r",
  '"': '"',
  "\\": "\\",
};

function isControl(c: string): boolean {
  const code = c.charCodeAt(0);
  return code < 0x20 || code === 0x7f;
}

/** A basic string when that needs no escape, a literal string when the value has quotes or
 *  backslashes but no apostrophe, the escaped basic form otherwise. */
function tomlString(value: string): string {
  const chars = Array.from(value);
  const control = chars.some((c) => c !== "\t" && isControl(c));
  if (!control && !/["\\]/.test(value)) return `"${value}"`;
  if (!control && !value.includes("'")) return `'${value}'`;
  const escaped = chars.map(
    (c) =>
      TOML_ESCAPES[c] ?? (isControl(c) ? `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}` : c),
  );
  return `"${escaped.join("")}"`;
}

/** The rules as TOML text, in execution order, without ids or timestamps (what `rules_export`
 *  returns). */
export function exportRulesToml(rules: readonly ReplacementRule[]): string {
  const tables = rules.map((r) =>
    [
      "",
      "[[rule]]",
      `name = ${tomlString(r.name)}`,
      `kind = "${r.kind}"`,
      `pattern = ${tomlString(r.pattern)}`,
      `replacement = ${tomlString(r.replacement)}`,
      `case_sensitive = ${String(r.case_sensitive)}`,
      `enabled = ${String(r.enabled)}`,
      "",
    ].join("\n"),
  );
  return `${TOML_HEADER}version = ${RULES_TOML_VERSION}\n${tables.join("")}`;
}

class TomlSyntaxError extends Error {
  constructor(line: number, detail: string) {
    super(`TOML parse error at line ${line}: ${detail}`);
  }
}

type TomlValue = string | number | boolean;

interface TomlTable {
  line: number;
  values: Map<string, { value: TomlValue; line: number }>;
}

/** The subset of TOML the format uses: comments, `key = value` with strings (all four forms),
 *  booleans and integers, and `[[rule]]` headers. */
function parseTomlSubset(text: string): { top: TomlTable; rules: TomlTable[] } {
  const top: TomlTable = { line: 1, values: new Map() };
  const rules: TomlTable[] = [];
  let table = top;
  let i = 0;
  let line = 1;
  const err = (detail: string) => new TomlSyntaxError(line, detail);
  const skipSpaces = () => {
    while (text[i] === " " || text[i] === "\t") i += 1;
  };
  const endOfLine = () => {
    skipSpaces();
    if (text[i] === "#") while (i < text.length && text[i] !== "\n") i += 1;
    if (text[i] === "\r" && text[i + 1] === "\n") i += 1;
    if (i < text.length && text[i] !== "\n")
      throw err(`unexpected \`${text[i] ?? ""}\` after a value`);
    i += 1;
    line += 1;
  };
  const readString = (): string => {
    const quote = text[i] === '"' ? '"' : "'";
    const triple = quote.repeat(3);
    const delimiter = text.startsWith(triple, i) ? triple : quote;
    const multi = delimiter === triple;
    i += delimiter.length;
    // A newline right after the opening delimiter of a multi-line string is dropped.
    const lead = /^\r?\n/.exec(text.slice(i, i + 2))?.[0];
    if (multi && lead !== undefined) {
      i += lead.length;
      line += 1;
    }
    let out = "";
    for (;;) {
      if (i >= text.length) throw err("unterminated string");
      if (text.startsWith(delimiter, i)) {
        i += delimiter.length;
        return out;
      }
      const c = text[i] ?? "";
      if (c === "\n") {
        if (!multi) throw err("newline in a single-line string");
        line += 1;
      }
      if (c === "\\" && quote === '"') {
        const e = text[i + 1] ?? "";
        const simple = TOML_UNESCAPES[e];
        const width = e === "u" ? 4 : e === "U" ? 8 : 0;
        const hex = text.slice(i + 2, i + 2 + width);
        if (simple !== undefined) {
          out += simple;
          i += 2;
        } else if (width > 0 && hex.length === width && /^[0-9A-Fa-f]+$/.test(hex)) {
          out += String.fromCodePoint(Number.parseInt(hex, 16));
          i += 2 + width;
        } else {
          throw err(`invalid escape \`\\${e}\``);
        }
        continue;
      }
      out += c;
      i += 1;
    }
  };
  const readValue = (): TomlValue => {
    const c = text[i];
    if (c === '"' || c === "'") return readString();
    const bare = /^[A-Za-z0-9_+-]+/.exec(text.slice(i))?.[0] ?? "";
    i += bare.length;
    if (bare === "true" || bare === "false") return bare === "true";
    if (/^[+-]?\d+$/.test(bare)) return Number.parseInt(bare, 10);
    throw err(bare.length === 0 ? "a value is expected" : `invalid value \`${bare}\``);
  };
  while (i < text.length) {
    skipSpaces();
    const c = text[i];
    if (c === "\n" || c === "\r" || c === "#" || c === undefined) {
      endOfLine();
      continue;
    }
    if (c === "[") {
      const header = /^\[\[\s*rule\s*\]\]/.exec(text.slice(i))?.[0];
      if (header === undefined) throw err("only [[rule]] tables are supported");
      i += header.length;
      table = { line, values: new Map() };
      rules.push(table);
      endOfLine();
      continue;
    }
    let key: string;
    if (c === '"' || c === "'") key = readString();
    else {
      key = /^[A-Za-z0-9_-]+/.exec(text.slice(i))?.[0] ?? "";
      i += key.length;
    }
    if (key.length === 0) throw err("a key is expected");
    skipSpaces();
    if (text[i] !== "=") throw err(`\`=\` is expected after \`${key}\``);
    i += 1;
    skipSpaces();
    if (table.values.has(key)) throw err(`duplicate key \`${key}\``);
    const keyLine = line;
    table.values.set(key, { value: readValue(), line: keyLine });
    endOfLine();
  }
  return { top, rules };
}

function stringField(table: TomlTable, key: string, fallback?: string): string {
  const entry = table.values.get(key);
  if (entry === undefined) {
    if (fallback === undefined) throw new TomlSyntaxError(table.line, `missing field \`${key}\``);
    return fallback;
  }
  if (typeof entry.value !== "string") {
    throw new TomlSyntaxError(entry.line, `invalid type for \`${key}\`, expected a string`);
  }
  return entry.value;
}

function boolField(table: TomlTable, key: string): boolean {
  const entry = table.values.get(key);
  if (entry === undefined) return true;
  if (typeof entry.value !== "boolean") {
    throw new TomlSyntaxError(entry.line, `invalid type for \`${key}\`, expected a boolean`);
  }
  return entry.value;
}

function draftFromTable(table: TomlTable): RuleDraft {
  for (const [key, { line }] of table.values) {
    if (!RULE_KEYS.includes(key)) {
      const expected = RULE_KEYS.map((k) => `\`${k}\``).join(", ");
      throw new TomlSyntaxError(line, `unknown field \`${key}\`, expected one of ${expected}`);
    }
  }
  const kind = stringField(table, "kind", "literal");
  if (kind !== "literal" && kind !== "regex") {
    const line = table.values.get("kind")?.line ?? table.line;
    throw new TomlSyntaxError(
      line,
      `unknown variant \`${kind}\`, expected \`literal\` or \`regex\``,
    );
  }
  return {
    name: stringField(table, "name"),
    kind,
    pattern: stringField(table, "pattern"),
    replacement: stringField(table, "replacement", ""),
    case_sensitive: boolField(table, "case_sensitive"),
    enabled: boolField(table, "enabled"),
  };
}

/** Parse and validate a whole import: the text, the version, the count, every rule (a regex is
 *  compiled) and the uniqueness of names inside the file. Nothing is partially accepted. */
export function parseRulesToml(text: string): RuleDraft[] {
  if (utf8Length(text) > MAX_TOML_BYTES) {
    throw rulesErr(`TOML 文本最多 ${MAX_TOML_BYTES / 1024} KiB`);
  }
  let drafts: RuleDraft[];
  let version: number;
  try {
    const { top, rules } = parseTomlSubset(text);
    for (const [key, { line }] of top.values) {
      if (key !== "version") {
        throw new TomlSyntaxError(
          line,
          `unknown field \`${key}\`, expected \`version\` or \`rule\``,
        );
      }
    }
    const raw = top.values.get("version");
    if (raw === undefined) throw new TomlSyntaxError(1, "missing field `version`");
    if (typeof raw.value !== "number") {
      throw new TomlSyntaxError(raw.line, "invalid type for `version`, expected an integer");
    }
    version = raw.value;
    drafts = rules.map(draftFromTable);
  } catch (e) {
    throw rulesErr(`TOML 无法解析：${messageOf(e)}`, e);
  }
  if (version !== RULES_TOML_VERSION) {
    throw rulesErr(`不支持的 version = ${version}（应为 ${RULES_TOML_VERSION}）`);
  }
  if (drafts.length > MAX_RULES) {
    throw rulesErr(`文件里有 ${drafts.length} 条规则，最多 ${MAX_RULES} 条`);
  }
  const valid: RuleDraft[] = [];
  drafts.forEach((rule, index) => {
    const n = index + 1;
    let draft: RuleDraft;
    try {
      draft = validateRuleDraft(rule);
    } catch (e) {
      const detail = messageOf(e).replace(/^rules: /, "");
      throw rulesErr(`第 ${n} 条（${rule.name.trim()}）：${detail}`, e);
    }
    if (valid.some((d) => d.name === draft.name)) {
      throw rulesErr(`第 ${n} 条：名称「${draft.name}」在文件里重复`);
    }
    valid.push(draft);
  });
  return valid;
}
