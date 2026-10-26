import type { ModelsLocationView, ModelsMoveOutcome } from "@voltip/shared";
import { Badge, Button, Icon, Progress, cx, useBackend, useI18n, useUiState } from "@voltip/ui";
import { copyWithToast, useShell } from "../../../app/shell-context";
import { formatBytes } from "./helpers";

/** 保存位置 (docs/dictation.md §10.9, issue #192 「模型下载，不能设置保存路径？C 盘满了」): the
 *  folder the model library is in, 更改位置… (the shell's folder dialog, then the move of what is
 *  downloaded), 恢复默认 (after a confirmation), the move's progress with 取消移动, and how the last
 *  move ended. The webview never names a folder: the dialog and the default are the shell's and
 *  the core's. Nothing when this app has no library (the phone). */
export function ModelsLocation() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const location = state.models_location;
  if (!location.available) return null;
  const moving = location.moving;
  const downloading = state.models.some(
    (m) => m.state.kind === "downloading" || m.state.kind === "verifying",
  );
  const locked = moving !== null || downloading;
  const fail = (e: unknown) => {
    shell.toast({
      message: String(e instanceof Error ? e.message : e),
      duration: 4000,
      tone: "danger",
    });
  };
  const choose = () => {
    backend.modelsLocationChoose(t("model.location.chooseTitle")).catch(fail);
  };
  const reset = () => {
    shell.confirm({
      title: t("model.location.resetTitle"),
      body: t("model.location.resetBody", { dir: location.default_dir }),
      confirmLabel: t("model.location.resetConfirm"),
      onConfirm: () => {
        void backend.invoke("models_location_reset");
      },
    });
  };
  return (
    <div
      className="flex flex-col gap-2 rounded-6 border border-border px-3 py-2.5"
      data-testid="models-location">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-medium text-fg">{t("model.location.title")}</span>
            <Badge tone={location.custom ? "accent" : "neutral"}>
              {location.custom ? t("model.location.custom") : t("model.location.default")}
            </Badge>
          </div>
          <span
            className="mono text-[12px] break-all text-fg-muted select-text"
            title={location.dir}
            data-testid="models-location-dir">
            {location.dir}
          </span>
          <span className="text-[11px] leading-4 text-fg-subtle">{t("model.location.help")}</span>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          <Button
            size="sm"
            variant="ghost"
            icon="copy"
            onClick={() => {
              void copyWithToast(shell, location.dir, t("model.location.copied"));
            }}>
            {t("model.location.copy")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            icon="folder"
            disabled={locked}
            title={downloading ? t("model.location.busyDownloading") : undefined}
            data-testid="models-location-change"
            onClick={choose}>
            {t("model.location.change")}
          </Button>
          {location.custom && (
            <Button
              size="sm"
              variant="text"
              disabled={locked}
              data-testid="models-location-reset"
              onClick={reset}>
              {t("model.location.reset")}
            </Button>
          )}
        </div>
      </div>
      {moving !== null && <MoveProgress location={location} />}
      {moving === null && downloading && (
        <span className="text-[12px] text-fg-muted">{t("model.location.busyDownloading")}</span>
      )}
      {moving === null && location.last !== null && <MoveOutcome outcome={location.last} />}
    </div>
  );
}

/** The move halfway: where to, how far (a running bar while it only renames), 取消移动. */
function MoveProgress({ location }: { location: ModelsLocationView }) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const moving = location.moving;
  if (moving === null) return null;
  const counted = moving.total_bytes > 0;
  const fraction = counted ? Math.min(1, moving.moved_bytes / moving.total_bytes) : 0;
  const label = t("model.location.moving", { dir: moving.to });
  return (
    <div className="flex flex-col gap-1" data-testid="models-location-moving">
      {counted ? (
        <Progress value={fraction} segments={24} size={6} label={label} />
      ) : (
        <Progress indeterminate size={6} label={label} />
      )}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="mono min-w-0 truncate text-[11px] text-fg-subtle" title={moving.to}>
          {label}
          {counted &&
            ` · ${t("model.location.progress", {
              moved: formatBytes(moving.moved_bytes),
              total: formatBytes(moving.total_bytes),
            })}`}
        </span>
        <Button
          size="sm"
          variant="text"
          icon="stop"
          data-testid="models-location-cancel"
          onClick={() => {
            void backend.invoke("models_location_cancel");
          }}>
          {t("model.location.cancel")}
        </Button>
      </div>
    </div>
  );
}

/** How the last move ended, until the next one starts. */
function MoveOutcome({ outcome }: { outcome: ModelsMoveOutcome }) {
  const { t } = useI18n();
  const [tone, icon, text] = ((): [
    "ok" | "warning" | "danger" | "muted",
    "check" | "info" | "alert",
    string,
  ] => {
    switch (outcome.kind) {
      case "moved":
        return outcome.deferred > 0
          ? ["warning", "info", t("model.location.movedDeferred", { n: outcome.deferred })]
          : ["ok", "check", t("model.location.moved")];
      case "cancelled":
        return ["muted", "info", t("model.location.cancelled")];
      case "failed":
        return ["danger", "alert", t("model.location.failed", { reason: outcome.message })];
    }
  })();
  return (
    <p
      role="status"
      data-testid="models-location-outcome"
      data-outcome={outcome.kind}
      className={cx(
        "flex items-start gap-1.5 text-[12px] leading-[18px] break-words",
        tone === "ok" && "text-ok",
        tone === "warning" && "text-warning",
        tone === "danger" && "text-danger",
        tone === "muted" && "text-fg-muted",
      )}>
      <Icon name={icon} size={13} className="mt-[2px] shrink-0" />
      <span className="min-w-0">{text}</span>
    </p>
  );
}
