import {
  AUTO_BACKUP_KEEP,
  type AutoBackupKind,
  BACKUP_PARTS,
  type BackupManifest,
  type BackupPart,
  type BackupPartReport,
  type BackupParts,
  type BackupImportReport,
  type ImportMode,
  type TFunction,
  attachmentSize,
  errorText,
  formatCount,
  formatDateTime,
  platformLabel,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Dialog,
  Icon,
  Segmented,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useShell } from "../../app/shell-context";

const TOAST_MS = 5000;

/** `7` → `07`. */
function twoDigits(n: number): string {
  return String(n).padStart(2, "0");
}

/** `Voltip 备份 2026-10-13`: the name the save dialog offers, dated today in local time. */
export function backupFileName(t: TFunction, nowMs: number): string {
  const d = new Date(nowMs);
  return t("settings.brief.privacy.backup.fileName", {
    date: `${d.getFullYear()}-${twoDigits(d.getMonth() + 1)}-${twoDigits(d.getDate())}`,
  });
}

/** `12 条` / `12 entries` of `part`, with a thousands separator. */
function countOf(t: TFunction, part: BackupPart, n: number): string {
  return t(`settings.brief.privacy.backup.count.${part}`, { n, count: formatCount(n) });
}

/** Settings · 隐私与历史 · 备份与迁移 (docs/dictation.md §4.6): 导出备份 through the shell's save
 *  dialog, and 导入备份 through its open dialog, a preview of what the file holds, the parts and
 *  the mode chosen, then what each part restored; then the automatic backups (§4.6.1). */
export function BackupSection() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [opened, setOpened] = useState<{ token: string; manifest: BackupManifest } | undefined>(
    undefined,
  );
  const fail = (message: string) => {
    shell.toast({ message, duration: TOAST_MS, tone: "danger" });
  };
  const exportBackup = async () => {
    setBusy(true);
    try {
      const outcome = await backend.backupExport(backupFileName(t, Date.now()));
      if (outcome.kind === "saved") {
        shell.toast({
          message: t("settings.brief.privacy.backup.exported", {
            size: attachmentSize(outcome.bytes),
            path: outcome.path,
          }),
          duration: TOAST_MS,
        });
      } else if (outcome.kind === "failed") {
        fail(t(`settings.brief.privacy.backup.exportFailed.${outcome.code}`));
      }
    } catch (e) {
      fail(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const openBackup = async () => {
    setBusy(true);
    try {
      const outcome = await backend.backupOpen(t("settings.brief.privacy.backup.openTitle"));
      if (outcome.kind === "preview") setOpened(outcome);
      else if (outcome.kind === "failed") {
        fail(
          t(`settings.brief.privacy.backup.openFailed.${outcome.code}`, {
            version: outcome.version ?? "",
          }),
        );
      }
    } catch (e) {
      fail(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <SettingsSection
        title={t("settings.brief.privacy.backup.title")}
        description={t("settings.brief.privacy.backup.note")}
        data-testid="privacy-backup">
        <SettingsRows>
          <StatusRow
            label={t("settings.brief.privacy.backup.export")}
            help={t("settings.brief.privacy.backup.exportHelp")}>
            <Button
              size="sm"
              variant="outline"
              icon="download"
              disabled={busy}
              onClick={() => {
                void exportBackup();
              }}>
              {t("settings.brief.privacy.backup.exportAction")}
            </Button>
          </StatusRow>
          <StatusRow
            label={t("settings.brief.privacy.backup.import")}
            help={t("settings.brief.privacy.backup.importHelp")}>
            <Button
              size="sm"
              variant="outline"
              icon="upload"
              disabled={busy}
              onClick={() => {
                void openBackup();
              }}>
              {t("settings.brief.privacy.backup.importAction")}
            </Button>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>
      <AutoBackupSection />
      {opened !== undefined && (
        <BackupImportDialog
          token={opened.token}
          manifest={opened.manifest}
          onClose={() => {
            setOpened(undefined);
          }}
        />
      )}
    </>
  );
}

/** Settings · 隐私与历史 · 自动备份 (docs/dictation.md §4.6.1): the weekly switch, where the
 *  automatic backups go (更改… through the shell's folder dialog and a trial file, 打开文件夹,
 *  恢复默认位置) and what a synced folder there does (user decision D23: each computer manages its
 *  own backups, named by its tag), this computer's newest one, and the last failure until one is
 *  written. The backups before an update and before a replacing import have no switch. The webview
 *  never names a folder. Nothing where the app makes no automatic backups (the phone). */
function AutoBackupSection() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const [choosing, setChoosing] = useState(false);
  const auto = state.auto_backup;
  if (!auto.available) return null;
  const kindOf = (kind: AutoBackupKind) => t(`settings.brief.privacy.backup.auto.kind.${kind}`);
  const timeOf = (ms: number) =>
    formatDateTime(locale, ms, { dateStyle: "medium", timeStyle: "short" });
  const fail = (message: string) => {
    shell.toast({ message, duration: TOAST_MS, tone: "danger" });
  };
  const choose = async () => {
    setChoosing(true);
    try {
      const outcome = await backend.backupFolderChoose(
        t("settings.brief.privacy.backup.auto.chooseTitle"),
      );
      if (outcome.kind === "chosen") {
        shell.toast({
          message: t("settings.brief.privacy.backup.auto.chosen", { folder: outcome.folder }),
          duration: TOAST_MS,
        });
      } else if (outcome.kind === "failed") {
        fail(t("settings.brief.privacy.backup.auto.unwritable", { detail: outcome.detail }));
      }
    } catch (e) {
      fail(errorText(e));
    } finally {
      setChoosing(false);
    }
  };
  const failure = auto.failure;
  return (
    <SettingsSection
      title={t("settings.brief.privacy.backup.auto.title")}
      description={t("settings.brief.privacy.backup.auto.note", { keep: AUTO_BACKUP_KEEP })}
      data-testid="privacy-auto-backup">
      <SettingsRows>
        <StatusRow
          label={t("settings.brief.privacy.backup.auto.weekly")}
          help={t("settings.brief.privacy.backup.auto.weeklyHelp")}>
          <Toggle
            checked={state.settings.backup.weekly}
            ariaLabel={t("settings.brief.privacy.backup.auto.weekly")}
            onChange={(weekly) => {
              void backend.invoke("settings_set_backup", { weekly });
            }}
          />
        </StatusRow>
        <StatusRow
          label={
            <span className="flex items-center gap-2">
              {t("settings.brief.privacy.backup.auto.folder")}
              <Badge tone={auto.custom ? "accent" : "neutral"}>
                {auto.custom
                  ? t("settings.brief.privacy.backup.auto.custom")
                  : t("settings.brief.privacy.backup.auto.default")}
              </Badge>
            </span>
          }
          help={
            <span className="flex flex-col gap-0.5">
              <span
                className="mono break-all select-text"
                title={auto.folder}
                data-testid="auto-backup-folder">
                {auto.folder}
              </span>
              <span className="text-fg-subtle">
                {t("settings.brief.privacy.backup.auto.folderHelp")}
              </span>
              <span className="text-fg-subtle" data-testid="auto-backup-sync">
                {t("settings.brief.privacy.backup.auto.sync", { device: auto.device })}
              </span>
            </span>
          }>
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            <Button
              size="sm"
              variant="outline"
              icon="folder"
              disabled={choosing}
              onClick={() => {
                void choose();
              }}>
              {t("settings.brief.privacy.backup.auto.change")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                backend.backupFolderOpen().catch((e: unknown) => {
                  fail(errorText(e));
                });
              }}>
              {t("settings.brief.privacy.backup.auto.open")}
            </Button>
            {auto.custom && (
              <Button
                size="sm"
                variant="text"
                disabled={choosing}
                onClick={() => {
                  void backend.invoke("backup_folder_reset");
                }}>
                {t("settings.brief.privacy.backup.auto.reset")}
              </Button>
            )}
          </div>
        </StatusRow>
        <StatusRow label={t("settings.brief.privacy.backup.auto.last")}>
          <span className="text-[13px] text-fg-muted" data-testid="auto-backup-last">
            {auto.last === null
              ? t("settings.brief.privacy.backup.auto.none")
              : t("settings.brief.privacy.backup.auto.lastValue", {
                  time: timeOf(auto.last.at_ms),
                  kind: kindOf(auto.last.kind),
                })}
          </span>
        </StatusRow>
      </SettingsRows>
      {failure !== null && (
        <p
          role="status"
          className="mt-2 flex items-start gap-1.5 text-[12px] leading-[18px] break-words text-danger"
          data-testid="auto-backup-failure">
          <Icon name="alert" size={13} className="mt-[2px] shrink-0" />
          <span className="flex min-w-0 flex-col">
            <span>
              {t("settings.brief.privacy.backup.auto.failed", {
                kind: kindOf(failure.kind),
                time: timeOf(failure.at_ms),
                detail: failure.detail,
              })}
            </span>
            {failure.kind === "weekly" && (
              <span>{t("settings.brief.privacy.backup.auto.retry")}</span>
            )}
          </span>
        </p>
      )}
    </SettingsSection>
  );
}

/** The preview of an opened backup, the parts and the mode, then the report. Closed before the
 *  import, it lets the shell drop the backup. */
function BackupImportDialog({
  token,
  manifest,
  onClose,
}: {
  token: string;
  manifest: BackupManifest;
  onClose: () => void;
}) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const counts = manifest.counts;
  const [parts, setParts] = useState<BackupParts>(() => ({
    history: counts.history > 0,
    dictionary: counts.dictionary > 0,
    rules: counts.rules > 0,
    scenes: counts.scenes > 0,
    presets: counts.presets > 0,
  }));
  const [mode, setMode] = useState<ImportMode>("merge");
  const [importing, setImporting] = useState(false);
  const [report, setReport] = useState<BackupImportReport | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);
  const close = () => {
    // The import runs to its end; its report follows.
    if (importing) return;
    // A backup not imported is let go of; after an import the shell holds nothing.
    if (report === undefined) void backend.backupDiscard(token).catch(() => undefined);
    onClose();
  };
  const run = async () => {
    setImporting(true);
    setError(undefined);
    try {
      const outcome = await backend.backupImport(token, parts, mode);
      if (outcome.kind === "done") setReport(outcome.report);
      else {
        setError(
          t(`settings.brief.privacy.backup.importFailed.${outcome.code}`, {
            detail: outcome.detail ?? "",
          }),
        );
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setImporting(false);
    }
  };
  if (report !== undefined) {
    return (
      <Dialog
        open
        title={t("settings.brief.privacy.backup.dialog.doneTitle")}
        width={520}
        onClose={onClose}
        actions={
          <Button size="sm" variant="primary" onClick={onClose} data-autofocus>
            {t("common.done")}
          </Button>
        }>
        <ul className="flex flex-col gap-2" data-testid="backup-result">
          {BACKUP_PARTS.map((part) => {
            const row = report.parts.find((p) => p.part === part);
            return row === undefined ? null : <PartResult key={part} report={row} />;
          })}
        </ul>
        {report.snapshot !== undefined && (
          <p
            className="mt-3 text-[12px] leading-4 break-all text-fg-muted select-text"
            data-testid="backup-snapshot">
            {t("settings.brief.privacy.backup.result.snapshot", { path: report.snapshot })}
          </p>
        )}
      </Dialog>
    );
  }
  const keep = state.settings.history.keep;
  const wouldKeep = counts.history + (mode === "merge" ? state.history_total : 0);
  const chosen = BACKUP_PARTS.some((part) => parts[part]);
  const platform = manifest.platform === "other" ? undefined : platformLabel(manifest.platform);
  return (
    <Dialog
      open
      title={t("settings.brief.privacy.backup.dialog.title")}
      width={560}
      onClose={close}
      actions={
        <>
          <Button size="sm" variant="ghost" disabled={importing} onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant={mode === "replace" ? "danger" : "primary"}
            disabled={!chosen || importing}
            onClick={() => {
              void run();
            }}>
            {importing
              ? t("settings.brief.privacy.backup.dialog.importing")
              : mode === "replace"
                ? t("settings.brief.privacy.backup.dialog.replaceAction")
                : t("settings.brief.privacy.backup.dialog.importAction")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-4">
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[13px]">
          <dt className="text-fg-muted">{t("settings.brief.privacy.backup.dialog.source")}</dt>
          <dd className="min-w-0 truncate text-fg">
            {platform === undefined
              ? manifest.device
              : t("settings.brief.privacy.backup.dialog.sourceValue", {
                  device: manifest.device,
                  platform,
                })}
          </dd>
          <dt className="text-fg-muted">{t("settings.brief.privacy.backup.dialog.created")}</dt>
          <dd className="text-fg">
            {formatDateTime(locale, manifest.created_at_ms, {
              dateStyle: "medium",
              timeStyle: "short",
            })}
          </dd>
          <dt className="text-fg-muted">{t("settings.brief.privacy.backup.dialog.version")}</dt>
          <dd className="text-fg">
            {t("settings.brief.privacy.backup.dialog.versionValue", {
              app: manifest.app_version,
              format: manifest.version,
            })}
          </dd>
        </dl>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-[13px] font-medium text-fg">
            {t("settings.brief.privacy.backup.dialog.parts")}
          </legend>
          {BACKUP_PARTS.map((part) => {
            const n = counts[part];
            const name = t(`settings.brief.privacy.backup.part.${part}`);
            return (
              <div
                key={part}
                className="flex items-center justify-between gap-3"
                data-testid={`backup-part-${part}`}>
                <Toggle
                  checked={parts[part]}
                  disabled={n === 0 || importing}
                  ariaLabel={name}
                  label={name}
                  onChange={(on) => {
                    setParts((p) => ({ ...p, [part]: on }));
                  }}
                />
                <span className="mono text-[12px] text-fg-muted">
                  {n === 0 ? t("settings.brief.privacy.backup.dialog.none") : countOf(t, part, n)}
                </span>
              </div>
            );
          })}
          {parts.history && wouldKeep > keep && (
            <p className="text-[12px] leading-4 text-fg-subtle" data-testid="backup-keep-note">
              {t("settings.brief.privacy.backup.dialog.keepNote", { keep: formatCount(keep) })}
            </p>
          )}
        </fieldset>
        <div className="flex flex-col gap-2">
          <span className="text-[13px] font-medium text-fg">
            {t("settings.brief.privacy.backup.dialog.mode")}
          </span>
          <Segmented
            label={t("settings.brief.privacy.backup.dialog.mode")}
            size="sm"
            value={mode}
            onChange={setMode}
            className="self-start"
            options={[
              { value: "merge", label: t("settings.brief.privacy.backup.dialog.merge") },
              { value: "replace", label: t("settings.brief.privacy.backup.dialog.replace") },
            ]}
          />
          <p className="text-[12px] leading-4 text-fg-subtle">
            {mode === "merge"
              ? t("settings.brief.privacy.backup.dialog.mergeHelp")
              : t("settings.brief.privacy.backup.dialog.replaceHelp")}
          </p>
        </div>
        {error !== undefined && (
          <p className="text-[12px] text-danger" role="alert" data-testid="backup-import-error">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
}

/** One part of the report: what it restored, what it skipped and why, what went, or why nothing. */
function PartResult({ report }: { report: BackupPartReport }) {
  const { t } = useI18n();
  const count = (n: number) => countOf(t, report.part, n);
  const reasons = (["duplicate", "limit", "conflict", "invalid", "older"] as const)
    .filter((why) => report.skipped[why] > 0)
    .map((why) =>
      t(`settings.brief.privacy.backup.result.${why}`, { count: count(report.skipped[why]) }),
    );
  return (
    <li className="flex flex-col gap-0.5" data-testid={`backup-result-${report.part}`}>
      <span className="text-[13px] font-medium text-fg">
        {t(`settings.brief.privacy.backup.part.${report.part}`)}
      </span>
      {report.error === undefined ? (
        <>
          <span className="text-[12px] text-fg-muted">
            {t("settings.brief.privacy.backup.result.restored", { count: count(report.restored) })}
          </span>
          {reasons.length > 0 && (
            <span className="text-[12px] text-fg-muted">
              {t("settings.brief.privacy.backup.result.skipped", {
                reasons: reasons.join(t("settings.brief.privacy.backup.result.separator")),
              })}
            </span>
          )}
          {report.removed > 0 && (
            <span className="text-[12px] text-fg-muted">
              {t("settings.brief.privacy.backup.result.removed", { count: count(report.removed) })}
            </span>
          )}
        </>
      ) : (
        <span className="text-[12px] text-danger">
          {t(`settings.brief.privacy.backup.result.error.${report.error}`)}
        </span>
      )}
    </li>
  );
}
