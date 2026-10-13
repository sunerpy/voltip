// A backup's import as the in-memory backend runs it (docs/dictation.md §4.6), with the rules of
// `voltip_core::backup::merge` and `HistoryStore::restore`: an item here already (the same id, or
// the same term, name or built-in scene), one the list's checks refuse, one that contradicts the
// dictionary or one past the list's limit is skipped and counted; `replace` starts from the
// backup's items alone; the history keeps the newest `keep` of both. Only the mock imports it.
import {
  type BackupManifest,
  type BackupPart,
  type BackupPartReport,
  type BackupSkipped,
  type CustomPreset,
  type DictionaryEntry,
  type HistoryEntry,
  type ImportMode,
  MAX_DICTIONARY_ENTRIES,
  MAX_PRESETS,
  MAX_RULES,
  MAX_SCENES,
  type ReplacementRule,
  type Scene,
} from "./schema";
import { validatePresetDraft } from "./presets";
import { validateSceneDraft } from "./scenes";
import { checkDictionary, validateDictionaryDraft, validateRuleDraft } from "./vocabulary";

/** What a backup file holds besides its manifest. */
export interface BackupContent {
  history: HistoryEntry[];
  dictionary: DictionaryEntry[];
  rules: ReplacementRule[];
  scenes: Scene[];
  presets: CustomPreset[];
}

/** A backup file of the mock's own "file system". */
export interface MockBackupFile extends BackupContent {
  manifest: BackupManifest;
}

type Skip = keyof BackupSkipped;

/** Nothing restored or skipped yet. */
export function partReport(part: BackupPart, inFile: number): BackupPartReport {
  return {
    part,
    in_file: inFile,
    restored: 0,
    skipped: { duplicate: 0, limit: 0, conflict: 0, invalid: 0, older: 0 },
    removed: 0,
  };
}

/** A–Z folded to a–z (the core compares names ignoring ASCII case only). */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

function sameAscii(a: string, b: string): boolean {
  return asciiLower(a) === asciiLower(b);
}

/** `validate` throws: the item is not one the list keeps. */
function refused(validate: () => unknown): boolean {
  try {
    validate();
    return false;
  } catch (_e) {
    return true;
  }
}

/** The list merged item by item: `why` says why one cannot go in (the same id always). */
function mergeList<T extends { id: string }>(
  part: BackupPart,
  current: readonly T[],
  incoming: readonly T[],
  mode: ImportMode,
  why: (list: readonly T[], item: T) => Skip | undefined,
): { list: T[]; report: BackupPartReport } {
  const list = mode === "merge" ? [...current] : [];
  const report = partReport(part, incoming.length);
  for (const item of incoming) {
    const skip = list.some((x) => x.id === item.id) ? "duplicate" : why(list, item);
    if (skip === undefined) {
      list.push(structuredClone(item));
      report.restored += 1;
    } else {
      report.skipped[skip] += 1;
    }
  }
  return { list, report };
}

export function mergeDictionary(
  current: readonly DictionaryEntry[],
  incoming: readonly DictionaryEntry[],
  mode: ImportMode,
) {
  return mergeList("dictionary", current, incoming, mode, (list, entry) => {
    if (refused(() => validateDictionaryDraft(entry))) return "invalid";
    if (list.some((e) => sameAscii(e.term, entry.term))) return "duplicate";
    if (list.length >= MAX_DICTIONARY_ENTRIES) return "limit";
    if (refused(() => checkDictionary([...list, entry]))) return "conflict";
    return undefined;
  });
}

export function mergeRules(
  current: readonly ReplacementRule[],
  incoming: readonly ReplacementRule[],
  mode: ImportMode,
) {
  return mergeList("rules", current, incoming, mode, (list, rule) => {
    if (refused(() => validateRuleDraft(rule))) return "invalid";
    if (list.some((r) => r.name === rule.name)) return "duplicate";
    if (list.length >= MAX_RULES) return "limit";
    return undefined;
  });
}

/** The presets merged, and for each preset of the backup skipped because one here has its name,
 *  the id of the one here (the backup's scenes that name it name that one). */
export function mergePresets(
  current: readonly CustomPreset[],
  incoming: readonly CustomPreset[],
  mode: ImportMode,
) {
  const same = new Map<string, string>();
  const merged = mergeList("presets", current, incoming, mode, (list, preset) => {
    if (refused(() => validatePresetDraft(preset))) return "invalid";
    const here = list.find((p) => sameAscii(p.name, preset.name));
    if (here !== undefined) {
      same.set(preset.id, here.id);
      return "duplicate";
    }
    if (list.length >= MAX_PRESETS) return "limit";
    return undefined;
  });
  return { ...merged, same };
}

/** The scenes merged: a built-in scene stands for its category (merging keeps the one here;
 *  replacing gives the one here the backup's settings under its own id); when merging, the
 *  backup's own scenes go before the first built-in one. */
export function mergeScenes(
  current: readonly Scene[],
  incoming: readonly Scene[],
  mode: ImportMode,
  samePreset: ReadonlyMap<string, string>,
) {
  const list: Scene[] = mode === "merge" ? [...current] : [];
  const added: Scene[] = [];
  const report = partReport("scenes", incoming.length);
  for (const item of incoming) {
    const all = [...list, ...added];
    let skip: Skip | undefined;
    if (all.some((s) => s.id === item.id)) skip = "duplicate";
    else if (refused(() => validateSceneDraft(item, item.builtin === undefined))) skip = "invalid";
    else if (item.builtin !== undefined)
      skip = all.some((s) => s.builtin === item.builtin)
        ? mode === "merge"
          ? "duplicate"
          : "conflict"
        : undefined;
    else if (all.some((s) => s.builtin === undefined && sameAscii(s.name, item.name)))
      skip = "duplicate";
    else if (all.filter((s) => s.builtin === undefined).length >= MAX_SCENES) skip = "limit";
    if (skip !== undefined) {
      report.skipped[skip] += 1;
      continue;
    }
    const scene = structuredClone(item);
    const preset = scene.overrides.refine_preset;
    const here = preset === undefined || preset === null ? undefined : samePreset.get(preset);
    if (here !== undefined) scene.overrides.refine_preset = here;
    report.restored += 1;
    if (scene.builtin !== undefined) {
      const mine = current.find((s) => s.builtin === scene.builtin);
      if (mine !== undefined) {
        scene.id = mine.id;
        scene.created_at_ms = mine.created_at_ms;
      }
      list.push(scene);
    } else if (mode === "merge") added.push(scene);
    else list.push(scene);
  }
  const at = list.findIndex((s) => s.builtin !== undefined);
  list.splice(at === -1 ? list.length : at, 0, ...added);
  return { list, report };
}

/** The history restored: merging adds the backup's entries not here, replacing starts from the
 *  backup alone; either way the newest `keep` of all stay (within a millisecond an entry written
 *  now is the newer). */
export function mergeHistory(
  current: readonly HistoryEntry[],
  incoming: readonly HistoryEntry[],
  mode: ImportMode,
  keep: number,
) {
  const here = mode === "merge" ? current : [];
  const known = new Set(here.map((e) => e.id));
  const report = partReport("history", incoming.length);
  const fresh: HistoryEntry[] = [];
  for (const entry of incoming) {
    if (known.has(entry.id)) {
      report.skipped.duplicate += 1;
      continue;
    }
    known.add(entry.id);
    fresh.push(structuredClone(entry));
  }
  // Newest first; a stable sort keeps the backup's order within a millisecond, and the entries
  // written now come before the ones here of the same millisecond.
  const all = [
    ...fresh.map((entry) => ({ entry, fresh: true })),
    ...here.map((entry) => ({ entry, fresh: false })),
  ];
  all.sort((a, b) => b.entry.at_ms - a.entry.at_ms);
  const kept = all.slice(0, keep);
  report.restored = kept.filter((k) => k.fresh).length;
  report.skipped.older = fresh.length - report.restored;
  report.removed =
    mode === "merge" ? here.length - (kept.length - report.restored) : current.length;
  return { list: kept.map((k) => k.entry), report };
}
