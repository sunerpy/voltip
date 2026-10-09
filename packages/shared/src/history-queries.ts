// The history queries of the mock backend (docs/dictation.md §4.4–4.5), answered from the full
// list it keeps in memory with the semantics of `voltip_core::history` (`reader.rs`,
// `derived.rs`), so the preview and the tests see what the database answers. Only
// `mock-backend.ts` imports this module: a release bundle carries neither.
import { LOCALES } from "./i18n/runtime";
import { sceneLabel } from "./labels";
import {
  HISTORY_QUERY_LIMIT,
  HISTORY_STATS_BOUNDARIES,
  type HistoryEntry,
  type HistoryHits,
  type HistoryPage,
  type HistoryQueryArgs,
  type HistoryStats,
  type HistoryStatsBucket,
} from "./schema";

/** `voltip_core::history::derived::CORRECTION_PIECE_CHARS`. */
export const CORRECTION_PIECE_CHARS = 2000;

/** Dictations count; voice edits, and texts or the clipboard a phone sent, do not. */
export function countsForStats(entry: HistoryEntry): boolean {
  return (
    entry.kind === "dictation" &&
    entry.origin?.kind !== "typed" &&
    entry.origin?.kind !== "clipboard"
  );
}

function levenshtein(a: readonly string[], b: readonly string[]): number {
  let row = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const next = [i];
    for (let j = 1; j <= b.length; j++) {
      const substitution = (row[j - 1] ?? 0) + (a[i - 1] === b[j - 1] ? 0 : 1);
      next.push(Math.min((row[j] ?? 0) + 1, (next[j - 1] ?? 0) + 1, substitution));
    }
    row = next;
  }
  return row[b.length] ?? 0;
}

/** The code points of `s` that are not white space. */
function withoutWhiteSpace(s: string): string[] {
  return Array.from(s).filter((c) => !/\p{White_Space}/u.test(c));
}

/** The characters between the transcript and the text delivered: their Levenshtein distance in
 *  code points, white space left out, piece by piece past `CORRECTION_PIECE_CHARS`. */
export function correctedChars(raw: string, text: string): number {
  const a = withoutWhiteSpace(raw);
  const b = withoutWhiteSpace(text);
  const pieces = Math.max(1, Math.ceil(Math.max(a.length, b.length) / CORRECTION_PIECE_CHARS));
  let total = 0;
  for (let i = 0; i < pieces; i++) {
    const piece = (chars: string[]) =>
      chars.slice(
        Math.floor((chars.length * i) / pieces),
        Math.floor((chars.length * (i + 1)) / pieces),
      );
    total += levenshtein(piece(a), piece(b));
  }
  return total;
}

/** Does `entry` contain `query` (case-insensitive) in one of the fields the search looks in: a
 *  built-in scene's name in both languages, the text, the transcript, both models, the
 *  application's name and id, the scene's name, the voice edit's instruction and selection. */
export function matchesHistoryQuery(entry: HistoryEntry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return true;
  const builtinNames =
    entry.scene?.builtin === undefined
      ? []
      : LOCALES.map((locale) => sceneLabel({ name: "", builtin: entry.scene?.builtin }, locale));
  return [
    ...builtinNames,
    entry.text,
    entry.raw_text,
    entry.asr_model,
    entry.refine_model ?? "",
    entry.app?.name ?? "",
    entry.app?.id ?? "",
    entry.scene?.name ?? "",
    entry.edit?.instruction ?? "",
    entry.edit?.selection ?? "",
  ].some((s) => s.toLowerCase().includes(q));
}

/** Newest first by time; the list order (the newest written first) breaks ties, as the rowid
 *  does in the database. */
function newestFirst(entries: readonly HistoryEntry[]): HistoryEntry[] {
  const indexed = entries.map((entry, index) => ({ entry, index }));
  indexed.sort((x, y) => y.entry.at_ms - x.entry.at_ms || x.index - y.index);
  return indexed.map(({ entry }) => entry);
}

/** `history_query`. Throws the core's message for a `limit` outside 1–`HISTORY_QUERY_LIMIT`. */
export function historyPageOf(
  entries: readonly HistoryEntry[],
  args: HistoryQueryArgs,
): HistoryPage {
  if (!Number.isInteger(args.limit) || args.limit < 1 || args.limit > HISTORY_QUERY_LIMIT) {
    throw new Error(`history_query: limit 1–${HISTORY_QUERY_LIMIT}`);
  }
  const since = args.sinceMs ?? undefined;
  const matching = newestFirst(entries).filter(
    (e) =>
      (since === undefined || e.at_ms >= since) &&
      (args.starred !== true || e.starred) &&
      (args.failed !== true || e.outcome.kind !== "inserted") &&
      matchesHistoryQuery(e, args.query ?? ""),
  );
  const offset = args.offset ?? 0;
  return {
    entries: matching.slice(offset, offset + args.limit),
    matching: matching.length,
    total: entries.length,
  };
}

function emptyBucket(): HistoryStatsBucket {
  return { count: 0, raw_chars: 0, corrected_chars: 0, spoken_ms: 0, latency_ms: 0 };
}

function addEntry(bucket: HistoryStatsBucket, entry: HistoryEntry) {
  bucket.count += 1;
  bucket.raw_chars += Array.from(entry.raw_text).length;
  bucket.corrected_chars += correctedChars(entry.raw_text, entry.text);
  bucket.spoken_ms += entry.duration_ms;
  bucket.latency_ms += entry.asr_ms + (entry.refine_ms ?? 0);
}

/** `history_stats`. Throws the core's message unless `boundaries` has 2–43 increasing values. */
export function historyStatsOf(
  entries: readonly HistoryEntry[],
  boundaries: readonly number[],
): HistoryStats {
  const first = boundaries[0];
  const last = boundaries[boundaries.length - 1];
  if (
    first === undefined ||
    last === undefined ||
    boundaries.length < 2 ||
    boundaries.length > HISTORY_STATS_BOUNDARIES ||
    boundaries.some((b, i) => i > 0 && b <= (boundaries[i - 1] ?? b))
  ) {
    throw new Error(`history_stats: 2–${HISTORY_STATS_BOUNDARIES} increasing boundaries`);
  }
  const buckets = Array.from({ length: boundaries.length - 1 }, emptyBucket);
  const total = emptyBucket();
  for (const entry of entries) {
    if (!countsForStats(entry)) continue;
    addEntry(total, entry);
    if (entry.at_ms < first || entry.at_ms >= last) continue;
    const index = boundaries.filter((b) => b <= entry.at_ms).length - 1;
    const bucket = buckets[index];
    if (bucket) addEntry(bucket, entry);
  }
  return { buckets, total };
}

/** `history_hits`. */
export function historyHitsOf(entries: readonly HistoryEntry[]): HistoryHits {
  const out: HistoryHits = { dictionary: {}, rules: {} };
  for (const entry of entries) {
    for (const hit of entry.vocabulary?.corrections ?? []) {
      out.dictionary[hit.id] = (out.dictionary[hit.id] ?? 0) + hit.count;
    }
    for (const hit of entry.vocabulary?.rules ?? []) {
      out.rules[hit.id] = (out.rules[hit.id] ?? 0) + hit.count;
    }
  }
  return out;
}
