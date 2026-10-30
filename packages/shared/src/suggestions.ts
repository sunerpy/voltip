// The dictionary's learning suggestions (docs/dictation.md §16.10) on the TypeScript side: where
// 加入词典 adds a pair and what its backtest tries (the page), and the core's answers for the
// in-memory backend — the suggestions, 加入词典's checks and the rules page's hints — with the
// semantics of `voltip_core::vocabulary::{suggest, rule_warnings}`.

import { diffTokens } from "./refine";
import { MAX_PAIR_DIFF_CELLS, takePolishPairs } from "./quick-fix";
import {
  type BacktestDraft,
  type DictionaryEntry,
  type HistoryEntry,
  type IgnoredPair,
  type ReplacementRule,
  type RuleWarning,
  type Suggestion,
  type SuggestionRefusal,
  type SuggestionsAnswer,
  BACKTEST_SNIPPET_CHARS,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  MAX_SUGGESTIONS,
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

/** `vocabulary_suggestions` as the core answers it, over `takes` (newest first, as the history
 *  holds them): the AI clean-up's pairs (`takePolishPairs`, the core's comparison), without the
 *  ones an enabled entry corrects already and the ignored ones, most frequent first, then the most
 *  recent; at most `limit` of them. Like the core it stops reading older takes once the cells it
 *  compared would pass `scanCells` (a take whose corrections or rules fired is compared twice),
 *  and `scanned` counts the takes it read. */
export function suggestVocabulary(
  dictionary: readonly DictionaryEntry[],
  rules: readonly ReplacementRule[],
  takes: readonly HistoryEntry[],
  ignored: readonly IgnoredPair[],
  limit: number = MAX_SUGGESTIONS,
  scanCells: number = MAX_SCAN_CELLS,
): ReadySuggestions {
  const known = dictionary.filter((e) => e.enabled).flatMap((e) => e.heard_as);
  const found = new Map<string, Suggestion>();
  let skipped = 0;
  let scanned = 0;
  let cells = 0;
  for (const take of takes) {
    if (!take.refined) {
      scanned += 1;
      continue;
    }
    const size = (diffTokens(take.raw_text).length + 1) * (diffTokens(take.text).length + 1);
    if (size > MAX_PAIR_DIFF_CELLS) {
      scanned += 1;
      skipped += 1;
      continue;
    }
    const fired =
      (take.vocabulary?.corrections.length ?? 0) + (take.vocabulary?.rules.length ?? 0) > 0;
    const cost = fired ? size * 2 : size;
    if (cells + cost > scanCells) break;
    cells += cost;
    scanned += 1;
    for (const pair of takePolishPairs(take, dictionary, rules)) {
      const key = JSON.stringify([pair.heard, pair.polished]);
      let suggestion = found.get(key);
      if (suggestion === undefined) {
        suggestion = {
          heard: pair.heard,
          polished: pair.polished,
          count: 0,
          last_at_ms: take.at_ms,
          examples: [],
        };
        found.set(key, suggestion);
      }
      suggestion.count += 1;
      suggestion.last_at_ms = Math.max(suggestion.last_at_ms, take.at_ms);
      if (
        suggestion.examples.length < 3 &&
        !suggestion.examples.some((e) => e.history_id === take.id)
      ) {
        suggestion.examples.push({
          history_id: take.id,
          snippet: snippetAround(take.raw_text, pair.start, pair.end),
        });
      }
    }
  }
  const pairs = [...found.values()].filter(
    (s) =>
      !known.some((k) => sameIgnoringAsciiCase(k, s.heard)) &&
      !ignored.some((i) => i.heard === s.heard && i.polished === s.polished),
  );
  pairs.sort((a, b) => b.count - a.count || b.last_at_ms - a.last_at_ms);
  return {
    status: "ready",
    scanned,
    refined: takes.slice(0, scanned).filter((t) => t.refined).length,
    skipped,
    total: pairs.length,
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
