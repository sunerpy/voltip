// The history quick fix (docs/dictation.md §16.9) as pure functions the desktop history page and the
// phone's entry screen share: the wordings the AI polish replaced (a deletion right next to an
// addition in `diffSegments`), the marks on a take's original text, a selection trimmed and grown
// to a whole change, the card's pre-filled fields and default kind, the short-form risk, the checks
// against the dictionary the card makes before the core checks again on save, the rule's generated
// name, the status rows the card shows, and the words the phone splits the text into. Offsets are
// UTF-16 code units, as the DOM and `String.prototype.slice` count them.
import { type TFunction, zhT } from "./i18n";
import { type DiffSegment, diffSegments, diffTokens } from "./refine";
import {
  type DictionaryDraft,
  type DictionaryEntry,
  type HistoryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_RULE_NAME_CHARS,
  MAX_TERM_CHARS,
  type ReplacementRule,
  type RuleDraft,
  type RuleKind,
  type VocabularyBacktest,
} from "./schema";
import { dictionaryForms } from "./vocabulary";
import {
  type DictionaryDraftProblem,
  type RuleDraftProblem,
  dictionaryDraftProblem,
  ruleDraftProblem,
  splitHeardAs,
} from "./vocabulary-drafts";

/** White space and punctuation: what a selection and a change lose at both ends. */
const EDGE = /^[\s\p{P}]$/u;

/** Characters (Unicode scalar values), as the core counts them. */
function chars(text: string): number {
  return Array.from(text).length;
}

/** A–Z folded to a–z: the core compares dictionary forms ignoring ASCII case only. */
function asciiFold(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

function sameIgnoringAsciiCase(a: string, b: string): boolean {
  return asciiFold(a) === asciiFold(b);
}

/** The first `max` characters of `text`, without white space at the end. */
function firstChars(text: string, max: number): string {
  return Array.from(text).slice(0, max).join("").trim();
}

/** A half-open range of a text, in UTF-16 code units. */
export interface TextRange {
  start: number;
  end: number;
}

/** `[start, end)` of `text` without white space and punctuation at either end (by code point);
 *  `undefined` when nothing is left. */
export function trimRange(text: string, start: number, end: number): TextRange | undefined {
  let s = Math.max(0, Math.min(start, end));
  let e = Math.min(text.length, Math.max(start, end));
  while (s < e) {
    const c = String.fromCodePoint(text.codePointAt(s) ?? 0);
    if (!EDGE.test(c)) break;
    s += c.length;
  }
  while (e > s) {
    const low = text.charCodeAt(e - 1);
    const width = low >= 0xdc00 && low <= 0xdfff && e - 2 >= s ? 2 : 1;
    if (!EDGE.test(text.slice(e - width, e))) break;
    e -= width;
  }
  return s < e ? { start: s, end: e } : undefined;
}

// ---- what the polish replaced ------------------------------------------------------------------

/** One run of `diffSegments` with where it starts in the original (`raw_text`) and the final text
 *  (`text`): an addition starts where it was inserted into the original, a deletion where it was
 *  taken out of the final text. */
export interface DiffRun {
  kind: DiffSegment["kind"];
  text: string;
  rawStart: number;
  textStart: number;
}

/** The token diff of a take's original and final text with each run's offsets in both. */
export function diffRuns(raw: string, text: string): DiffRun[] {
  const out: DiffRun[] = [];
  let r = 0;
  let t = 0;
  for (const seg of diffSegments({ rawText: raw, text })) {
    out.push({ kind: seg.kind, text: seg.text, rawStart: r, textStart: t });
    if (seg.kind !== "add") r += seg.text.length;
    if (seg.kind !== "del") t += seg.text.length;
  }
  return out;
}

/** A wording the AI polish (or the dictionary and rules) replaced: the original wording and what
 *  the final text says in its place, each without white space and punctuation at the ends. */
export interface PolishPair {
  /** Where the original wording is in `raw_text`. */
  start: number;
  end: number;
  heard: string;
  /** Where its replacement is in `text`. */
  polishedStart: number;
  polishedEnd: number;
  polished: string;
}

/** The largest diff (tokens of the original × tokens of the final text) the replacements are
 *  looked for in: the token LCS keeps a table of that size, so a long recording's text (§22) gets
 *  no marks rather than a stalled page. */
export const MAX_PAIR_DIFF_CELLS = 1_000_000;

/** A deletion followed by an addition as one replacement (`diffSegments` breaks ties towards the
 *  deletion, so an addition never comes first); pure white space or punctuation changes and lone
 *  deletions or additions are not replacements. None past `MAX_PAIR_DIFF_CELLS`. */
export function polishPairs(raw: string, text: string): PolishPair[] {
  if (diffTokens(raw).length * diffTokens(text).length > MAX_PAIR_DIFF_CELLS) return [];
  const runs = diffRuns(raw, text);
  const pairs: PolishPair[] = [];
  for (let i = 0; i + 1 < runs.length; i += 1) {
    const del = runs[i];
    const add = runs[i + 1];
    if (del?.kind !== "del" || add?.kind !== "add") continue;
    // Both runs belong to this replacement, whatever trimming leaves of them.
    i += 1;
    const heard = trimRange(raw, del.rawStart, del.rawStart + del.text.length);
    const polished = trimRange(text, add.textStart, add.textStart + add.text.length);
    if (heard === undefined || polished === undefined) continue;
    const h = raw.slice(heard.start, heard.end);
    const p = text.slice(polished.start, polished.end);
    if (h === p) continue;
    pairs.push({
      start: heard.start,
      end: heard.end,
      heard: h,
      polishedStart: polished.start,
      polishedEnd: polished.end,
      polished: p,
    });
  }
  return pairs;
}

/** One thing the diff view shows: an unchanged, deleted or added run, or a replacement that can be
 *  clicked (its deletion and its addition). */
export type DiffItem =
  | { kind: "same"; text: string }
  | { kind: "del"; text: string }
  | { kind: "add"; text: string }
  | { kind: "pair"; del: string; add: string; pair: PolishPair };

/** The diff view's runs with each replacement grouped (docs/dictation.md §16.9 3.5). */
export function diffItems(raw: string, text: string): DiffItem[] {
  const runs = diffRuns(raw, text);
  const pairs = polishPairs(raw, text);
  const items: DiffItem[] = [];
  for (let i = 0; i < runs.length; i += 1) {
    const run = runs[i];
    if (run === undefined) continue;
    const next = runs[i + 1];
    const pair =
      run.kind === "del" && next?.kind === "add"
        ? pairs.find((p) => p.start >= run.rawStart && p.end <= run.rawStart + run.text.length)
        : undefined;
    if (pair !== undefined && next !== undefined) {
      items.push({ kind: "pair", del: run.text, add: next.text, pair });
      i += 1;
      continue;
    }
    items.push({ kind: run.kind, text: run.text });
  }
  return items;
}

// ---- marks on the original text ----------------------------------------------------------------

/** `polish`: a wording the AI polish replaced; `known`: a heard-as form of an enabled entry. */
export type MarkKind = "polish" | "known";

export interface TextMark extends TextRange {
  kind: MarkKind;
  /** The marked wording. */
  text: string;
  /** `polish`: the replacement. */
  pair?: PolishPair;
  /** `known`: the entry the form belongs to. */
  entry?: DictionaryEntry;
}

/** The marks of a take's original text, in order: every heard-as form of an enabled entry (ASCII
 *  case ignored, as the corrections find them), and every replacement the polish made where no
 *  such form is (a form the dictionary corrects shows as a polish replacement in the diff too). */
export function textMarks(
  raw: string,
  text: string,
  dictionary: readonly DictionaryEntry[],
): TextMark[] {
  const known: TextMark[] = [];
  for (const form of dictionaryForms(dictionary, raw)) {
    const entry = dictionary.find((e) => e.id === form.entryId);
    if (entry !== undefined) {
      known.push({
        kind: "known",
        start: form.start,
        end: form.end,
        text: raw.slice(form.start, form.end),
        entry,
      });
    }
  }
  const polish: TextMark[] = polishPairs(raw, text)
    .filter((p) => !known.some((k) => p.start < k.end && k.start < p.end))
    .map((p) => ({ kind: "polish", start: p.start, end: p.end, text: p.heard, pair: p }));
  const marks = [...known, ...polish];
  marks.sort((x, y) => x.start - y.start);
  return marks;
}

// ---- the selection and what the card starts with ---------------------------------------------

/** The text view a selection was made in. */
export type FixView = "raw" | "polished";
/** What the card writes: a dictionary entry or a replacement rule. */
export type FixKind = "dictionary" | "rule";
/** Why a selection cannot be a dictionary form: over `MAX_TERM_CHARS`, or across lines. */
export type DictionaryBlock = "tooLong" | "oneLine";

/** A selection made fit for the card (docs/dictation.md §16.9 3.2, 5.1). */
export interface FixTarget extends TextRange {
  view: FixView;
  /** The selected words: the heard-as form, or the rule's pattern. */
  text: string;
  /** The replacement the selection lies in: its original wording (`raw`) or its replacement
   *  (`polished`), to which the selection grew. */
  pair?: PolishPair;
  /** The words are only in the polished text: the polish wrote them, so a rule fits. */
  polishOnly: boolean;
  /** Why 加入词典 is unavailable, when it is. */
  block?: DictionaryBlock;
  /** The card the selection opens by default. */
  kind: FixKind;
}

/** A selection of the raw or the polished view made fit for the card: white space and punctuation
 *  trimmed off both ends, grown to a whole replacement it lies in, and its default kind — a rule
 *  when the words are too long or span lines for a dictionary form, or are only in the polished
 *  text; a dictionary entry otherwise. `undefined` when nothing but white space and punctuation was
 *  selected. */
export function fixTarget(
  entry: Pick<HistoryEntry, "raw_text" | "text">,
  view: FixView,
  start: number,
  end: number,
): FixTarget | undefined {
  const source = view === "raw" ? entry.raw_text : entry.text;
  const trimmed = trimRange(source, start, end);
  if (trimmed === undefined) return undefined;
  let s = trimmed.start;
  let e = trimmed.end;
  const pairs = polishPairs(entry.raw_text, entry.text);
  let pair: PolishPair | undefined;
  let polishOnly = false;
  if (view === "raw") {
    pair = pairs.find((p) => p.start <= s && e <= p.end);
    if (pair !== undefined) {
      s = pair.start;
      e = pair.end;
    }
  } else {
    pair = pairs.find((p) => p.polishedStart <= s && e <= p.polishedEnd);
    if (pair !== undefined) {
      s = pair.polishedStart;
      e = pair.polishedEnd;
      polishOnly = true;
    } else {
      // Words over an addition (white space and punctuation aside) are not in the original.
      polishOnly = diffRuns(entry.raw_text, entry.text).some(
        (r) =>
          r.kind === "add" &&
          trimRange(r.text, 0, r.text.length) !== undefined &&
          s < r.textStart + r.text.length &&
          r.textStart < e,
      );
    }
  }
  const text = source.slice(s, e);
  const block: DictionaryBlock | undefined = /[\r\n]/.test(text)
    ? "oneLine"
    : chars(text) > MAX_TERM_CHARS
      ? "tooLong"
      : undefined;
  return {
    view,
    start: s,
    end: e,
    text,
    ...(pair === undefined ? {} : { pair }),
    polishOnly,
    ...(block === undefined ? {} : { block }),
    kind: block !== undefined || polishOnly ? "rule" : "dictionary",
  };
}

/** Where the card's right spelling came from: the polished text (来自润色结果) or the original (来自原文). */
export type TermSource = "polish" | "raw";

/** The card's fields as it opens: 曾听成 / 查找 (`heard`) and 正确写法 / 替换为 (`term`). */
export interface FixPrefill {
  kind: FixKind;
  heard: string;
  term: string;
  termFrom?: TermSource;
}

/** What a selection pre-fills (docs/dictation.md §16.9 5.2): the selected words, and, for an
 *  original wording the polish replaced, the replacement (trimmed); otherwise the right spelling is
 *  left empty for the person to type. */
export function prefillFor(target: FixTarget): FixPrefill {
  if (target.view === "raw" && target.pair !== undefined) {
    return {
      kind: target.kind,
      heard: target.text,
      term: target.pair.polished,
      termFrom: "polish",
    };
  }
  return { kind: target.kind, heard: target.text, term: "" };
}

/** Which spelling of a replacement is right (the diff view's question, §16.9 3.5): the polished
 *  one — recognition was wrong, the original wording is heard-as — or the original one — the polish
 *  changed it wrongly, so it becomes the right spelling and the heard-as forms are left to fill. */
export type RightSpelling = "polished" | "raw";

export function prefillPair(pair: PolishPair, right: RightSpelling): FixPrefill {
  return right === "polished"
    ? { kind: "dictionary", heard: pair.heard, term: pair.polished, termFrom: "polish" }
    : { kind: "dictionary", heard: "", term: pair.heard, termFrom: "raw" };
}

// ---- risk and the checks before saving ---------------------------------------------------------

/** A heard-as form or pattern short enough to change other words by mistake (§16.9 5.3): one
 *  character, or up to two ASCII letters. */
export interface ShortForm {
  form: string;
  chars: number;
  /** ASCII letters (the message counts letters, not characters). */
  latin: boolean;
}

export function shortForm(forms: readonly string[]): ShortForm | undefined {
  for (const raw of forms) {
    const form = raw.trim();
    const n = chars(form);
    const latin = /^[A-Za-z]+$/.test(form);
    if (n === 1 || (latin && n <= 2)) return { form, chars: n, latin };
  }
  return undefined;
}

/** A clash with the dictionary that the core would refuse (§16.1, §16.9 5.3), so the card does not
 *  save: a form already another entry's, a form that is another entry's right spelling, a form the
 *  same as the right spelling, a right spelling that is another entry's form, an entry that would
 *  have more than `MAX_HEARD_AS` forms, a full dictionary, or a merge that adds nothing. */
export type DictionaryConflict =
  | { kind: "taken"; heard: string; owner: DictionaryEntry }
  | { kind: "isTerm"; heard: string; owner: DictionaryEntry }
  | { kind: "same"; heard: string }
  | { kind: "termTaken"; term: string; owner: DictionaryEntry }
  | { kind: "tooMany"; entry: DictionaryEntry; max: number }
  | { kind: "full"; max: number }
  | { kind: "exists"; entry: DictionaryEntry };

/** What saving the dictionary card would do. */
export interface DictionaryPlan {
  /** The entry whose right spelling this already is (ASCII case ignored): the forms go into it
   *  (`dictionary_update`) instead of a new entry (`dictionary_add`). */
  merge?: DictionaryEntry;
  /** The forms typed into 曾听成, split at the separators, empty ones dropped. */
  heardAs: string[];
  /** Of those, the ones the entry does not have yet: all of them for a new entry. */
  added: string[];
  /** What is sent: a new entry, or for a merge the entry's spelling and flag with its forms and the
   *  new ones. */
  draft: DictionaryDraft;
  /** Problems of the fields themselves, shown under them (localised). */
  problem?: DictionaryDraftProblem;
  /** A clash with another entry: the card says why and does not save. */
  conflict?: DictionaryConflict;
  /** A right spelling, no problem, no clash. */
  ready: boolean;
}

/** The dictionary card's draft checked against the current `dictionary` before anything is sent:
 *  the field checks of the dictionary page (`dictionaryDraftProblem`) and the clashes the core
 *  refuses. The core checks again on save and its word is final. */
export function planDictionary(
  term: string,
  heard: string,
  dictionary: readonly DictionaryEntry[],
  t: TFunction = zhT.t,
): DictionaryPlan {
  const clean = term.trim();
  const heardAs: string[] = [];
  for (const form of splitHeardAs(heard)) {
    if (!heardAs.some((h) => sameIgnoringAsciiCase(h, form))) heardAs.push(form);
  }
  const merge =
    clean.length === 0 ? undefined : dictionary.find((e) => sameIgnoringAsciiCase(e.term, clean));
  const others = dictionary.filter((e) => e !== merge);
  const added =
    merge === undefined
      ? heardAs
      : heardAs.filter((h) => !merge.heard_as.some((x) => sameIgnoringAsciiCase(x, h)));
  const draft: DictionaryDraft =
    merge === undefined
      ? { term: clean, heard_as: heardAs, enabled: true }
      : { term: merge.term, heard_as: [...merge.heard_as, ...added], enabled: merge.enabled };
  const problem = dictionaryDraftProblem(clean, heardAs, others, t);
  // Before a right spelling is typed, a heard-as form the dictionary has already is said at once
  // (§16.9 5.2): as a right spelling there is nothing to fix, as another entry's form a clash.
  const conflict =
    problem?.heard !== undefined
      ? undefined
      : clean.length === 0
        ? heardClash(heardAs, dictionary)
        : problem === undefined
          ? clashWith(draft.term, heardAs, added, merge, others, dictionary)
          : undefined;
  return {
    ...(merge === undefined ? {} : { merge }),
    heardAs,
    added,
    draft,
    ...(problem === undefined ? {} : { problem }),
    ...(conflict === undefined ? {} : { conflict }),
    ready: clean.length > 0 && problem === undefined && conflict === undefined,
  };
}

/** The forms alone against the dictionary: one that is an entry's right spelling, or an entry's
 *  heard-as form. */
function heardClash(
  heardAs: readonly string[],
  dictionary: readonly DictionaryEntry[],
): DictionaryConflict | undefined {
  for (const heard of heardAs) {
    const entry = dictionary.find((e) => sameIgnoringAsciiCase(e.term, heard));
    if (entry !== undefined) return { kind: "exists", entry };
    const owner = dictionary.find((e) => e.heard_as.some((h) => sameIgnoringAsciiCase(h, heard)));
    if (owner !== undefined) return { kind: "taken", heard, owner };
  }
  return undefined;
}

function clashWith(
  term: string,
  heardAs: readonly string[],
  added: readonly string[],
  merge: DictionaryEntry | undefined,
  others: readonly DictionaryEntry[],
  dictionary: readonly DictionaryEntry[],
): DictionaryConflict | undefined {
  if (merge === undefined) {
    const owner = others.find((e) => e.heard_as.some((h) => sameIgnoringAsciiCase(h, term)));
    if (owner !== undefined) return { kind: "termTaken", term, owner };
  }
  for (const heard of heardAs) {
    // The core compares a form with its own entry's spelling exactly.
    if (heard === term) return { kind: "same", heard };
    const termOwner = others.find((e) => sameIgnoringAsciiCase(e.term, heard));
    if (termOwner !== undefined) return { kind: "isTerm", heard, owner: termOwner };
    const formOwner = others.find((e) => e.heard_as.some((h) => sameIgnoringAsciiCase(h, heard)));
    if (formOwner !== undefined) return { kind: "taken", heard, owner: formOwner };
  }
  if (merge !== undefined) {
    if (heardAs.length === 0) return { kind: "exists", entry: merge };
    const first = heardAs[0] ?? "";
    if (added.length === 0) return { kind: "taken", heard: first, owner: merge };
    if (merge.heard_as.length + added.length > MAX_HEARD_AS) {
      return { kind: "tooMany", entry: merge, max: MAX_HEARD_AS };
    }
    return undefined;
  }
  if (dictionary.length >= MAX_DICTIONARY_ENTRIES)
    return { kind: "full", max: MAX_DICTIONARY_ENTRIES };
  return undefined;
}

/** `{find} → {replace}`, at most `MAX_RULE_NAME_CHARS` characters, and not the name of a rule in
 *  `rules` (a ` (2)`, ` (3)` … is added): the name the rule card starts with. */
export function ruleNameFor(
  find: string,
  replace: string,
  rules: readonly Pick<ReplacementRule, "name">[],
): string {
  const base = firstChars(`${find.trim()} → ${replace.trim()}`.trim(), MAX_RULE_NAME_CHARS);
  const taken = (name: string) => rules.some((r) => r.name === name);
  if (!taken(base)) return base;
  for (let n = 2; ; n += 1) {
    const suffix = ` (${n})`;
    const name = `${firstChars(base, MAX_RULE_NAME_CHARS - chars(suffix))}${suffix}`;
    if (!taken(name)) return name;
  }
}

/** What saving the rule card would do: the draft, and the rules page's own checks of it (a regex
 *  is the core's to judge, through `vocabulary_preview`). */
export interface RulePlan {
  draft: RuleDraft;
  problem?: RuleDraftProblem;
  ready: boolean;
}

export function planRule(
  fields: { find: string; replace: string; name: string; kind: RuleKind; caseSensitive: boolean },
  rules: readonly ReplacementRule[],
  t: TFunction = zhT.t,
): RulePlan {
  const draft: RuleDraft = {
    name: fields.name.trim(),
    kind: fields.kind,
    pattern: fields.find,
    replacement: fields.replace,
    case_sensitive: fields.caseSensitive,
    enabled: true,
  };
  const problem = ruleDraftProblem(fields.name, fields.find, rules, t);
  return { draft, ...(problem === undefined ? {} : { problem }), ready: problem === undefined };
}

// ---- the card's status rows --------------------------------------------------------------------

/** One row of the card's status box (§16.9 3.3, 3.4), worst first in its own tone. */
export type FixStatus =
  | { kind: "conflict"; conflict: DictionaryConflict }
  | { kind: "error"; message: string }
  | { kind: "merge"; entry: DictionaryEntry; heard: string }
  | { kind: "computing" }
  | { kind: "impact"; scanned: number; matched: number }
  | { kind: "kept"; kept: number }
  | { kind: "short"; short: ShortForm };

export interface FixStatusInput {
  conflict?: DictionaryConflict | undefined;
  /** The core's refusal of the draft (a regex that does not compile, say). */
  error?: string | undefined;
  merge?: DictionaryEntry | undefined;
  /** The new forms, as the merge row names them. */
  heard: string;
  /** The backtest's answer, `computing` while it is asked, `undefined` before. */
  backtest?: VocabularyBacktest | "computing" | undefined;
  short?: ShortForm | undefined;
}

/** At most two rows: a clash or a refusal alone; otherwise the effect on the recent takes and the
 *  one risk worth saying (forms the final texts kept, else a short form). With no risk, a merge
 *  says where the forms go; with one, the primary button's 「添加到「…」」 says it. */
export function fixStatus(input: FixStatusInput): FixStatus[] {
  if (input.conflict !== undefined) return [{ kind: "conflict", conflict: input.conflict }];
  if (input.error !== undefined) return [{ kind: "error", message: input.error }];
  const backtest = input.backtest;
  const impact: FixStatus | undefined =
    backtest === "computing"
      ? { kind: "computing" }
      : backtest !== undefined && backtest.scanned > 0
        ? { kind: "impact", scanned: backtest.scanned, matched: backtest.matched }
        : undefined;
  const risk: FixStatus | undefined =
    backtest !== undefined && backtest !== "computing" && backtest.kept > 0
      ? { kind: "kept", kept: backtest.kept }
      : input.short === undefined
        ? undefined
        : { kind: "short", short: input.short };
  const merge: FixStatus | undefined =
    input.merge === undefined
      ? undefined
      : { kind: "merge", entry: input.merge, heard: input.heard };
  const candidates: (FixStatus | undefined)[] =
    impact !== undefined && risk !== undefined ? [impact, risk] : [merge, impact, risk];
  return candidates.filter((row): row is FixStatus => row !== undefined).slice(0, 2);
}

/** A status row as it reads: its tone, its words, and the link at its end — `show` lists the
 *  examples (查看 / 收起), `openDictionary` and `viewEntry` open the dictionary. */
export interface FixStatusLine {
  tone: "info" | "warn" | "error" | "busy";
  text: string;
  link?: "show" | "openDictionary" | "viewEntry";
}

/** Why the core would refuse the draft, in words. */
function conflictText(conflict: DictionaryConflict, t: TFunction): string {
  switch (conflict.kind) {
    case "taken":
      return t("history.fix.card.taken", { heard: conflict.heard, term: conflict.owner.term });
    case "isTerm":
      return t("history.fix.card.isTerm", { heard: conflict.heard });
    case "same":
      return t("history.fix.card.same");
    case "termTaken":
      return t("history.fix.card.termTaken", { term: conflict.term, owner: conflict.owner.term });
    case "tooMany":
      return t("history.fix.card.tooMany", { max: conflict.max });
    case "full":
      return t("history.fix.card.full", { max: conflict.max });
    case "exists":
      return t("history.fix.card.exists", { term: conflict.entry.term });
  }
}

/** The rows of `fixStatus` in words, for the desktop's card and the phone's sheet alike. `heard`
 *  names the forms (or the rule's pattern), `rule` says the draft is a rule, `examples` how many
 *  takes the backtest gave as examples: the effect row (or, when there is one, the kept-forms row)
 *  links to them. */
export function fixStatusLines(
  rows: readonly FixStatus[],
  context: { heard: string; rule: boolean; examples: number },
  t: TFunction = zhT.t,
): FixStatusLine[] {
  const { heard, rule, examples } = context;
  const show = examples > 0 ? ({ link: "show" } as const) : {};
  const kept = rows.some((r) => r.kind === "kept");
  return rows.map((row): FixStatusLine => {
    switch (row.kind) {
      case "conflict": {
        const c = row.conflict;
        const link =
          c.kind === "exists"
            ? ({ link: "viewEntry" } as const)
            : c.kind === "taken" || c.kind === "isTerm" || c.kind === "termTaken"
              ? ({ link: "openDictionary" } as const)
              : {};
        return { tone: "error", text: conflictText(c, t), ...link };
      }
      case "error":
        return { tone: "error", text: row.message };
      case "merge":
        return {
          tone: "info",
          text: t("history.fix.card.merge", {
            term: row.entry.term,
            heard,
            n: row.entry.heard_as.length,
          }),
        };
      case "computing":
        return { tone: "busy", text: t("history.fix.card.computing") };
      case "impact": {
        const text = rule
          ? row.matched > 0
            ? t("history.fix.card.ruleImpact", { scanned: row.scanned, n: row.matched })
            : t("history.fix.card.ruleImpactNone", { scanned: row.scanned })
          : row.matched > 0
            ? t("history.fix.card.impact", { scanned: row.scanned, n: row.matched, heard })
            : t("history.fix.card.impactNone", { scanned: row.scanned, heard });
        return { tone: "info", text, ...(kept ? {} : show) };
      }
      case "kept":
        return { tone: "warn", text: t("history.fix.card.kept", { n: row.kept, heard }), ...show };
      case "short":
        return {
          tone: "warn",
          text: row.short.latin
            ? t("history.fix.card.shortLatin", { n: row.short.chars })
            : t("history.fix.card.short"),
        };
    }
  });
}

/** The tone of the box around the rows: its worst row's (a busy row reads as information). */
export function fixStatusTone(lines: readonly FixStatusLine[]): "info" | "warn" | "error" {
  if (lines.some((l) => l.tone === "error")) return "error";
  if (lines.some((l) => l.tone === "warn")) return "warn";
  return "info";
}

/** A backtest example's `before`, split around what the draft changes there: `[lead, hit, tail]`. */
export function snippetParts(before: string, after: string): [string, string, string] {
  const b = Array.from(before);
  const a = Array.from(after);
  let prefix = 0;
  while (prefix < b.length && prefix < a.length && b[prefix] === a[prefix]) prefix += 1;
  let suffix = 0;
  while (
    suffix < b.length - prefix &&
    suffix < a.length - prefix &&
    b[b.length - 1 - suffix] === a[a.length - 1 - suffix]
  )
    suffix += 1;
  return [
    b.slice(0, prefix).join(""),
    b.slice(prefix, b.length - suffix).join(""),
    b.slice(b.length - suffix).join(""),
  ];
}

/** Characters of unchanged text kept on each side of an excerpt's changes. */
export const EXCERPT_CONTEXT_CHARS = 12;

/** The diff of `before` → `after` cut to its changes and `EXCERPT_CONTEXT_CHARS` characters around
 *  them, `…` where the text goes on: the rule card's 本条预览 (§16.9 3.4). Empty when nothing
 *  changes. Past `MAX_PAIR_DIFF_CELLS` the changes are taken as one stretch, from the first
 *  difference to the last. */
export function excerptSegments(before: string, after: string): DiffSegment[] {
  let segs: DiffSegment[];
  if (diffTokens(before).length * diffTokens(after).length > MAX_PAIR_DIFF_CELLS) {
    const [lead, hit, tail] = snippetParts(before, after);
    const added = Array.from(after)
      .slice(Array.from(lead).length, Array.from(after).length - Array.from(tail).length)
      .join("");
    segs = [
      { kind: "same", text: lead },
      { kind: "del", text: hit },
      { kind: "add", text: added },
      { kind: "same", text: tail },
    ].filter((s): s is DiffSegment => s.text.length > 0);
  } else {
    segs = diffSegments({ rawText: before, text: after });
  }
  const first = segs.findIndex((s) => s.kind !== "same");
  if (first < 0) return [];
  let last = segs.length - 1;
  while (segs[last]?.kind === "same") last -= 1;
  const out: DiffSegment[] = [];
  const lead = segs[first - 1];
  if (lead !== undefined) {
    const letters = Array.from(lead.text);
    const cut = letters.length > EXCERPT_CONTEXT_CHARS ? "…" : "";
    out.push({ kind: "same", text: `${cut}${letters.slice(-EXCERPT_CONTEXT_CHARS).join("")}` });
  }
  out.push(...segs.slice(first, last + 1));
  const tail = segs[last + 1];
  if (tail !== undefined) {
    const letters = Array.from(tail.text);
    const cut = letters.length > EXCERPT_CONTEXT_CHARS ? "…" : "";
    out.push({ kind: "same", text: `${letters.slice(0, EXCERPT_CONTEXT_CHARS).join("")}${cut}` });
  }
  return out;
}

// ---- where the fix applies ---------------------------------------------------------------------

/** Whether a history entry gets the quick fix (§16.9 3.8): `fix` for a take this device
 *  recognised, `note` with the line to show instead, `none` for what was not recognised here and
 *  needs no word (a text a phone sent) or is not offered yet (a voice edit). */
export type FixScope =
  | { kind: "fix" }
  | { kind: "note"; note: "phone" | "computer" }
  | { kind: "none" };

export function fixScope(
  entry: Pick<HistoryEntry, "kind" | "origin">,
  device: "desktop" | "phone",
): FixScope {
  if (entry.kind === "edit") return { kind: "none" };
  const origin = entry.origin?.kind;
  if (origin === "typed" || origin === "clipboard") return { kind: "none" };
  if (device === "desktop") {
    if (origin === undefined || origin === "take") return { kind: "fix" };
    if (origin === "standalone") return { kind: "note", note: "phone" };
    return { kind: "none" };
  }
  if (origin === undefined) return { kind: "fix" };
  if (origin === "sent") return { kind: "note", note: "computer" };
  return { kind: "none" };
}

// ---- words ------------------------------------------------------------------------------------

/** What `Intl.Segmenter` with `granularity: "word"` gives, as far as the words need it. */
export interface WordSegmenter {
  segment(text: string): Iterable<{ segment: string; index: number; isWordLike?: boolean }>;
}

/** `Intl.Segmenter` for words where the engine has it (the desktop webviews); `null` where it does
 *  not (Hermes, the phone's engine, docs/mobile-rn.md): the words are then split by script. */
export function wordSegmenter(): WordSegmenter | null {
  return "Segmenter" in Intl ? new Intl.Segmenter("zh", { granularity: "word" }) : null;
}

/** One piece of a text the phone shows as a tappable word: a word, a mark (one piece, whatever it
 *  holds), or white space and punctuation between them (`word: false`). */
export interface WordPiece extends TextRange {
  text: string;
  word: boolean;
  mark?: TextMark;
}

/** Without a segmenter: Han and kana one character a word (they are written without spaces), the
 *  other letters and digits by the word, white space and every other character on its own. */
const FALLBACK_PIECE =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]|(?:(?![\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}])[\p{L}\p{M}\p{N}_])+(?:['’.-](?:(?![\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}])[\p{L}\p{M}\p{N}_])+)*|\s+|[\s\S]/gu;
const WORDISH = /[\p{L}\p{N}]/u;

function splitWords(text: string, offset: number, segmenter: WordSegmenter | null): WordPiece[] {
  const out: WordPiece[] = [];
  if (segmenter !== null) {
    for (const s of segmenter.segment(text)) {
      out.push({
        start: offset + s.index,
        end: offset + s.index + s.segment.length,
        text: s.segment,
        word: s.isWordLike ?? WORDISH.test(s.segment),
      });
    }
    return out;
  }
  for (const m of text.matchAll(FALLBACK_PIECE)) {
    const piece = m[0];
    out.push({
      start: offset + m.index,
      end: offset + m.index + piece.length,
      text: piece,
      word: WORDISH.test(piece),
    });
  }
  return out;
}

/** `text` as the phone's tappable words (§16.9 4): each mark one piece, the rest split into words
 *  by `segmenter` (by script without one). */
export function wordPieces(
  text: string,
  marks: readonly TextMark[] = [],
  segmenter: WordSegmenter | null = wordSegmenter(),
): WordPiece[] {
  const out: WordPiece[] = [];
  let at = 0;
  const ordered = [...marks];
  ordered.sort((a, b) => a.start - b.start);
  for (const mark of ordered) {
    if (mark.start < at) continue;
    out.push(...splitWords(text.slice(at, mark.start), at, segmenter));
    out.push({
      start: mark.start,
      end: mark.end,
      text: text.slice(mark.start, mark.end),
      word: true,
      mark,
    });
    at = mark.end;
  }
  out.push(...splitWords(text.slice(at), at, segmenter));
  return out.filter((p) => p.end > p.start);
}

/** The word at `offset` of `text` (the desktop's double click, §16.9 3.2: the same words on every
 *  platform), `undefined` on white space or punctuation. */
export function wordAt(
  text: string,
  offset: number,
  segmenter: WordSegmenter | null = wordSegmenter(),
): TextRange | undefined {
  const piece = splitWords(text, 0, segmenter).find((p) => p.start <= offset && offset < p.end);
  return piece?.word === true ? { start: piece.start, end: piece.end } : undefined;
}

/** The phone's selection after a tap on `piece` (§16.9 4): a word selects itself, a tap on another
 *  word extends the selection to it, a tap inside the selection clears it; a mark selects itself
 *  whole (and clears when it is the selection); white space and punctuation do nothing. */
export function tapPiece(
  selection: TextRange | undefined,
  piece: WordPiece,
): TextRange | undefined {
  if (!piece.word) return selection;
  if (piece.mark !== undefined) {
    return selection?.start === piece.start && selection.end === piece.end
      ? undefined
      : { start: piece.start, end: piece.end };
  }
  if (selection === undefined) return { start: piece.start, end: piece.end };
  if (piece.start >= selection.start && piece.end <= selection.end) return undefined;
  return { start: Math.min(selection.start, piece.start), end: Math.max(selection.end, piece.end) };
}
