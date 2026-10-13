// The in-memory backend's merges of a backup (docs/dictation.md section 4.6) follow the core's
// rules (`voltip_core::backup::merge`, `HistoryStore::restore`).
import {
  mergeDictionary,
  mergeHistory,
  mergePresets,
  mergeRules,
  mergeScenes,
} from "./backup-merge";
import { sampleHistory } from "./mock-backend";
import {
  type CustomPreset,
  type DictionaryEntry,
  type HistoryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_PRESETS,
  MAX_RULES,
  MAX_SCENES,
  type ReplacementRule,
  type Scene,
  dictionaryEntrySchema,
  replacementRuleSchema,
} from "./schema";

let next = 0;
const id = () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, "0")}`;

function entry(term: string, heard: string[] = []): DictionaryEntry {
  return dictionaryEntrySchema.parse({
    id: id(),
    term,
    heard_as: heard,
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: 1,
    updated_at_ms: 1,
  });
}

function rule(name: string, pattern = "p", kind: "literal" | "regex" = "literal"): ReplacementRule {
  return replacementRuleSchema.parse({
    id: id(),
    name,
    kind,
    pattern,
    replacement: "",
    case_sensitive: true,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
  });
}

function preset(name: string): CustomPreset {
  return { id: id(), name, prompt: `${name}的提示词`, created_at_ms: 1, updated_at_ms: 1 };
}

function scene(name: string, apps: string[] = ["app"], builtin?: Scene["builtin"]): Scene {
  return {
    id: id(),
    name: builtin ?? name,
    enabled: true,
    match: { apps, title_contains: [] },
    overrides: {},
    created_at_ms: 1,
    updated_at_ms: 1,
    ...(builtin === undefined ? {} : { builtin }),
  };
}

describe("backup merges of the in-memory backend", () => {
  it("the dictionary skips what is here, what is not valid, contradictions and what is over the limit", () => {
    const here = [entry("Voltip", ["沃提普"])];
    const sameId = { ...entry("同一条"), id: here[0]?.id ?? "" };
    const incoming = [sameId, entry("voltip"), entry(" "), entry("沃提普"), entry("Paseo")];
    const merged = mergeDictionary(here, incoming, "merge");
    expect(merged.list.map((e) => e.term)).toEqual(["Voltip", "Paseo"]);
    expect(merged.report).toMatchObject({
      in_file: 5,
      restored: 1,
      skipped: { duplicate: 2, invalid: 1, conflict: 1, limit: 0 },
    });
    const full = Array.from({ length: MAX_DICTIONARY_ENTRIES }, (_, i) => entry(`词${i}`));
    expect(mergeDictionary(full, [entry("新词")], "merge").report.skipped.limit).toBe(1);
    expect(mergeDictionary(here, incoming, "replace").list.map((e) => e.term)).toEqual([
      "同一条",
      "voltip",
      "沃提普",
      "Paseo",
    ]);
  });

  it("rules skip a name here, a regex that does not compile and what is over the limit", () => {
    const here = Array.from({ length: MAX_RULES - 1 }, (_, i) => rule(`r${i}`));
    const merged = mergeRules(
      here,
      [rule("r0"), rule("坏", "(", "regex"), rule("新"), rule("再")],
      "merge",
    );
    expect(merged.list).toHaveLength(MAX_RULES);
    expect(merged.report.skipped).toMatchObject({ duplicate: 1, invalid: 1, limit: 1 });
  });

  it("presets skip a name here and remember it for the scenes; past the limit is full", () => {
    const here = [preset("周报")];
    const theirs = [preset("周报"), preset("纪要")];
    const merged = mergePresets(here, theirs, "merge");
    expect(merged.list.map((p) => p.name)).toEqual(["周报", "纪要"]);
    expect([...merged.same]).toEqual([[theirs[0]?.id, here[0]?.id]]);
    const full = Array.from({ length: MAX_PRESETS }, (_, i) => preset(`预设${i}`));
    expect(mergePresets(full, [preset("再")], "merge").report.skipped.limit).toBe(1);
    expect(
      mergePresets(here, [{ ...preset("x"), name: " " }], "merge").report.skipped.invalid,
    ).toBe(1);
  });

  it("scenes keep the built-in ones here when merging and take them over in place when replacing", () => {
    const coding = scene("", [], "coding");
    const here = [scene("Slack"), coding];
    const weekly = { ...scene("Outlook"), overrides: { refine_preset: "theirs" } };
    const theirCoding = { ...scene("", ["code"], "coding"), enabled: false };
    const incoming = [
      scene("slack"),
      weekly,
      scene("无应用", []),
      theirCoding,
      scene("", [], "legal"),
    ];
    const same = new Map([["theirs", "mine"]]);
    const merged = mergeScenes(here, incoming, "merge", same);
    expect(merged.list.map((s) => s.name)).toEqual(["Slack", "Outlook", "coding", "legal"]);
    expect(merged.list[1]?.overrides.refine_preset).toBe("mine");
    expect(merged.report.skipped).toMatchObject({ duplicate: 2, invalid: 1 });
    const replaced = mergeScenes(here, incoming, "replace", new Map());
    const takenOver = replaced.list.find((s) => s.builtin === "coding");
    expect([takenOver?.id, takenOver?.enabled, takenOver?.match.apps]).toEqual([
      coding.id,
      false,
      ["code"],
    ]);
    expect(
      mergeScenes([], [coding, { ...coding, id: id() }], "replace", new Map()).report.skipped
        .conflict,
    ).toBe(1);
    const full = Array.from({ length: MAX_SCENES }, (_, i) => scene(`场景${i}`));
    expect(mergeScenes(full, [scene("再")], "merge", new Map()).report.skipped.limit).toBe(1);
  });

  it("the history keeps the newest of both, and replacing starts from the backup alone", () => {
    const now = 1_760_000_000_000;
    const at = (minute: number, mark: string): HistoryEntry => ({
      ...(sampleHistory(now)[0] as HistoryEntry),
      id: id(),
      text: mark,
      at_ms: now + minute * 60_000,
    });
    const here = [at(6, "6"), at(4, "4"), at(2, "2")];
    const incoming = [at(7, "7"), at(4, "新4"), at(1, "1"), here[0] as HistoryEntry];
    const merged = mergeHistory(here, incoming, "merge", 4);
    expect(merged.list.map((e) => e.text)).toEqual(["7", "6", "新4", "4"]);
    expect(merged.report).toMatchObject({
      restored: 2,
      removed: 1,
      skipped: { duplicate: 1, older: 1 },
    });
    const replaced = mergeHistory(here, incoming, "replace", 2);
    expect(replaced.list.map((e) => e.text)).toEqual(["7", "6"]);
    expect(replaced.report).toMatchObject({ restored: 2, removed: 3, skipped: { older: 2 } });
  });
});
