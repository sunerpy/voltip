import { APP_LICENSE, type ProjectLink, coreMessageText } from "@voltip/shared";
import { Button, Card, Readout, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

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
    </div>
  );
}
