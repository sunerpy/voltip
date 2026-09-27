import { type UpdateStatus, formatDateTime } from "@voltip/shared";
import { Button, Dialog, LampText, Progress, useBackend, useI18n, useUiState } from "@voltip/ui";
import type { ReactNode } from "react";
import { downloadProgress, updateStatusLine } from "../../pages/settings/General";
import { etaText, formatBytes, useDownloadRate } from "./download-rate";
import { ReleaseNotes } from "./release-notes";

export interface UpdateDialogProps {
  open: boolean;
  onClose: () => void;
}

/** The version the status is about, when it names one. */
export function statusVersion(update: UpdateStatus): string | undefined {
  switch (update.state) {
    case "available":
    case "downloading":
    case "ready":
    case "installing":
      return update.version;
    default:
      return undefined;
  }
}

/** The update dialog (docs/frontend.md §7): what the new version brings, then the download with
 *  its speed and the time left, then the restart. The release notes come from the updater's own
 *  manifest (`latest.json`), rendered as text: nothing in them is a live link. It stays open
 *  across the states, so one dialog walks the user from 发现新版本 to 重启并更新; closing it
 *  never cancels a download, which carries on in the background. */
export function UpdateDialog({ open, onClose }: UpdateDialogProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const { update, app_version: current } = useUiState();
  const rate = useDownloadRate(update);
  const version = statusVersion(update);

  const title =
    update.state === "available" || update.state === "downloading"
      ? t("update.title", { version: update.version })
      : update.state === "ready"
        ? t("update.titleReady", { version: update.version })
        : update.state === "installing"
          ? t("update.titleInstalling", { version: update.version })
          : update.state === "failed"
            ? t("update.titleFailed")
            : t("update.titleStatus");

  const install = () => {
    void backend.invoke("update_install");
  };
  const retry = () => {
    void backend.invoke("update_check");
  };

  let actions: ReactNode;
  switch (update.state) {
    case "available":
      actions = (
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("update.later")}
          </Button>
          <Button size="sm" variant="primary" icon="download" onClick={install} data-autofocus>
            {t("update.install")}
          </Button>
        </>
      );
      break;
    case "downloading":
      actions = (
        <Button size="sm" variant="ghost" onClick={onClose} data-autofocus>
          {t("update.background")}
        </Button>
      );
      break;
    case "ready":
      actions = (
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("update.later")}
          </Button>
          <Button size="sm" variant="primary" icon="refresh" onClick={install} data-autofocus>
            {t("update.restart")}
          </Button>
        </>
      );
      break;
    case "installing":
      actions = (
        <Button size="sm" variant="primary" disabled loading>
          {t("update.installing")}
        </Button>
      );
      break;
    case "failed":
      actions = (
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("update.close")}
          </Button>
          <Button size="sm" variant="primary" icon="refresh" onClick={retry} data-autofocus>
            {t("update.retry")}
          </Button>
        </>
      );
      break;
    default:
      actions = (
        <Button size="sm" variant="ghost" onClick={onClose} data-autofocus>
          {t("update.close")}
        </Button>
      );
  }

  const notes = update.state === "available" ? update.notes : undefined;
  const date = update.state === "available" ? update.date : undefined;
  const published =
    date !== undefined && !Number.isNaN(Date.parse(date))
      ? formatDateTime(locale, Date.parse(date), { dateStyle: "medium" })
      : undefined;
  const line = updateStatusLine(update, t, (secs) =>
    formatDateTime(locale, secs * 1000, { dateStyle: "medium", timeStyle: "short" }),
  );

  return (
    <Dialog open={open} title={title} width={600} onClose={onClose} actions={actions}>
      <div className="flex flex-col gap-4" data-testid="update-dialog" data-state={update.state}>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-fg-muted">
          {current.length > 0 && (
            <span className="mono" data-testid="update-current">
              {t("update.current", { version: current })}
            </span>
          )}
          {published !== undefined && (
            <span data-testid="update-published">{t("update.published", { date: published })}</span>
          )}
          {version !== undefined && (
            <Button
              size="sm"
              variant="ghost"
              icon="external"
              className="ml-auto"
              onClick={() => {
                void backend.projectLinkOpen("releases");
              }}>
              {t("update.releases")}
            </Button>
          )}
        </div>
        {update.state === "available" && (
          <section
            aria-label={t("update.notes")}
            className="max-h-[320px] overflow-auto rounded-10 bg-inset p-4 hairline">
            {notes !== undefined && notes.trim().length > 0 ? (
              <ReleaseNotes markdown={notes} version={update.version} />
            ) : (
              <p className="text-[13px] text-fg-muted">{t("update.noNotes")}</p>
            )}
          </section>
        )}
        {update.state === "downloading" && (
          <div className="flex flex-col gap-2" data-testid="update-progress">
            <Progress
              value={
                update.total !== undefined && update.total > 0
                  ? update.received / update.total
                  : undefined
              }
              indeterminate={update.total === undefined || update.total === 0}
              size={6}
              label={t("update.downloading", {
                progress: downloadProgress(update.received, update.total),
              })}
            />
            <div className="mono flex flex-wrap gap-x-3 text-[11px] text-fg-muted">
              <span>{downloadProgress(update.received, update.total)}</span>
              {update.total !== undefined && (
                <span>
                  {t("update.size", {
                    received: formatBytes(update.received),
                    total: formatBytes(update.total),
                  })}
                </span>
              )}
              {rate !== undefined && (
                <span data-testid="update-speed">
                  {t("update.speed", { speed: formatBytes(rate) })}
                </span>
              )}
              {etaText(update.received, update.total, rate, t) !== undefined && (
                <span data-testid="update-eta">
                  {etaText(update.received, update.total, rate, t)}
                </span>
              )}
            </div>
          </div>
        )}
        {update.state === "ready" && (
          <LampText tone="ok" size="sm">
            {t("update.ready")}
          </LampText>
        )}
        {update.state === "failed" && (
          <p role="alert" className="text-[13px] text-danger">
            {t("update.failed", { message: update.message })}
          </p>
        )}
        {update.state !== "available" &&
          update.state !== "downloading" &&
          update.state !== "ready" &&
          update.state !== "failed" && (
            <LampText tone={line.tone} size="sm">
              {line.text}
            </LampText>
          )}
      </div>
    </Dialog>
  );
}

/** The title bar's note that an update is waiting: the version, the download's progress, or the
 *  restart it needs. Nothing when there is nothing to update. */
export function UpdateBadge({ onOpen }: { onOpen: () => void }) {
  const { t } = useI18n();
  const { update } = useUiState();
  let label: string;
  switch (update.state) {
    case "available":
      label = t("update.badge.available", { version: update.version });
      break;
    case "downloading":
      label = t("update.badge.downloading", {
        progress: downloadProgress(update.received, update.total),
      });
      break;
    case "ready":
      label = t("update.badge.ready");
      break;
    default:
      return null;
  }
  return (
    <button
      type="button"
      data-testid="update-badge"
      title={t("update.badgeTitle")}
      onClick={onOpen}
      className="inline-flex h-7 items-center gap-1.5 rounded-full bg-accent-soft px-2.5 text-[12px] whitespace-nowrap text-accent-text transition-colors hover:bg-accent hover:text-accent-fg">
      <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-accent" />
      {label}
    </button>
  );
}
