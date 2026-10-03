import {
  APP_LICENSE,
  type ProjectLink,
  type UpdateStatus,
  coreMessageText,
  formatDateTime,
} from "@voltip/shared";
import {
  Button,
  Card,
  type LampTone,
  Logo,
  Readout,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { StateLine, TOUCH, TOUCH_TOGGLE } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

type T = ReturnType<typeof useI18n>["t"];

/** The lamp beside the update line, as the line reads it: the desktop's tones, with the download
 *  steps (which the phone words as a newer version, `updateLine`) lit as a newer version is. */
const UPDATE_TONE: Record<UpdateStatus["state"], LampTone> = {
  idle: "idle",
  checking: "accent",
  up_to_date: "ok",
  available: "accent",
  downloading: "accent",
  ready: "accent",
  installing: "accent",
  failed: "danger",
  store: "idle",
  disabled: "idle",
};

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
        <h2 className="eyebrow">{t("mobile.about.update.storeTitle")}</h2>
        <p className="text-[13px] leading-5 text-fg-muted">{t("mobile.about.update.storeBody")}</p>
        <Button
          variant="outline"
          icon="external"
          className={`${TOUCH} self-start`}
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
    <Card padding="none" data-testid="phone-update">
      <div className="flex flex-col gap-3 p-4">
        <h2 className="eyebrow">{t("mobile.about.update.title")}</h2>
        <p className="text-[13px] leading-5 text-fg-muted">{t("mobile.about.update.directBody")}</p>
        <StateLine tone={UPDATE_TONE[update.state]} pulse={update.state === "checking"}>
          <span
            className={update.state === "failed" ? "text-danger" : "text-fg"}
            role="status"
            data-testid="phone-update-status">
            {line}
          </span>
        </StateLine>
        {update.state === "available" && (
          <div className="flex flex-col gap-3">
            {update.notes !== undefined && (
              <details className="rounded-6 bg-inset px-3 text-[12px] leading-5">
                <summary className="cursor-pointer py-3 text-fg-muted">
                  {t("mobile.about.update.notes")}
                </summary>
                <p className="pb-3 whitespace-pre-wrap text-fg select-text">{update.notes}</p>
              </details>
            )}
            <Button
              variant="primary"
              icon="download"
              className={`${TOUCH} self-start`}
              onClick={install}>
              {t("mobile.about.update.download")}
            </Button>
            <p className="text-[12px] leading-5 text-fg-muted">
              {t("mobile.about.update.downloadHint")}
            </p>
          </div>
        )}
        <Button
          variant="outline"
          className={`${TOUCH} self-start`}
          disabled={update.state === "checking"}
          onClick={() => {
            backend.invoke("update_check").catch(fail);
          }}>
          {t("mobile.about.update.check")}
        </Button>
      </div>
      <div className="border-t border-border px-4">
        <StatusRow label={t("mobile.about.update.auto")} help={t("mobile.about.update.autoHelp")}>
          <Toggle
            checked={settings.auto_update}
            ariaLabel={t("mobile.about.update.auto")}
            className={TOUCH_TOGGLE}
            onChange={(enabled) => {
              backend.invoke("settings_set_auto_update", { enabled }).catch(fail);
            }}
          />
        </StatusRow>
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
      <Card padding="none">
        <div className="flex items-center gap-3 p-4">
          <Logo size={36} />
          <span className="text-[16px] font-semibold text-fg">Voltip</span>
        </div>
        <div className="border-t border-border px-4">
          <StatusRow label={t("mobile.about.version")}>
            <Readout label="" value={version} size="sm" />
          </StatusRow>
          <StatusRow label={t("mobile.about.license")}>
            <Readout label="" value={APP_LICENSE} size="sm" />
          </StatusRow>
        </div>
        <div className="flex flex-col gap-3 border-t border-border p-4">
          <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.licenseBody")}</p>
          <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.about.notices")}</p>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              icon="external"
              className={TOUCH}
              onClick={() => {
                open("source");
              }}>
              {t("mobile.about.source")}
            </Button>
            <Button
              variant="outline"
              icon="external"
              className={TOUCH}
              onClick={() => {
                open("releases");
              }}>
              {t("mobile.about.releases")}
            </Button>
          </div>
        </div>
      </Card>
      <UpdateCard />
    </div>
  );
}
