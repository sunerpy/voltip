// Backup and migration (docs/dictation.md section 4.6): the answers the Rust side writes
// (crates/voltip-tauri-bridge/tests/contract.rs, fixtures/ipc/backup.json) parse with the zod
// schemas, and TauriBackend sends the four commands with the arguments the desktop shell takes.
import { readFileSync } from "node:fs";
import { z } from "zod";

import {
  BACKUP_EXTENSION,
  BACKUP_PARTS,
  MAX_BACKUP_BYTES,
  backupExportOutcomeSchema,
  backupImportOutcomeSchema,
  backupOpenOutcomeSchema,
  backupPartsSchema,
} from "./schema";
import { TauriBackend } from "./tauri-backend";

const fixture = z
  .record(z.string(), z.unknown())
  .parse(JSON.parse(readFileSync(new URL("./fixtures/ipc/backup.json", import.meta.url), "utf8")));

describe("backup and migration (docs/dictation.md section 4.6)", () => {
  it("every answer the Rust side writes parses, with every code", () => {
    const exports = z.array(backupExportOutcomeSchema).parse(fixture.export);
    expect(exports.map((o) => (o.kind === "failed" ? o.code : o.kind))).toEqual([
      "saved",
      "cancelled",
      "write",
      "history",
      "unsupported",
    ]);
    const saved = exports[0];
    expect(saved?.kind === "saved" && saved.counts.history).toBe(20_000);
    const opens = z.array(backupOpenOutcomeSchema).parse(fixture.open);
    expect(opens.map((o) => (o.kind === "failed" ? o.code : o.kind))).toEqual([
      "preview",
      "cancelled",
      "unreadable",
      "not_backup",
      "damaged",
      "too_new",
      "too_large",
      "unsupported",
    ]);
    const preview = opens[0];
    expect(preview?.kind === "preview" && preview.manifest).toEqual({
      format: "voltip-backup",
      version: 1,
      app_version: "0.1.4",
      created_at_ms: 1_758_700_600_000,
      device: "Studio",
      platform: "windows",
      counts: { history: 20_000, dictionary: 12, rules: 3, scenes: 9, presets: 2 },
    });
    expect(opens.find((o) => o.kind === "failed" && o.code === "too_new")).toMatchObject({
      version: 2,
    });
    const imports = z.array(backupImportOutcomeSchema).parse(fixture.import);
    const done = imports[0];
    if (done?.kind !== "done") throw new Error("the first import is done");
    expect(done.report.parts.map((p) => p.part)).toEqual([
      "presets",
      "scenes",
      "dictionary",
      "rules",
      "history",
    ]);
    expect(done.report.parts.find((p) => p.part === "rules")?.error).toBe("write");
    expect(done.report.parts.find((p) => p.part === "history")).toMatchObject({
      restored: 19_000,
      removed: 5,
      skipped: { duplicate: 10, older: 990 },
    });
    expect(imports.slice(2).map((o) => o.kind === "failed" && o.code)).toEqual([
      "expired",
      "unavailable",
      "unsupported",
    ]);
    expect(backupPartsSchema.parse(fixture.parts)).toEqual({
      history: true,
      dictionary: true,
      rules: false,
      scenes: true,
      presets: false,
    });
    // A code the Rust side does not have is refused rather than shown.
    expect(
      backupOpenOutcomeSchema.safeParse({ kind: "failed", code: "gone", detail: "" }).success,
    ).toBe(false);
    expect([BACKUP_PARTS.length, BACKUP_EXTENSION, MAX_BACKUP_BYTES]).toEqual([
      5,
      "voltip-backup",
      512 * 1024 * 1024,
    ]);
  });

  it("TauriBackend sends the four commands with the arguments the shell takes", async () => {
    const answers: Record<string, unknown> = {
      backup_export: (fixture.export as unknown[])[0],
      backup_open: (fixture.open as unknown[])[0],
      backup_import: (fixture.import as unknown[])[0],
      backup_discard: null,
    };
    const calls: { command: string; args: unknown }[] = [];
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answers[command]);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    expect((await backend.backupExport("Voltip 备份 2026-10-13")).kind).toBe("saved");
    const opened = await backend.backupOpen("选择要导入的备份");
    if (opened.kind !== "preview") throw new Error("a preview");
    const parts = backupPartsSchema.parse(fixture.parts);
    expect((await backend.backupImport(opened.token, parts, "replace")).kind).toBe("done");
    await backend.backupDiscard(opened.token);
    expect(calls).toStrictEqual([
      { command: "backup_export", args: { fileName: "Voltip 备份 2026-10-13" } },
      { command: "backup_open", args: { title: "选择要导入的备份" } },
      { command: "backup_import", args: { token: opened.token, parts, mode: "replace" } },
      { command: "backup_discard", args: { token: opened.token } },
    ]);
  });
});
