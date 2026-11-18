// The dictionary's learning suggestions (docs/dictation.md §16.10) on the TypeScript side: where
// 加入词典 adds a pair and what its backtest tries (the page), and the core's answers for the
// in-memory backend — the suggestions, 加入词典's checks, a batch of them and its 撤销, and the
// rules page's hints — with the semantics of `voltip_core::vocabulary::{suggest, rule_warnings}`.

import { diffTokens } from "./refine";
import { MAX_PAIR_DIFF_CELLS, type PolishPair, takeEditPairs, takePolishPairs } from "./quick-fix";
import {
  type BacktestDraft,
  type BatchItem,
  type BatchUndoOutcome,
  type DictionaryEntry,
  type HistoryEntry,
  type IgnoredPair,
  type ReplacementRule,
  type RuleWarning,
  type Suggestion,
  type SuggestionPick,
  type SuggestionRefusal,
  type SuggestionsAnswer,
  BACKTEST_SNIPPET_CHARS,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_SUGGESTIONS,
  SUGGESTION_REPEATED_MIN,
} from "./schema";
import { Vocabulary, validateDictionaryDraft } from "./vocabulary";

/** The answer with suggestions in it (history recording on). */
export type ReadySuggestions = Extract<SuggestionsAnswer, { status: "ready" }>;

/** A–Z folded to a–z. */
function foldAscii(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** The core compares right spellings and forms ignoring ASCII case only. */
function sameIgnoringAsciiCase(a: string, b: string): boolean {
  return foldAscii(a) === foldAscii(b);
}

/** Whether `entry` already has the heard form `form` (ASCII case ignored). */
function hasForm(entry: DictionaryEntry, form: string): boolean {
  return entry.heard_as.some((h) => sameIgnoringAsciiCase(h, form));
}

/** 加入词典 refused because of `entry`. */
function clash(
  entry: DictionaryEntry,
  kind: Extract<SuggestionRefusal, { kind: "entry" }>["clash"],
): SuggestionRefusal {
  return { kind: "entry", entry_id: entry.id, term: entry.term, clash: kind };
}

/** The entry whose right spelling `term` is (ASCII case ignored): 加入词典 adds the heard form to
 *  it instead of making a new entry. */
export function suggestionTarget(
  dictionary: readonly DictionaryEntry[],
  term: string,
): DictionaryEntry | undefined {
  return dictionary.find((e) => sameIgnoringAsciiCase(e.term, term));
}

/** What 加入词典 on `heard` → `polished` would save, for `vocabulary_backtest`: the entry with that
 *  spelling with `heard` added, or a new entry. */
export function suggestionDraft(
  dictionary: readonly DictionaryEntry[],
  heard: string,
  polished: string,
): Extract<BacktestDraft, { kind: "dictionary" }> {
  const target = suggestionTarget(dictionary, polished);
  if (target === undefined) {
    return {
      kind: "dictionary",
      id: null,
      entry: { term: polished, heard_as: [heard], enabled: true },
    };
  }
  const known = hasForm(target, heard);
  return {
    kind: "dictionary",
    id: target.id,
    entry: {
      term: target.term,
      heard_as: known ? target.heard_as : [...target.heard_as, heard],
      enabled: target.enabled,
    },
  };
}

/** The recognised text around `start..end`, `BACKTEST_SNIPPET_CHARS` on each side, `…` where it
 *  goes on (the core's snippet). */
function snippetAround(raw: string, start: number, end: number): string {
  const from = Math.max(0, start - BACKTEST_SNIPPET_CHARS);
  const to = Math.min(raw.length, end + BACKTEST_SNIPPET_CHARS);
  return `${from > 0 ? "…" : ""}${raw.slice(from, to)}${to < raw.length ? "…" : ""}`;
}

/** `voltip_core::mcp::candidates::MAX_SCAN_CELLS`: the comparison cells one answer spends at most;
 *  older takes are not read past it. */
export const MAX_SCAN_CELLS = 50_000_000;

/** One pair of one take before the counting (`voltip_core::mcp::candidates::Found`). */
interface Found {
  take: number;
  pair: PolishPair;
  /** The user's correction made it (§35), not the AI clean-up. */
  edit: boolean;
}

/** One CJK character: Han, kana or Hangul (`is_cjk_letter`). */
const CJK_LETTER =
  /^[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}々〆〇]$/u;

function singleCjk(text: string): boolean {
  return CJK_LETTER.test(text);
}

/** How often `needle` occurs in `text`, overlapping occurrences counted. */
function occurrences(text: string, needle: string): number {
  let n = 0;
  for (let at = text.indexOf(needle); at >= 0; at = text.indexOf(needle, at + 1)) n += 1;
  return n;
}

/** A pair widened by one unchanged neighbour (`voltip_core::mcp::candidates::widen`). */
interface Widened {
  found: Found;
  /** The pair before it was widened, when it was. */
  single?: { heard: string; polished: string };
}

/** Every pair whose heard or right form is a single CJK character, widened by the same
 *  unchanged neighbour on both sides (docs/dictation.md §16.10): 非 → 飞 in 「非书」 becomes
 *  非书 → 飞书. With a neighbour on both sides, the side whose widened right form occurs more often
 *  in the compared takes' final texts and corrections wins, the right one on a tie. */
function widen(
  takes: readonly HistoryEntry[],
  found: readonly Found[],
  compared: readonly number[],
): Widened[] {
  const options = found.map((f) => {
    const { pair } = f;
    if (!singleCjk(pair.heard) && !singleCjk(pair.polished)) return [];
    const take = takes[f.take];
    if (take === undefined) return [];
    const raw = take.raw_text;
    const right = f.edit ? (take.corrected?.text ?? take.text) : take.text;
    const out: {
      heard: string;
      polished: string;
      start: number;
      end: number;
      polishedStart: number;
      polishedEnd: number;
    }[] = [];
    const after = Array.from(raw.slice(pair.end))[0];
    const afterRight = Array.from(right.slice(pair.polishedEnd))[0];
    if (after !== undefined && after === afterRight && singleCjk(after)) {
      out.push({
        heard: pair.heard + after,
        polished: pair.polished + after,
        start: pair.start,
        end: pair.end + after.length,
        polishedStart: pair.polishedStart,
        polishedEnd: pair.polishedEnd + after.length,
      });
    }
    const before = Array.from(raw.slice(0, pair.start)).at(-1);
    const beforeRight = Array.from(right.slice(0, pair.polishedStart)).at(-1);
    if (before !== undefined && before === beforeRight && singleCjk(before)) {
      out.push({
        heard: before + pair.heard,
        polished: before + pair.polished,
        start: pair.start - before.length,
        end: pair.end,
        polishedStart: pair.polishedStart - before.length,
        polishedEnd: pair.polishedEnd,
      });
    }
    return out;
  });
  const counts = new Map<string, number>();
  for (const o of options) {
    if (o.length < 2) continue;
    for (const w of o) {
      if (counts.has(w.polished)) continue;
      let n = 0;
      for (const index of compared) {
        const take = takes[index];
        if (take === undefined) continue;
        n += occurrences(take.text, w.polished);
        if (take.corrected !== undefined) n += occurrences(take.corrected.text, w.polished);
      }
      counts.set(w.polished, n);
    }
  }
  return found.map((f, i) => {
    const o = options[i] ?? [];
    const [rightSide, leftSide] = o;
    const chosen =
      rightSide === undefined
        ? undefined
        : leftSide !== undefined &&
            (counts.get(leftSide.polished) ?? 0) > (counts.get(rightSide.polished) ?? 0)
          ? leftSide
          : rightSide;
    if (chosen === undefined) return { found: f };
    return {
      found: { ...f, pair: { ...chosen } },
      single: { heard: f.pair.heard, polished: f.pair.polished },
    };
  });
}

/** The cells the token LCS of `a` and `b` compares, as the core counts them against its caps. */
function cellsOf(a: string, b: string): number {
  return (diffTokens(a).length + 1) * (diffTokens(b).length + 1);
}

/** `vocabulary_suggestions` as the core answers it, over `takes` (newest first, as the history
 *  holds them): the AI clean-up's pairs (`takePolishPairs`, the core's comparison) and the user's
 *  corrections (`takeEditPairs`, §35), widened when they were single characters and counted
 *  together, without the ones an enabled entry corrects already and the ignored ones (also when
 *  ignored before the widening), most frequent first, then the most recent; at most `limit` of
 *  them. Like the core it stops reading older takes once the cells it compared would pass
 *  `scanCells`, and `scanned` counts the takes it read. */
export function suggestVocabulary(
  dictionary: readonly DictionaryEntry[],
  rules: readonly ReplacementRule[],
  takes: readonly HistoryEntry[],
  ignored: readonly IgnoredPair[],
  limit: number = MAX_SUGGESTIONS,
  scanCells: number = MAX_SCAN_CELLS,
): ReadySuggestions {
  const known = dictionary.filter((e) => e.enabled).flatMap((e) => e.heard_as);
  const isKnown = (heard: string) => known.some((k) => sameIgnoringAsciiCase(k, heard));
  const found: Found[] = [];
  const compared: number[] = [];
  let skipped = 0;
  let scanned = 0;
  let edited = 0;
  let cells = 0;
  for (const [index, take] of takes.entries()) {
    const corrected = take.corrected?.text;
    if (!take.refined && corrected === undefined) {
      scanned += 1;
      continue;
    }
    const polish = take.refined ? cellsOf(take.raw_text, take.text) : undefined;
    const edit =
      corrected === undefined
        ? undefined
        : { text: cellsOf(take.text, corrected), raw: cellsOf(take.raw_text, take.text) };
    if (
      (polish ?? 0) > MAX_PAIR_DIFF_CELLS ||
      (edit !== undefined && (edit.text > MAX_PAIR_DIFF_CELLS || edit.raw > MAX_PAIR_DIFF_CELLS))
    ) {
      scanned += 1;
      skipped += 1;
      continue;
    }
    const fired =
      (take.vocabulary?.corrections.length ?? 0) + (take.vocabulary?.rules.length ?? 0) > 0;
    const replayCost = fired ? (polish ?? cellsOf(take.raw_text, take.raw_text)) : 0;
    const cost = (polish ?? 0) + (edit === undefined ? 0 : edit.text + edit.raw) + replayCost;
    if (cells + cost > scanCells) break;
    cells += cost;
    scanned += 1;
    compared.push(index);
    if (corrected !== undefined) edited += 1;
    for (const pair of takePolishPairs(take, dictionary, rules))
      found.push({ take: index, pair, edit: false });
    for (const pair of takeEditPairs(take, dictionary, rules))
      found.push({ take: index, pair, edit: true });
  }
  const bySuggestion = new Map<
    string,
    Suggestion & { single?: { heard: string; polished: string } }
  >();
  for (const { found: f, single } of widen(takes, found, compared)) {
    const take = takes[f.take];
    if (take === undefined) continue;
    const key = JSON.stringify([f.pair.heard, f.pair.polished]);
    let suggestion = bySuggestion.get(key);
    if (suggestion === undefined) {
      suggestion = {
        heard: f.pair.heard,
        polished: f.pair.polished,
        count: 0,
        last_at_ms: take.at_ms,
        examples: [],
        from_edits: 0,
        ...(single === undefined ? {} : { single }),
      };
      bySuggestion.set(key, suggestion);
    }
    suggestion.count += 1;
    if (f.edit) suggestion.from_edits += 1;
    suggestion.last_at_ms = Math.max(suggestion.last_at_ms, take.at_ms);
    if (
      suggestion.examples.length < 3 &&
      !suggestion.examples.some((e) => e.history_id === take.id)
    ) {
      suggestion.examples.push({
        history_id: take.id,
        snippet: snippetAround(take.raw_text, f.pair.start, f.pair.end),
      });
    }
  }
  const isIgnored = (s: {
    heard: string;
    polished: string;
    single?: { heard: string; polished: string };
  }) =>
    ignored.some(
      (i) =>
        (i.heard === s.heard && i.polished === s.polished) ||
        (s.single !== undefined && i.heard === s.single.heard && i.polished === s.single.polished),
    );
  const pairs: Suggestion[] = [...bySuggestion.values()]
    .filter(
      (s) =>
        !isKnown(s.heard) && !(s.single !== undefined && isKnown(s.single.heard)) && !isIgnored(s),
    )
    .map(({ single: _single, ...s }) => s);
  pairs.sort((a, b) => b.count - a.count || b.last_at_ms - a.last_at_ms);
  return {
    status: "ready",
    scanned,
    refined: takes.slice(0, scanned).filter((t) => t.refined).length,
    skipped,
    total: pairs.length,
    repeated: pairs.filter((p) => p.count >= SUGGESTION_REPEATED_MIN || p.from_edits > 0).length,
    edited,
    pairs: pairs.slice(0, limit),
  };
}

/** What 加入词典 did: the new dictionary, the entry before (absent: it was created) and after. */
export interface AcceptedSuggestion {
  dictionary: DictionaryEntry[];
  before?: DictionaryEntry;
  after: DictionaryEntry;
}

/** 加入词典 on `heard` → `polished` as the core does it: refused when a rule that runs rewrites the
 *  spelling, an entry is in the way or the dictionary is full; otherwise `heard` goes into the
 *  entry with that spelling, or a new entry from history entry `historyId`. */
export function acceptSuggestion(
  dictionary: readonly DictionaryEntry[],
  rules: readonly ReplacementRule[],
  heard: string,
  polished: string,
  historyId: string,
  nowMs: number,
  newId: () => string,
): AcceptedSuggestion | SuggestionRefusal {
  let draft;
  try {
    draft = validateDictionaryDraft({ term: polished, heard_as: [heard], enabled: true });
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
  const term = draft.term;
  const form = draft.heard_as[0] ?? heard;
  const ruled = Vocabulary.compile([], rules).applyRules(term);
  const first = ruled.hits[0];
  const rule = first === undefined ? undefined : rules.find((r) => r.id === first.id);
  if (ruled.text !== term && rule !== undefined) {
    return { kind: "rule", rule_id: rule.id, name: rule.name, becomes: ruled.text };
  }
  const target = suggestionTarget(dictionary, term);
  for (const other of dictionary.filter((e) => e.id !== target?.id)) {
    if (sameIgnoringAsciiCase(other.term, form)) return clash(other, "heard_is_term");
    if (hasForm(other, form)) return clash(other, "heard_taken");
    if (target === undefined && hasForm(other, term)) return clash(other, "term_taken");
  }
  if (target !== undefined) {
    if (!target.enabled) return clash(target, "disabled");
    if (hasForm(target, form)) return clash(target, "heard_taken");
    if (target.heard_as.length >= MAX_HEARD_AS) return clash(target, "too_many");
    const after = { ...target, heard_as: [...target.heard_as, form], updated_at_ms: nowMs };
    return {
      dictionary: dictionary.map((e) => (e.id === target.id ? after : e)),
      before: target,
      after,
    };
  }
  if (dictionary.length >= MAX_DICTIONARY_ENTRIES) {
    return { kind: "full", max: MAX_DICTIONARY_ENTRIES };
  }
  const after: DictionaryEntry = {
    id: newId(),
    term,
    heard_as: [form],
    enabled: true,
    source: { kind: "history", history_id: historyId },
    created_at_ms: nowMs,
    updated_at_ms: nowMs,
  };
  return { dictionary: [...dictionary, after], after };
}

/** An entry a batch 加入词典 made (`before` absent) or added to, as it was before the batch and as
 *  the batch left it. */
export interface BatchChange {
  before?: DictionaryEntry;
  after: DictionaryEntry;
}

/** What a batch 加入词典 did: the new dictionary, one item per pick in their order, and every entry
 *  it changed, once, in the order first changed (none: nothing to save). */
export interface AcceptedBatch {
  dictionary: DictionaryEntry[];
  items: BatchItem[];
  changes: BatchChange[];
}

/** A batch 加入词典 as the core does it: each pick in order with `acceptSuggestion`'s checks, against
 *  the dictionary as the picks before it left it, so picks with one right spelling go into one
 *  entry and a pick past a cap is refused on its own. */
export function acceptSuggestions(
  dictionary: readonly DictionaryEntry[],
  rules: readonly ReplacementRule[],
  picks: readonly SuggestionPick[],
  nowMs: number,
  newId: () => string,
): AcceptedBatch {
  let working = [...dictionary];
  const items: BatchItem[] = [];
  const touched = new Map<string, DictionaryEntry | undefined>();
  for (const pick of picks) {
    const result = acceptSuggestion(
      working,
      rules,
      pick.heard,
      pick.polished,
      pick.history_id,
      nowMs,
      newId,
    );
    if ("kind" in result) {
      items.push({ kind: "refused", refusal: result });
      continue;
    }
    working = result.dictionary;
    if (!touched.has(result.after.id)) touched.set(result.after.id, result.before);
    items.push({
      kind: "accepted",
      entry_id: result.after.id,
      term: result.after.term,
      created: result.before === undefined,
    });
  }
  const changes: BatchChange[] = [];
  for (const [id, before] of touched) {
    const after = working.find((e) => e.id === id);
    if (after !== undefined) changes.push(before === undefined ? { after } : { before, after });
  }
  return { dictionary: working, items, changes };
}

/** 撤销 of a batch 加入词典 as the core does it: the dictionary with every entry in `changes` back as
 *  it was (the ones the batch made gone), while each is exactly as the batch left it; otherwise the
 *  first entry deleted or changed since, nothing undone. */
export function undoSuggestions(
  dictionary: readonly DictionaryEntry[],
  changes: readonly BatchChange[],
): DictionaryEntry[] | Extract<BatchUndoOutcome, { kind: "gone" | "changed" }> {
  for (const { after } of changes) {
    const current = dictionary.find((e) => e.id === after.id);
    if (current === undefined) return { kind: "gone", term: after.term };
    if (JSON.stringify(current) !== JSON.stringify(after)) {
      return { kind: "changed", term: after.term };
    }
  }
  const made = new Set(changes.filter((c) => c.before === undefined).map((c) => c.after.id));
  const back = new Map(
    changes.flatMap((c) => (c.before === undefined ? [] : [[c.before.id, c.before]])),
  );
  return dictionary.filter((e) => !made.has(e.id)).map((e) => back.get(e.id) ?? e);
}

/** `rules_warnings` as the core answers it: for each rule that runs, in order, the first other one
 *  that finds the same (pattern, kind and case setting), the first of the same kind that turns its
 *  replacement back into its pattern, and every enabled entry whose right spelling it turns into
 *  one of its mis-hearings. */
export function ruleWarnings(
  rules: readonly ReplacementRule[],
  dictionary: readonly DictionaryEntry[],
): RuleWarning[] {
  const running = rules.filter((r) => r.enabled && !r.pending);
  const entries = dictionary.filter((e) => e.enabled);
  const out: RuleWarning[] = [];
  for (const rule of running) {
    const others = running.filter((o) => o.id !== rule.id);
    const twin = others.find(
      (o) =>
        o.kind === rule.kind &&
        o.pattern === rule.pattern &&
        o.case_sensitive === rule.case_sensitive,
    );
    if (twin !== undefined) {
      out.push({ kind: "duplicate", rule_id: rule.id, other_id: twin.id, other_name: twin.name });
    }
    const back =
      rule.pattern === rule.replacement
        ? undefined
        : others.find(
            (o) =>
              o.kind === rule.kind &&
              o.pattern === rule.replacement &&
              o.replacement === rule.pattern,
          );
    if (back !== undefined) {
      out.push({ kind: "inverse", rule_id: rule.id, other_id: back.id, other_name: back.name });
    }
    if (entries.length === 0) continue;
    const alone = Vocabulary.compile([], [rule]);
    for (const entry of entries) {
      const after = alone.applyRules(entry.term).text;
      const heard =
        after === entry.term
          ? undefined
          : entry.heard_as.find((h) => sameIgnoringAsciiCase(h, after));
      if (heard !== undefined) {
        out.push({
          kind: "undoes_entry",
          rule_id: rule.id,
          entry_id: entry.id,
          term: entry.term,
          heard,
        });
      }
    }
  }
  return out;
}
