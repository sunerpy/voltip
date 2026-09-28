import {
  LOCALE_SETTINGS,
  type LocaleSetting,
  type TFunction,
  type UpdateStatus,
  formatDateTime,
} from "@voltip/shared";
import {
  Button,
  LampText,
  Segmented,
  SettingsPane,
  SettingsRows,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";

/** `12582912 / 48000000` → `26%`; without a total, the received megabytes. */
export function downloadProgress(received: number, total: number | undefined): string {
  if (total !== undefined && total > 0) return `${Math.round((received / total) * 100)}%`;
  return `${(received / 1_048_576).toFixed(1)} MB`;
}

/** One line for the updater's state (`state.update`), shared by the 通用 and 关于 panes. */
export function updateStatusLine(
  update: UpdateStatus,
  t: TFunction,
  formatAt: (unixSecs: number) => string,
): { text: string; tone: "ok" | "danger" | "accent" | "idle" | "warn" } {
  switch (update.state) {
    case "idle":
      return { text: t("settings.general.update.idle"), tone: "idle" };
    case "checking":
      return { text: t("settings.general.update.checking"), tone: "accent" };
    case "up_to_date":
      return {
        text: t("settings.general.update.up_to_date", {
          version: update.version,
          at: formatAt(update.checked_at),
        }),
        tone: "ok",
      };
    case "available":
      return {
        text: t("settings.general.update.available", {
          version: update.version,
          current: update.current,
        }),
        tone: "accent",
      };
    case "downloading":
      return {
        text: t("settings.general.update.downloading", {
          version: update.version,
          progress: downloadProgress(update.received, update.total),
        }),
        tone: "accent",
      };
    case "ready":
      return { text: t("settings.general.update.ready", { version: update.version }), tone: "ok" };
    case "installing":
      return {
        text: t("settings.general.update.installing", { version: update.version }),
        tone: "accent",
      };
    case "failed":
      return {
        text: t("settings.general.update.failed", { message: update.message }),
        tone: "danger",
      };
    case "disabled":
      return { text: t("settings.general.update.disabled"), tone: "idle" };
  }
}

/** The updater's status line plus the one action its state allows (check / update / restart). */
export function UpdateControls({ compact = false }: { compact?: boolean }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { update } = useUiState();
  const line = updateStatusLine(update, t, (secs) =>
    formatDateTime(locale, secs * 1000, { dateStyle: "medium", timeStyle: "short" }),
  );
  const busy =
    update.state === "checking" || update.state === "downloading" || update.state === "installing";
  const disabled = update.state === "disabled";
  return (
    <div className="flex flex-wrap items-center gap-3" data-testid="update-controls">
      <LampText
        tone={line.tone}
        size="sm"
        mono={compact}
        pulse={update.state === "checking" || update.state === "downloading"}>
        <span data-testid="update-status">{line.text}</span>
      </LampText>
      {(update.state === "available" || update.state === "downloading") && (
        <Button
          size="sm"
          variant="primary"
          icon="download"
          onClick={() => {
            shell.setUpdateOpen(true);
          }}>
          {t("settings.general.viewUpdate")}
        </Button>
      )}
      {update.state === "ready" && (
        <Button
          size="sm"
          variant="primary"
          icon="refresh"
          onClick={() => {
            void backend.invoke("update_install");
          }}>
          {t("settings.general.restartInstall")}
        </Button>
      )}
      {update.state !== "available" &&
        update.state !== "downloading" &&
        update.state !== "ready" && (
          <Button
            size="sm"
            variant={compact ? "ghost" : "outline"}
            disabled={busy || disabled}
            loading={update.state === "checking"}
            onClick={() => {
              void backend.invoke("update_check");
            }}>
            {update.state === "checking"
              ? t("settings.general.checking")
              : t("settings.general.checkUpdate")}
          </Button>
        )}
    </div>
  );
}

/** Settings · 通用: the UI language (`settings_set_locale`, shared by every window and the phone),
 *  the setup guide, and automatic updates (`settings_set_auto_update`, `update_check`,
 *  `update_install`). */
export function General() {
  const { backend } = useBackend();
  const { navigate } = useRouter();
  const { t } = useI18n();
  const { settings } = useUiState();
  return (
    <SettingsPane title={t("settings.general.title")} lede={t("settings.general.lede")}>
      <SettingsRows>
        <StatusRow label={t("settings.general.language")} help={t("settings.general.languageHelp")}>
          <Segmented
            label={t("settings.general.language")}
            value={settings.locale}
            onChange={(locale: LocaleSetting) => {
              void backend.invoke("settings_set_locale", { locale });
            }}
            options={LOCALE_SETTINGS.map((value) => ({
              value,
              label: t(`settings.general.locale.${value}`),
            }))}
          />
        </StatusRow>
        <StatusRow label={t("settings.general.guide")} help={t("settings.general.guideHelp")}>
          <Button
            size="sm"
            onClick={() => {
              navigate({ name: "onboarding", step: 1 });
            }}>
            {t("settings.general.guideRun")}
          </Button>
        </StatusRow>
        <StatusRow
          label={t("settings.general.autoUpdate")}
          help={t("settings.general.autoUpdateHelp")}>
          <Toggle
            checked={settings.auto_update}
            onChange={(enabled) => {
              void backend.invoke("settings_set_auto_update", { enabled });
            }}
            label={t("settings.general.autoUpdate")}
          />
        </StatusRow>
        <div className="py-4" data-testid="update-section">
          <UpdateControls />
        </div>
      </SettingsRows>
    </SettingsPane>
  );
}
