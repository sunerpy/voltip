import { type ModelState, formatDateTime, modelDisplayName } from "@voltip/shared";
import {
  Badge,
  Button,
  CardGrid,
  OptionCard,
  Progress,
  SettingsSection,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useShell } from "../../../app/shell-context";
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
