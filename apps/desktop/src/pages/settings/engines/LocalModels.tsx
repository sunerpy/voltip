import { type ModelState, formatDateTime, modelDisplayName } from "@voltip/shared";
import {
  Badge,
  Button,
  CardGrid,
  IconButton,
  OptionCard,
  Progress,
  SettingsSection,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { copyWithToast, useShell } from "../../../app/shell-context";
import {
  STATE_BADGE_TONE,
  activateLocalModel,
  downloadFraction,
  formatBytes,
  languageSummary,
  modelAction,
  modelDescription,
  modelEngineLabel,
  modelStateCell,
  offlineModelsByTier,
} from "./helpers";

/** The local model manager (docs/dictation.md §10): one card per recognition model of
 *  `state.models`, in product-tier order (均衡 → 高精度 → 轻量 → 轻量 · 中文), every button a real
 *  core command — `model_download` / `model_cancel` / `model_remove` — and activation =
 *  `settings_set_engines { asr_provider: "local", local_model }`. Download progress, the failure
 *  message and the installed path all come from the core's `models` events. The streaming preview
 *  model is not a recognition choice: it lives in the 实时预览 block (`LivePreview`). */
export function LocalModels() {
  const { t } = useI18n();
  const state = useUiState();
  const models = offlineModelsByTier(state.models);
  const installed = models.filter((m) => m.state.kind === "installed").length;
  return (
    <SettingsSection
      title={t("engines.localModels")}
      description={t("engines.localModelsNote")}
      data-testid="local-models"
      aside={
        <span className="mono text-[11px] text-fg-muted">
          {t("engines.installedCount", { installed, total: models.length })}
        </span>
      }>
      <CardGrid min={260} role="list" aria-label={t("engines.modelLibrary")}>
        {models.map((m) => (
          <div key={m.id} role="listitem" data-tier={m.tier} className="flex min-w-0">
            <ModelCard model={m} />
          </div>
        ))}
      </CardGrid>
    </SettingsSection>
  );
}

export interface ModelCardProps {
  model: ModelState;
}

/** One catalogue model: name · engine · state badge, languages / size / description, the progress
 *  row while downloading (received / total, file), the failure message, the installed path and
 *  date, and the one button the state allows (download / cancel / retry / use / in use) plus
 *  delete for installed files. The streaming preview model gets no 使用此模型: it is not a
 *  recognition model (`modelAction` → `installed`). Names come from the core under zh-CN and
 *  from the dictionary by id under en (`modelDisplayName`). */
export function ModelCard({ model }: ModelCardProps) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const uiState = useUiState();
  const cell = modelStateCell(model.state, t);
  const action = modelAction(model);
  const size = formatBytes(model.size_bytes);
  const engineLabel = modelEngineLabel(model.engine, t);
  const languages = languageSummary(model.languages, t);
  const name = modelDisplayName(model.id, model.name, locale);

  const download = () => {
    void backend.invoke("model_download", { id: model.id });
  };
  const cancel = () => {
    void backend.invoke("model_cancel", { id: model.id });
  };
  const use = () => {
    void backend.invoke("settings_set_engines", {
      engines: activateLocalModel(uiState.settings.engines, model.id),
    });
    shell.toast({ message: t("model.activated", { name }), duration: 3000 });
  };
  const remove = () => {
    const path = model.state.kind === "installed" ? model.state.path : model.id;
    shell.confirm({
      title: t("model.removeTitle", { name }),
      body: t("model.removeBody", { path, size }),
      confirmLabel: t("model.removeConfirm"),
      tone: "danger",
      onConfirm: () => {
        void backend.invoke("model_remove", { id: model.id });
        shell.toast({ message: t("model.removed", { name }), duration: 3000 });
      },
    });
  };

  const primary = (() => {
    switch (action) {
      case "download":
        return (
          <Button size="sm" variant="primary" icon="download" onClick={download}>
            {t("model.action.download")}
          </Button>
        );
      case "cancel":
        return (
          <Button size="sm" icon="stop" onClick={cancel}>
            {t("model.action.cancel")}
          </Button>
        );
      case "retry":
        return (
          <Button size="sm" variant="primary" icon="refresh" onClick={download}>
            {t("model.action.retry")}
          </Button>
        );
      case "use":
        return (
          <Button
            size="sm"
            variant="primary"
            icon="check"
            title={t("model.useTitle", { name })}
            onClick={use}>
            {t("model.action.use")}
          </Button>
        );
      case "current":
        return (
          <Button size="sm" icon="check" disabled>
            {t("model.action.current")}
          </Button>
        );
      case "installed":
        return undefined;
    }
  })();

  return (
    <OptionCard
      icon="cpu"
      title={name}
      subtitle={`${model.id} · ${engineLabel}`}
      aria-label={name}
      selected={model.active}
      className="min-w-0 flex-1"
      badge={
        <>
          {model.recommended && <Badge tone="accent">{t("model.recommended")}</Badge>}
          {model.active && <Badge tone="ok">{t("model.action.current")}</Badge>}
          <Badge tone={STATE_BADGE_TONE[cell.tone]}>{cell.text}</Badge>
        </>
      }
      footer={
        <>
          {primary}
          {model.state.kind === "installed" && (
            <Button size="sm" variant="text-danger" icon="trash" onClick={remove}>
              {t("model.action.remove")}
            </Button>
          )}
          <span className="mono ml-auto truncate text-[11px] text-fg-subtle">{size}</span>
        </>
      }>
      <div className="flex flex-wrap gap-1.5">
        <Badge mono>{engineLabel}</Badge>
        <Badge mono title={languages.title}>
          {languages.text}
        </Badge>
        <Badge mono>{size}</Badge>
      </div>
      <p className="text-[12px] leading-4 text-fg-muted">{modelDescription(model, locale)}</p>
      {model.state.kind === "downloading" && (
        <div className="flex flex-col gap-1" data-testid="download-row">
          <Progress
            value={downloadFraction(model.state)}
            segments={24}
            size={6}
            label={t("model.state.downloading", {
              percent: Math.round(downloadFraction(model.state) * 100),
            })}
          />
          <div className="mono truncate text-[10px] text-fg-subtle">
            {t("model.progress", {
              received: formatBytes(model.state.received),
              total: formatBytes(model.state.total),
              file: model.state.file,
            })}
          </div>
        </div>
      )}
      {model.state.kind === "verifying" && (
        <div className="flex flex-col gap-1" data-testid="verifying-row">
          <Progress indeterminate size={6} label={t("model.state.verifying")} />
          <div className="mono text-[10px] text-fg-subtle">{t("model.verifyingNote")}</div>
        </div>
      )}
      {model.state.kind === "failed" && (
        <p
          className="rounded-6 bg-danger-soft px-3 py-2 text-[12px] text-danger"
          data-testid="model-failure">
          {t("model.failedPrefix")} · {model.state.message}
        </p>
      )}
      {model.state.kind !== "installed" && model.files.length > 0 && (
        <ManualDownload model={model} />
      )}
      {model.state.kind === "installed" && (
        <div
          className="mono flex flex-col gap-0.5 text-[11px] text-fg-muted"
          data-testid="model-installed">
          <span>
            {t("model.installedAt", {
              date: formatDateTime(locale, model.state.installed_at * 1000, {
                year: "numeric",
                month: "2-digit",
                day: "2-digit",
              }),
            })}
          </span>
          <span className="truncate text-fg-subtle" title={model.state.path}>
            {model.state.path}
          </span>
        </div>
      )}
    </OptionCard>
  );
}

/** 手动下载 (docs/dictation.md §10, user request 2026-10-02): for a network the app cannot
 *  download through. The folder the files go into (copy, or open it in the file manager), each
 *  file with the addresses a browser can fetch it from (`model_link_open`: the shell takes the
 *  address from the core; each copies too, for another device that can reach the sources), and
 *  检查并导入 (`model_import`), which installs what was put there or
 *  names what is missing. Open from the start when the last import came back incomplete. */
function ManualDownload({ model }: { model: ModelState }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const incomplete = model.state.kind === "import_incomplete" ? model.state : undefined;
  const [open, setOpen] = useState(incomplete !== undefined);
  const busy = model.state.kind === "downloading" || model.state.kind === "verifying";
  const fail = (e: unknown) => {
    shell.toast({
      message: String(e instanceof Error ? e.message : e),
      duration: 4000,
      tone: "danger",
    });
  };
  return (
    <div className="flex flex-col gap-2" data-testid="manual-download">
      <Button
        size="sm"
        variant="text"
        icon={open ? "chevronDown" : "chevronRight"}
        aria-expanded={open}
        className="self-start"
        onClick={() => {
          setOpen(!open);
        }}>
        {t("model.manual.toggle")}
      </Button>
      {(open || incomplete !== undefined) && (
        <div className="flex flex-col gap-2 rounded-6 bg-inset p-3 text-[12px] leading-5">
          <p className="text-fg-muted">{t("model.manual.lede")}</p>
          <div className="flex flex-col gap-1">
            <span className="text-fg-subtle">{t("model.manual.folder")}</span>
            <span
              className="mono break-all text-fg select-text"
              title={model.dir}
              data-testid="manual-download-dir">
              {model.dir}
            </span>
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="ghost"
                icon="copy"
                onClick={() => {
                  void copyWithToast(shell, model.dir, t("model.manual.folderCopied"));
                }}>
                {t("model.manual.copyFolder")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon="folder"
                onClick={() => {
                  backend.modelFolderOpen(model.id).catch(fail);
                }}>
                {t("model.manual.openFolder")}
              </Button>
            </div>
          </div>
          <ul className="flex flex-col gap-1.5" aria-label={t("model.manual.toggle")}>
            {model.files.map((file) => (
              <li
                key={file.name}
                className="flex flex-col gap-0.5"
                data-testid="manual-download-file">
                <span className="mono text-fg">
                  {t("model.manual.file", { name: file.name, size: formatBytes(file.size_bytes) })}
                </span>
                {file.urls.map((url, source) => (
                  <div key={url} className="flex items-start gap-1">
                    <button
                      type="button"
                      className="mono min-w-0 text-left break-all text-accent underline-offset-2 hover:underline"
                      aria-label={t("model.manual.open", { url })}
                      onClick={() => {
                        backend.modelLinkOpen(model.id, file.name, source).catch(fail);
                      }}>
                      {url}
                    </button>
                    <IconButton
                      icon="copy"
                      label={t("model.manual.copyLink", { url })}
                      onClick={() => {
                        void copyWithToast(shell, url, t("model.manual.linkCopied"));
                      }}
                    />
                  </div>
                ))}
              </li>
            ))}
          </ul>
          {incomplete !== undefined && (
            <div
              className="flex flex-col gap-1 rounded-6 bg-warning-soft px-3 py-2 text-warning"
              role="status"
              data-testid="manual-download-problems">
              {incomplete.missing.length > 0 && (
                <span>{t("model.manual.missing", { files: incomplete.missing.join("、") })}</span>
              )}
              {incomplete.mismatched.length > 0 && (
                <span>
                  {t("model.manual.mismatched", { files: incomplete.mismatched.join("、") })}
                </span>
              )}
            </div>
          )}
          <Button
            size="sm"
            variant="primary"
            icon="check"
            className="self-start"
            disabled={busy}
            onClick={() => {
              void backend.invoke("model_import", { id: model.id });
            }}>
            {t("model.manual.import")}
          </Button>
        </div>
      )}
    </div>
  );
}
