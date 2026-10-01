// Pure helpers the dictionary, rules and history pages share (docs/dictation.md §16): hit totals
// from the core's `history_hits`, the heard-as field's separators, the core's rejection text, and
// the instant localised checks shown while typing. The core validates everything again on save; its
// refusals come back as the command's rejection or as an `error` event.
import {
  type DictionaryEntry,
  type HistoryHits,
  MAX_HEARD_AS,
  MAX_TERM_CHARS,
  type ReplacementRule,
  type TFunction,
  zhT,
} from "@voltip/shared";

/** How often each entry (`corrections`) or rule (`rules`) fired across the history, from the
 *  core's `history_hits` answer (`undefined` until it arrives: nothing counted yet). */
export function hitTotals(
  hits: HistoryHits | undefined,
  kind: "corrections" | "rules",
): Map<string, number> {
  return new Map(Object.entries((kind === "corrections" ? hits?.dictionary : hits?.rules) ?? {}));
}

/** Separators of the heard-as field: `·`, commas (both widths), `、`, semicolons, new lines. */
const HEARD_AS_SEPARATORS = /[·,，、;；\n]/;

/** `a, b · c` → `["a", "b", "c"]`; empty pieces are dropped. */
export function splitHeardAs(text: string): string[] {
  return text
    .split(HEARD_AS_SEPARATORS)
    .map((piece) => piece.trim())
    .filter((piece) => piece.length > 0);
}

/** How a list of heard-as forms is shown and edited. */
export const HEARD_AS_JOINER = " · ";

/** A rejection as text: Tauri rejects with the core's string, the in-memory backend with an Error;
 *  either way without the core's machine prefix. */
export { errorText } from "@voltip/shared";

function chars(text: string): number {
  return Array.from(text).length;
}

/** A–Z folded to a–z (the core compares terms ignoring ASCII case only). */
function asciiFold(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

function sameIgnoringAsciiCase(a: string, b: string): boolean {
  return asciiFold(a) === asciiFold(b);
}

/** What is wrong with a dictionary draft before it is sent (localised); `undefined` when nothing. */
export interface DictionaryDraftProblem {
  term?: string;
  heard?: string;
}

export function dictionaryDraftProblem(
  term: string,
  heardAs: readonly string[],
  others: readonly DictionaryEntry[],
  t: TFunction = zhT.t,
): DictionaryDraftProblem | undefined {
  const clean = term.trim();
  const problem: DictionaryDraftProblem = {};
  if (clean.length === 0) problem.term = t("dictionary.error.empty");
  else if (chars(clean) > MAX_TERM_CHARS)
    problem.term = t("dictionary.error.tooLong", { max: MAX_TERM_CHARS, n: chars(clean) });
  else if (others.some((e) => sameIgnoringAsciiCase(e.term, clean)))
    problem.term = t("dictionary.error.duplicate");
  const long = heardAs.find((h) => chars(h) > MAX_TERM_CHARS);
  if (heardAs.length > MAX_HEARD_AS)
    problem.heard = t("dictionary.error.tooMany", { max: MAX_HEARD_AS, n: heardAs.length });
  else if (long !== undefined)
    problem.heard = t("dictionary.error.tooLong", { max: MAX_TERM_CHARS, n: chars(long) });
  return problem.term === undefined && problem.heard === undefined ? undefined : problem;
}

/** What is wrong with a rule draft before it is sent (localised); a regex that does not compile
 *  is the core's to say (`vocabulary_preview` with the draft). */
export interface RuleDraftProblem {
  name?: string;
  pattern?: string;
}

export function ruleDraftProblem(
  name: string,
  pattern: string,
  others: readonly ReplacementRule[],
  t: TFunction = zhT.t,
): RuleDraftProblem | undefined {
  const clean = name.trim();
  const problem: RuleDraftProblem = {};
  if (clean.length === 0) problem.name = t("rules.validate.nameEmpty");
  else if (others.some((r) => r.name === clean)) problem.name = t("rules.validate.nameDuplicate");
  if (pattern.length === 0) problem.pattern = t("rules.validate.patternEmpty");
  return problem.name === undefined && problem.pattern === undefined ? undefined : problem;
}

/** `ids` with the one at `from` moved by `delta` places; the same order when it cannot move. */
export function moved(ids: readonly string[], from: number, delta: -1 | 1): string[] {
  const to = from + delta;
  const next = [...ids];
  const a = next[from];
  const b = next[to];
  if (a === undefined || b === undefined) return next;
  next[from] = b;
  next[to] = a;
  return next;
}
