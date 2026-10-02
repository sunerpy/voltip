import {
  APP_LICENSE,
  type ProjectLink,
  type UpdateStatus,
  coreMessageText,
  formatDateTime,
} from "@voltip/shared";
import { Button, Card, Readout, Toggle, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

type T = ReturnType<typeof useI18n>["t"];

/** The updater's state in one line, in the desktop's words (`settings.general.update`); `current`
 *  is this build's version. */
function updateLine(
  update: UpdateStatus,
  t: T,
  formatAt: (unixSecs: number) => string,
  current: string,
): string {
  switch (update.state) {
    case "idle":
      return t("settings.general.update.idle");
    case "checking":
      return t("settings.general.update.checking");
    case "up_to_date":
      return t("settings.general.update.up_to_date", {
        version: update.version,
        at: formatAt(update.checked_at),
      });
    case "available":
      return t("settings.general.update.available", {
        version: update.version,
        current: update.current,
      });
    case "failed":
      return t("settings.general.update.failed", { message: coreMessageText(update.message) });
    // The desktop's download steps (and the store, which has a card of its own) never reach the
    // phone's line: a newer version is all they say.
    case "downloading":
    case "ready":
    case "installing":
    case "store":
      return t("settings.general.update.available", { version: update.version, current });
    case "disabled":
      return t("settings.general.update.disabled");
  }
}

/** 软件更新 (docs/dictation.md §20.9): Google Play updates what it installed, so its install only
 *  opens the listing; any other install asks GitHub (检查更新, or at start with 自动检查更新) and
 *  opens the newer release's APK in the browser, which Android installs over this one. */
function UpdateCard() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const { update, settings, app_version: version } = useUiState();
  const fail = (e: unknown) => {
    shell.toast(
      t("mobile.toast.error", {
        message: coreMessageText(e instanceof Error ? e.message : String(e)),
      }),
      "danger",
    );
  };
  const install = () => {
    backend.invoke("update_install").catch(fail);
  };
  if (update.state === "disabled") return null;
  if (update.state === "store") {
    return (
      <Card className="flex flex-col gap-3" data-testid="phone-update">
        <h2 className="text-[14px] font-medium text-fg">{t("mobile.about.update.storeTitle")}</h2>
        <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.update.storeBody")}</p>
        <Button
          size="sm"
          variant="outline"
          icon="external"
          className="self-start"
          onClick={install}>
          {t("mobile.about.update.openStore")}
        </Button>
      </Card>
    );
  }
  const line = updateLine(
    update,
    t,
    (secs) => formatDateTime(locale, secs * 1000, { dateStyle: "medium", timeStyle: "short" }),
    version,
  );
  return (
    <Card className="flex flex-col gap-3" data-testid="phone-update">
      <h2 className="text-[14px] font-medium text-fg">{t("mobile.about.update.title")}</h2>
      <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.update.directBody")}</p>
      <p
        className={update.state === "failed" ? "text-[13px] text-danger" : "text-[13px] text-fg"}
        role="status"
        data-testid="phone-update-status">
        {line}
      </p>
      {update.state === "available" && (
        <div className="flex flex-col gap-2">
          {update.notes !== undefined && (
            <details className="rounded-6 bg-inset px-3 py-2 text-[12px] leading-5">
              <summary className="cursor-pointer text-fg-muted">
                {t("mobile.about.update.notes")}
              </summary>
              <p className="mt-2 whitespace-pre-wrap text-fg">{update.notes}</p>
            </details>
          )}
          <Button
            size="sm"
            variant="primary"
            icon="download"
            className="self-start"
            onClick={install}>
            {t("mobile.about.update.download")}
          </Button>
          <p className="text-[12px] leading-5 text-fg-muted">
            {t("mobile.about.update.downloadHint")}
          </p>
        </div>
      )}
      <Button
        size="sm"
        variant="outline"
        className="self-start"
        disabled={update.state === "checking"}
        onClick={() => {
          backend.invoke("update_check").catch(fail);
        }}>
        {t("mobile.about.update.check")}
      </Button>
      <div className="flex items-center gap-3">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-[14px] font-medium text-fg">{t("mobile.about.update.auto")}</span>
          <span className="text-[12px] leading-5 text-fg-muted">
            {t("mobile.about.update.autoHelp")}
          </span>
        </span>
        <Toggle
          checked={settings.auto_update}
          ariaLabel={t("mobile.about.update.auto")}
          onChange={(enabled) => {
            backend.invoke("settings_set_auto_update", { enabled }).catch(fail);
          }}
        />
      </div>
    </Card>
  );
}

/** 关于 on the phone: the version, the licence (AGPL-3.0-or-later after 0.0.20, user decision
 *  2026-10-01) and the project's pages, opened in the phone's browser (`project_link_open`). */
export function About() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t } = useI18n();
  const { app_version: version } = useUiState();
  const open = (link: ProjectLink) => {
    backend.projectLinkOpen(link).catch((e: unknown) => {
      shell.toast(
        t("mobile.toast.error", {
          message: coreMessageText(e instanceof Error ? e.message : String(e)),
        }),
        "danger",
      );
    });
  };
  return (
    <div className="flex flex-col gap-4 p-4" data-testid="phone-about">
      <Card className="flex flex-col gap-3">
        <Readout label={t("mobile.about.version")} value={version} />
        <Readout label={t("mobile.about.license")} value={APP_LICENSE} />
        <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.licenseBody")}</p>
        <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.notices")}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            icon="external"
            onClick={() => {
              open("source");
            }}>
            {t("mobile.about.source")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            icon="external"
            onClick={() => {
              open("releases");
            }}>
            {t("mobile.about.releases")}
          </Button>
        </div>
      </Card>
      <UpdateCard />
    </div>
  );
}
