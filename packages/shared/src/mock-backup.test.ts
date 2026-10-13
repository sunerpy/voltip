// The in-memory backend's backup and migration (docs/dictation.md section 4.6): what the UI
// tests and the browser preview run on, with the core's merge rules.
import { type MockBackupFile } from "./backup-merge";
import { MockBackend, sampleBackup } from "./mock-backend";
import { type BackupParts, type UiEvent } from "./schema";

const NOW = 1_760_000_000_000;
const ALL: BackupParts = {
  history: true,
  dictionary: true,
  rules: true,
  scenes: true,
  presets: true,
};

async function openPreview(backend: MockBackend) {
  const opened = await backend.backupOpen("选择备份");
  if (opened.kind !== "preview") throw new Error(`no preview: ${JSON.stringify(opened)}`);
  return opened;
}

describe("MockBackend backups (docs/dictation.md section 4.6)", () => {
  it("exports every part as the state holds it, and opens the last export", async () => {
    const backend = new MockBackend({ now: () => NOW });
    await backend.invoke("dictionary_add", {
      entry: { term: "Voltip", heard_as: ["沃提普"], enabled: true },
    });
    const state = backend.peek();
    const saved = await backend.backupExport("Voltip 备份 2025-10-09");
    expect(saved).toEqual({
      kind: "saved",
      path: "/home/you/Documents/Voltip 备份 2025-10-09.voltip-backup",
      counts: {
        history: state.history_total,
        dictionary: 1,
        rules: 0,
        scenes: state.scenes.length,
        presets: 0,
      },
      bytes: expect.any(Number) as number,
    });
    expect(backend.backupExports).toEqual(["Voltip 备份 2025-10-09"]);
    const preview = await openPreview(backend);
    expect(preview.manifest).toMatchObject({
      format: "voltip-backup",
      version: 1,
      device: state.identity?.name,
      platform: "windows",
      created_at_ms: NOW,
    });
    // Everything is here already: nothing is restored, no list is sent again.
    const events = backend.log.length;
    const answer = await backend.backupImport(preview.token, ALL, "merge");
    if (answer.kind !== "done") throw new Error("imported");
    expect(answer.report.parts.map((p) => p.restored)).toEqual([0, 0, 0, 0, 0]);
    expect(answer.report.parts.find((p) => p.part === "history")?.skipped.duplicate).toBe(
      state.history_total,
    );
    expect(backend.log.length).toBe(events);
  });

  it("restores another computer's backup merged, keeping the newest entries the setting allows", async () => {
    const backend = new MockBackend({
      now: () => NOW,
      history: [],
      settings: { history: { enabled: true, keep: 10 } },
      presets: [
        { id: "p-here", name: "周报", prompt: "我的周报", created_at_ms: 1, updated_at_ms: 1 },
      ],
    });
    // Fifteen entries a minute apart, newest first: more than the ten the setting keeps.
    const sample = sampleBackup(NOW);
    const history = Array.from({ length: 15 }, (_, i) => ({
      ...(sample.history[i % sample.history.length] as (typeof sample.history)[number]),
      id: `00000000-0000-4000-9000-1${String(i).padStart(11, "0")}`,
      at_ms: NOW - (i + 1) * 60_000,
    }));
    const file: MockBackupFile = {
      ...sample,
      history,
      manifest: { ...sample.manifest, counts: { ...sample.manifest.counts, history: 15 } },
    };
    backend.simulateBackupFile(file);
    const seen: UiEvent["type"][] = [];
    backend.on((e) => seen.push(e.type));
    const preview = await openPreview(backend);
    expect(preview.manifest.device).toBe("Office-PC");
    expect(backend.peek().dictionary).toEqual([]);
    const answer = await backend.backupImport(preview.token, ALL, "merge");
    if (answer.kind !== "done") throw new Error("imported");
    const part = (name: string) => answer.report.parts.find((p) => p.part === name);
    expect(part("history")).toMatchObject({ in_file: 15, restored: 10, skipped: { older: 5 } });
    expect((await backend.historyQuery({ limit: 200 })).entries.map((e) => e.id)).toEqual(
      history.slice(0, 10).map((e) => e.id),
    );
    expect(part("presets")).toMatchObject({ restored: 0, skipped: { duplicate: 1 } });
    expect(seen).toEqual(["scenes", "dictionary", "rules", "history"]);
    const state = backend.peek();
    expect(state.history_total).toBe(10);
    expect(state.dictionary.map((e) => e.term)).toEqual(["Kubernetes"]);
    expect(state.rules.map((r) => r.name)).toEqual(["句号"]);
    const outlook = state.scenes.find((s) => s.name === "Outlook");
    expect(outlook?.overrides.refine_preset).toBe("p-here");
    expect(state.scenes.indexOf(outlook as never)).toBeLessThan(
      state.scenes.findIndex((s) => s.builtin !== undefined),
    );
    // The statistics follow from the restored entries.
    const stats = await backend.historyStats([NOW - 7 * 86_400_000, NOW + 86_400_000]);
    const dictations = (await backend.historyQuery({ limit: 200 })).entries.filter(
      (e) => e.kind === "dictation",
    ).length;
    expect(stats.total.count).toBe(dictations);
  });

  it("replaces only the chosen parts with the backup's", async () => {
    const backend = new MockBackend({ now: () => NOW });
    await backend.invoke("dictionary_add", {
      entry: { term: "Voltip", heard_as: [], enabled: true },
    });
    const before = backend.peek();
    backend.simulateBackupFile(sampleBackup(NOW));
    const preview = await openPreview(backend);
    const parts = { ...ALL, history: false, scenes: false, presets: false, rules: false };
    const answer = await backend.backupImport(preview.token, parts, "replace");
    expect(answer.kind === "done" && answer.report.parts.map((p) => p.part)).toEqual([
      "dictionary",
    ]);
    const after = backend.peek();
    expect(after.dictionary.map((e) => e.term)).toEqual(["Kubernetes"]);
    expect(after.history_total).toBe(before.history_total);
  });

  it("a token works once, a closed preview is let go, refusals come back as they are, and the phone has no backups", async () => {
    const backend = new MockBackend({ now: () => NOW });
    const preview = await openPreview(backend);
    await backend.backupDiscard(preview.token);
    expect(await backend.backupImport(preview.token, ALL, "merge")).toEqual({
      kind: "failed",
      code: "expired",
    });
    const again = await openPreview(backend);
    expect((await backend.backupImport(again.token, ALL, "merge")).kind).toBe("done");
    expect((await backend.backupImport(again.token, ALL, "merge")).kind).toBe("failed");
    backend.simulateBackupFile({ kind: "failed", code: "too_new", detail: "", version: 2 });
    expect(await backend.backupOpen("x")).toEqual({
      kind: "failed",
      code: "too_new",
      detail: "",
      version: 2,
    });
    backend.simulateBackupFile({ kind: "cancelled" });
    expect(await backend.backupOpen("x")).toEqual({ kind: "cancelled" });
    const phone = new MockBackend({ role: "phone", now: () => NOW });
    expect((await phone.backupExport("x")).kind).toBe("failed");
    expect(await phone.backupOpen("x")).toMatchObject({ code: "unsupported" });
    expect(await phone.backupImport("t", ALL, "merge")).toEqual({
      kind: "failed",
      code: "unsupported",
    });
  });

  it("the sample backup is one the core's checks accept", () => {
    const file: MockBackupFile = sampleBackup(NOW);
    expect(file.manifest.counts).toEqual({
      history: file.history.length,
      dictionary: 1,
      rules: 1,
      scenes: 1,
      presets: 1,
    });
    expect(new Set(file.history.map((e) => e.id)).size).toBe(file.history.length);
    expect(file.history.every((e) => e.at_ms < NOW)).toBe(true);
  });
});
