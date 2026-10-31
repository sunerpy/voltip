import { type BackupOpenOutcome, formatDateTime } from "@voltip/shared";
import {
  MOCK_BACKUPS_DIR,
  MOCK_CHOSEN_BACKUPS_DIR,
  MockBackend,
  type MockBackupFile,
  sampleBackup,
} from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

const NOW = 1_758_700_000_000;

/** Another computer's backup with fifteen entries a minute apart, newest first. */
function backupOfFifteen(): MockBackupFile {
  const sample = sampleBackup(NOW);
  const history = Array.from({ length: 15 }, (_, i) => ({
    ...(sample.history[i % sample.history.length] as MockBackupFile["history"][number]),
    id: `00000000-0000-4000-9000-1${String(i).padStart(11, "0")}`,
    at_ms: NOW - (i + 1) * 60_000,
  }));
  return {
    ...sample,
    history,
    manifest: { ...sample.manifest, counts: { ...sample.manifest.counts, history: 15 } },
  };
}

async function openImport(user: ReturnType<typeof userEvent.setup>) {
  const section = await screen.findByTestId("privacy-backup");
  await user.click(within(section).getByRole("button", { name: "导入…" }));
  return screen.findByRole("dialog", { name: "导入备份" });
}

describe("Settings · 隐私与历史 · 备份与迁移 (docs/dictation.md section 4.6)", () => {
  it("导出备份 hands the shell's save dialog a dated name and says where the file went", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/privacy" });
    const section = await screen.findByTestId("privacy-backup");
    expect(section).toHaveTextContent("备份与迁移");
    expect(section).toHaveTextContent("备份不包含设置、API 密钥和已配对的设备");
    expect(section).toHaveTextContent("备份文件未加密");
    await user.click(within(section).getByRole("button", { name: "导出…" }));
    await waitFor(() => {
      expect(backend.backupExports).toHaveLength(1);
    });
    expect(backend.backupExports[0]).toMatch(/^Voltip 备份 \d{4}-\d{2}-\d{2}$/);
    expect(
      await screen.findByText(
        /^已导出备份（\d+(\.\d)? [KM]B）：\/home\/you\/Documents\/Voltip 备份 \d{4}-\d{2}-\d{2}\.voltip-backup$/,
      ),
    ).toBeInTheDocument();
  });

  it("导入备份 shows what the file holds before anything changes, then replaces only the parts chosen", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW, history: [] });
    backend.simulateBackupFile(sampleBackup(NOW));
    renderApp({ path: "/settings/privacy", backend });
    const dialog = await openImport(user);
    expect(dialog).toHaveTextContent("Office-PC · Windows");
    expect(dialog).toHaveTextContent("Voltip 0.0.1 · 格式版本 1");
    const counts = sampleBackup(NOW).manifest.counts;
    expect(within(dialog).getByTestId("backup-part-history")).toHaveTextContent(
      `${counts.history} 条`,
    );
    expect(within(dialog).getByTestId("backup-part-scenes")).toHaveTextContent("1 个");
    for (const name of ["历史记录", "词典", "规则", "场景", "AI 预设"]) {
      expect(within(dialog).getByRole("switch", { name })).toHaveAttribute("aria-checked", "true");
    }
    expect(backend.peek().dictionary).toEqual([]);
    expect(backend.peek().history_total).toBe(0);
    await user.click(within(dialog).getByRole("switch", { name: "历史记录" }));
    await user.click(within(dialog).getByRole("radio", { name: "替换" }));
    expect(dialog).toHaveTextContent("删除这台电脑上所选部分的现有内容，替换为备份中的内容。");
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    const done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).getByTestId("backup-result-dictionary")).toHaveTextContent("已导入 1 条");
    expect(within(done).queryByTestId("backup-result-history")).toBeNull();
    const state = backend.peek();
    expect(state.dictionary.map((e) => e.term)).toEqual(["Kubernetes"]);
    expect(state.rules.map((r) => r.name)).toEqual(["句号"]);
    expect(state.history_total).toBe(0);
    await user.click(within(done).getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog", { name: "导入完成" })).toBeNull();
  });

  it("the result says what each part restored, what it skipped and why, and the home page counts it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      now: () => NOW,
      history: [],
      settings: { history: { enabled: true, keep: 10 } },
    });
    await backend.invoke("dictionary_add", {
      entry: { term: "kubernetes", heard_as: [], enabled: true },
    });
    backend.simulateBackupFile(backupOfFifteen());
    renderApp({ path: "/settings/privacy", backend });
    const dialog = await openImport(user);
    expect(dialog).toHaveTextContent("历史记录最多保留最近 10 条，超出部分中较早的记录不会保留。");
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    const done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).getByTestId("backup-result-history")).toHaveTextContent(
      "已导入 10 条跳过：超出保留条数 5 条",
    );
    expect(within(done).getByTestId("backup-result-dictionary")).toHaveTextContent(
      "已导入 0 条跳过：已有 1 条",
    );
    expect(backend.peek().history_total).toBe(10);
    const stats = await backend.historyStats([NOW - 86_400_000, NOW + 86_400_000]);
    expect(stats.total.count).toBe(10);
  });

  it("a file that cannot be imported is named in a sentence, and nothing opens", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    renderApp({ path: "/settings/privacy", backend });
    const section = await screen.findByTestId("privacy-backup");
    const cases: [BackupOpenOutcome, string][] = [
      [{ kind: "failed", code: "not_backup", detail: "" }, "所选文件不是 Voltip 备份。"],
      [
        { kind: "failed", code: "too_new", detail: "", version: 2 },
        "此备份由较新版本的 Voltip 创建（格式版本 2），请先更新 Voltip。",
      ],
      [{ kind: "failed", code: "damaged", detail: "EOF at line 8" }, "备份文件已损坏，无法导入。"],
      [{ kind: "failed", code: "too_large", detail: "" }, "备份文件超过 512 MB，无法导入。"],
      [{ kind: "failed", code: "unreadable", detail: "denied" }, "无法读取所选文件。"],
    ];
    for (const [outcome, sentence] of cases) {
      backend.simulateBackupFile(outcome);
      await user.click(within(section).getByRole("button", { name: "导入…" }));
      expect(await screen.findByText(sentence)).toBeInTheDocument();
    }
    backend.simulateBackupFile({ kind: "cancelled" });
    await user.click(within(section).getByRole("button", { name: "导入…" }));
    expect(screen.queryByRole("dialog", { name: "导入备份" })).toBeNull();
  });

  it("closing the preview lets go of the backup, and a part the backup lacks cannot be chosen", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    const sample = sampleBackup(NOW);
    backend.simulateBackupFile({
      ...sample,
      rules: [],
      manifest: { ...sample.manifest, counts: { ...sample.manifest.counts, rules: 0 } },
    });
    const discard = vi.spyOn(backend, "backupDiscard");
    renderApp({ path: "/settings/privacy", backend });
    const dialog = await openImport(user);
    const rules = within(dialog).getByRole("switch", { name: "规则" });
    expect(rules).toBeDisabled();
    expect(rules).toHaveAttribute("aria-checked", "false");
    expect(within(dialog).getByTestId("backup-part-rules")).toHaveTextContent("备份中没有");
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog", { name: "导入备份" })).toBeNull();
    expect(discard).toHaveBeenCalledTimes(1);
    expect(backend.peek().dictionary).toEqual([]);
  });

  it("a replaced history says how many entries went, and a part that could not be written says so", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    const before = backend.peek().history_total;
    expect(before).toBeGreaterThan(0);
    backend.simulateBackupFile(sampleBackup(NOW));
    renderApp({ path: "/settings/privacy", backend });
    let dialog = await openImport(user);
    for (const name of ["词典", "规则", "场景", "AI 预设"]) {
      await user.click(within(dialog).getByRole("switch", { name }));
    }
    await user.click(within(dialog).getByRole("radio", { name: "替换" }));
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    let done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).getByTestId("backup-result-history")).toHaveTextContent(
      `已从这台电脑删除 ${before} 条`,
    );
    await user.click(within(done).getByRole("button", { name: "完成" }));

    const zero = { duplicate: 0, limit: 0, conflict: 0, invalid: 0, older: 0 };
    vi.spyOn(backend, "backupImport").mockResolvedValue({
      kind: "done",
      report: {
        mode: "merge",
        parts: [
          { part: "rules", in_file: 1, restored: 0, skipped: zero, removed: 0, error: "write" },
          {
            part: "history",
            in_file: 6,
            restored: 0,
            skipped: zero,
            removed: 0,
            error: "unavailable",
          },
        ],
      },
    });
    dialog = await openImport(user);
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).getByTestId("backup-result-history")).toHaveTextContent(
      "无法打开历史记录，未导入。",
    );
    expect(within(done).getByTestId("backup-result-rules")).toHaveTextContent(
      "未能保存，此项保持不变。",
    );
    // History first, as the interface lists the parts.
    expect(
      within(done)
        .getAllByRole("listitem")
        .map((li) => li.dataset.testid),
    ).toEqual(["backup-result-history", "backup-result-rules"]);
  });

  it("an import without an answer says so and keeps the preview open", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    const sample = sampleBackup(NOW);
    backend.simulateBackupFile({ ...sample, manifest: { ...sample.manifest, platform: "other" } });
    const imported = vi
      .spyOn(backend, "backupImport")
      .mockResolvedValueOnce({ kind: "failed", code: "unavailable" })
      .mockRejectedValueOnce(new Error("backup: 连接已断开"));
    renderApp({ path: "/settings/privacy", backend });
    const dialog = await openImport(user);
    expect(dialog).toHaveTextContent("来源Office-PC创建时间");
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "导入未完成：Voltip 没有响应，请稍后重试。",
    );
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    await waitFor(() => {
      expect(within(dialog).getByRole("alert")).toHaveTextContent("连接已断开");
    });
    expect(imported).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("dialog", { name: "导入备份" })).toBeInTheDocument();
  });

  it("an export that could not be written or read says why; a closed dialog says nothing", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    const exported = vi
      .spyOn(backend, "backupExport")
      .mockResolvedValueOnce({ kind: "failed", code: "write", detail: "No space left on device" })
      .mockResolvedValueOnce({ kind: "failed", code: "history", detail: "database is locked" })
      .mockResolvedValueOnce({ kind: "cancelled" })
      .mockRejectedValueOnce(new Error("backup: 连接已断开"));
    const opened = vi
      .spyOn(backend, "backupOpen")
      .mockRejectedValueOnce(new Error("对话框无法打开"));
    renderApp({ path: "/settings/privacy", backend });
    const section = await screen.findByTestId("privacy-backup");
    const button = within(section).getByRole("button", { name: "导出…" });
    await user.click(button);
    expect(await screen.findByText("无法在所选位置保存备份。")).toBeInTheDocument();
    await user.click(button);
    expect(await screen.findByText("无法读取历史记录，未导出备份。")).toBeInTheDocument();
    await user.click(button);
    await waitFor(() => {
      expect(exported).toHaveBeenCalledTimes(3);
    });
    await user.click(button);
    expect(await screen.findByText(/连接已断开/)).toBeInTheDocument();
    await user.click(within(section).getByRole("button", { name: "导入…" }));
    expect(await screen.findByText(/对话框无法打开/)).toBeInTheDocument();
    expect(opened).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/已导出备份/)).toBeNull();
  });

  it("in English", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW, history: [] });
    backend.simulateBackupFile(sampleBackup(NOW));
    renderApp({ path: "/settings/privacy", backend, systemLanguage: "en-US" });
    const section = await screen.findByTestId("privacy-backup");
    expect(section).toHaveTextContent("Backup and migration");
    await user.click(within(section).getByRole("button", { name: "Import…" }));
    const dialog = await screen.findByRole("dialog", { name: "Import a backup" });
    expect(within(dialog).getByTestId("backup-part-history")).toHaveTextContent("6 entries");
    await user.click(within(dialog).getByRole("button", { name: "Import" }));
    const done = await screen.findByRole("dialog", { name: "Import complete" });
    expect(within(done).getByTestId("backup-result-rules")).toHaveTextContent("1 rule imported");
  });
});

/** The time the 自动备份 rows show, as the page formats it. */
function shown(atMs: number, locale: "zh-CN" | "en" = "zh-CN"): string {
  return formatDateTime(locale, atMs, { dateStyle: "medium", timeStyle: "short" });
}

describe("Settings · 隐私与历史 · 自动备份 (docs/dictation.md section 4.6.1)", () => {
  it("shows the weekly switch, where the backups go and the latest one, and what else is backed up", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      now: () => NOW,
      autoBackup: {
        last: {
          name: "voltip-auto-20250924-154640-weekly-a7c4198e.voltip-backup",
          at_ms: NOW,
          kind: "weekly",
        },
      },
    });
    renderApp({ path: "/settings/privacy", backend });
    const section = await screen.findByTestId("privacy-auto-backup");
    expect(section).toHaveTextContent("自动备份");
    expect(section).toHaveTextContent("更新安装前和替换导入前也会自动备份一份");
    expect(section).toHaveTextContent("每台电脑只保留自己最近 5 份自动备份");
    expect(section).toHaveTextContent("备份文件未加密，放在同步盘中会随同步上传");
    // User decision D23 (2026-10-30): what a synced folder is for, and that each computer manages
    // its own backups there, named by this computer's tag.
    const sync = within(section).getByTestId("auto-backup-sync");
    expect(sync).toHaveTextContent("即使这台电脑损坏、丢失或重装系统，备份仍会保留");
    expect(sync).toHaveTextContent("Voltip 不会在电脑之间同步数据，换电脑时请在新电脑上导入备份");
    expect(sync).toHaveTextContent(
      "多台电脑可以共用同一个文件夹，每台电脑只管理文件名中带有自己标记的自动备份，不会改动其他电脑的备份",
    );
    expect(sync).toHaveTextContent("这台电脑的标记：a7c4198e");
    expect(within(section).getByTestId("auto-backup-folder")).toHaveTextContent(MOCK_BACKUPS_DIR);
    expect(section).toHaveTextContent("默认");
    expect(within(section).queryByRole("button", { name: "恢复默认位置" })).toBeNull();
    expect(within(section).getByTestId("auto-backup-last")).toHaveTextContent(
      `${shown(NOW)}（每周）`,
    );
    const weekly = within(section).getByRole("switch", { name: "每周自动备份" });
    expect(weekly).toHaveAttribute("aria-checked", "true");
    await user.click(weekly);
    await waitFor(() => {
      expect(backend.peek().settings.backup.weekly).toBe(false);
    });
    expect(weekly).toHaveAttribute("aria-checked", "false");
    await user.click(within(section).getByRole("button", { name: "打开文件夹" }));
    expect(backend.backupFoldersOpened).toEqual([MOCK_BACKUPS_DIR]);
  });

  it("更改… moves the backups to a folder that can be written; one that cannot is refused with why", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    renderApp({ path: "/settings/privacy", backend });
    const section = await screen.findByTestId("privacy-auto-backup");
    expect(within(section).getByTestId("auto-backup-last")).toHaveTextContent("尚未进行自动备份");
    await user.click(within(section).getByRole("button", { name: "更改…" }));
    expect(
      await screen.findByText(`自动备份将保存到：${MOCK_CHOSEN_BACKUPS_DIR}`),
    ).toBeInTheDocument();
    expect(within(section).getByTestId("auto-backup-folder")).toHaveTextContent(
      MOCK_CHOSEN_BACKUPS_DIR,
    );
    expect(section).toHaveTextContent("自定义");
    expect(backend.peek().settings.backup.folder).toBe(MOCK_CHOSEN_BACKUPS_DIR);

    backend.simulateBackupFolder({ unwritable: "Permission denied (os error 13)" });
    await user.click(within(section).getByRole("button", { name: "更改…" }));
    expect(
      await screen.findByText("无法写入所选文件夹，位置未更改：Permission denied (os error 13)"),
    ).toBeInTheDocument();
    expect(backend.peek().settings.backup.folder).toBe(MOCK_CHOSEN_BACKUPS_DIR);

    backend.simulateBackupFolder(null);
    await user.click(within(section).getByRole("button", { name: "更改…" }));
    expect(backend.peek().settings.backup.folder).toBe(MOCK_CHOSEN_BACKUPS_DIR);

    await user.click(within(section).getByRole("button", { name: "恢复默认位置" }));
    await waitFor(() => {
      expect(within(section).getByTestId("auto-backup-folder")).toHaveTextContent(MOCK_BACKUPS_DIR);
    });
    expect(backend.peek().settings.backup.folder).toBeNull();
    expect(within(section).queryByRole("button", { name: "恢复默认位置" })).toBeNull();
  });

  it("a failed automatic backup is shown until one is written", async () => {
    const backend = new MockBackend({
      now: () => NOW,
      autoBackup: {
        failure: { kind: "weekly", at_ms: NOW, detail: "No space left on device (os error 28)" },
      },
    });
    renderApp({ path: "/settings/privacy", backend });
    const failure = await screen.findByTestId("auto-backup-failure");
    expect(failure).toHaveTextContent(
      `自动备份失败（每周，${shown(NOW)}）：No space left on device (os error 28)`,
    );
    expect(failure).toHaveTextContent("将在 24 小时后重试。");
    backend.simulateWeeklyBackup();
    await waitFor(() => {
      expect(screen.queryByTestId("auto-backup-failure")).toBeNull();
    });
    expect(screen.getByTestId("auto-backup-last")).toHaveTextContent(`${shown(NOW)}（每周）`);
  });

  it("a replacing import backs up what is here first and names the file; a merge does not", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    backend.simulateBackupFile(sampleBackup(NOW));
    renderApp({ path: "/settings/privacy", backend });
    let dialog = await openImport(user);
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    let done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).queryByTestId("backup-snapshot")).toBeNull();
    expect(backend.autoBackups).toEqual([]);
    await user.click(within(done).getByRole("button", { name: "完成" }));

    backend.simulateBackupFile(sampleBackup(NOW));
    dialog = await openImport(user);
    await user.click(within(dialog).getByRole("radio", { name: "替换" }));
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    done = await screen.findByRole("dialog", { name: "导入完成" });
    const [snapshot] = backend.autoBackups;
    expect(snapshot?.kind).toBe("import");
    expect(snapshot?.name).toMatch(/^voltip-auto-\d{8}-\d{6}-import-a7c4198e\.voltip-backup$/);
    expect(within(done).getByTestId("backup-snapshot")).toHaveTextContent(
      `替换前的内容已自动备份到：${MOCK_BACKUPS_DIR}/${snapshot?.name ?? ""}`,
    );
    await user.click(within(done).getByRole("button", { name: "完成" }));
    expect(screen.getByTestId("auto-backup-last")).toHaveTextContent(`${shown(NOW)}（替换导入前）`);
  });

  it("a replacing import is refused during a dictation or when its backup fails, and can be tried again", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => NOW });
    backend.simulateBackupFile(sampleBackup(NOW));
    renderApp({ path: "/settings/privacy", backend });
    const before = backend.peek().history_total;
    const dialog = await openImport(user);
    await user.click(within(dialog).getByRole("radio", { name: "替换" }));
    await backend.invoke("dictation_start");
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "正在听写，请在听写结束后再替换导入。",
    );
    await backend.invoke("dictation_cancel");
    backend.simulateAutoBackupFails("No space left on device (os error 28)");
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    await waitFor(() => {
      expect(within(dialog).getByRole("alert")).toHaveTextContent(
        "替换前未能先备份现有数据，未导入任何内容：No space left on device (os error 28)",
      );
    });
    expect(backend.peek().history_total).toBe(before);
    expect(backend.peek().dictionary).toEqual([]);
    backend.simulateAutoBackupFails(undefined);
    await user.click(within(dialog).getByRole("button", { name: "替换" }));
    const done = await screen.findByRole("dialog", { name: "导入完成" });
    expect(within(done).getByTestId("backup-snapshot")).toBeInTheDocument();
    expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Kubernetes"]);
  });

  it("in English", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      now: () => NOW,
      autoBackup: {
        last: {
          name: "voltip-auto-20250924-154640-update-a7c4198e.voltip-backup",
          at_ms: NOW,
          kind: "update",
        },
        failure: { kind: "import", at_ms: NOW, detail: "Access is denied. (os error 5)" },
      },
    });
    renderApp({ path: "/settings/privacy", backend, systemLanguage: "en-US" });
    const section = await screen.findByTestId("privacy-auto-backup");
    expect(section).toHaveTextContent("Automatic backups");
    expect(section).toHaveTextContent(
      "Each computer keeps only its own latest 5 automatic backups",
    );
    const sync = within(section).getByTestId("auto-backup-sync");
    expect(sync).toHaveTextContent(
      "In a synced folder, the backups are kept even if this computer breaks, is lost or is reinstalled.",
    );
    expect(sync).toHaveTextContent(
      "Voltip does not sync data between computers: on a new computer, import a backup.",
    );
    expect(sync).toHaveTextContent(
      "Several computers can share one folder; each manages only the automatic backups whose names carry its own tag and leaves the other computers' backups alone.",
    );
    expect(sync).toHaveTextContent("This computer's tag: a7c4198e");
    expect(within(section).getByRole("switch", { name: "Back up every week" })).toBeInTheDocument();
    expect(within(section).getByTestId("auto-backup-last")).toHaveTextContent(
      `${shown(NOW, "en")} (before an update)`,
    );
    expect(within(section).getByTestId("auto-backup-failure")).toHaveTextContent(
      `Automatic backup failed (before a replacing import, ${shown(NOW, "en")}): Access is denied. (os error 5)`,
    );
    expect(within(section).getByTestId("auto-backup-failure")).not.toHaveTextContent("24 hours");
    await user.click(within(section).getByRole("button", { name: "Change…" }));
    expect(
      await screen.findByText(`Automatic backups will be saved to: ${MOCK_CHOSEN_BACKUPS_DIR}`),
    ).toBeInTheDocument();
  });
});
