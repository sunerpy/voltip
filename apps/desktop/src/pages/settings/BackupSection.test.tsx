import { type BackupOpenOutcome } from "@voltip/shared";
import { MockBackend, type MockBackupFile, sampleBackup } from "@voltip/shared/mock";
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
