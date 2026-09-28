import { APP_LICENSE, type ProjectLink } from "@voltip/shared";
import {
  Button,
  Readout,
  SettingsPane,
  SettingsRows,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { openProjectLink } from "../../app/project-links";
import { useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";
import { UpdateControls } from "./General";

/** Settings · 关于: the version the core reports (`state.app_version`, the shells' package
 *  version), the license, where the catalogue's local models come from, the updater, and the
 *  project's repository (opened by the shell, `project_link_open`), and the way to the 反馈
 *  dialog. */
export function AboutPane() {
  const { t } = useI18n();
  const state = useUiState();
  const { backend } = useBackend();
  const shell = useShell();
  const { navigate } = useRouter();
  const open = (link: ProjectLink) => {
    openProjectLink(backend, shell, link);
  };
  const sources = [...new Set(state.models.map((m) => m.repo).filter((r) => r.length > 0))];
  return (
    <SettingsPane
      title={t("settings.brief.about.title")}
      lede={t("settings.brief.about.lede")}
      data-testid="about-pane">
      <SettingsRows>
        <StatusRow
          label={t("settings.brief.about.version")}
          help={t("settings.brief.about.versionHelp")}>
          <Readout
            label=""
            value={state.app_version.length > 0 ? state.app_version : "—"}
            size="sm"
          />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.about.licence")}
          help={t("settings.brief.about.licenceHelp")}>
          <Readout label="" value={APP_LICENSE} size="sm" />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.about.models")}
          help={t("settings.brief.about.modelsHelp")}>
          {sources.length === 0 ? (
            <span className="text-[12px] text-fg-muted">
              {t("settings.brief.about.modelsNone")}
            </span>
          ) : (
            <ul
              className="mono flex flex-col items-end gap-0.5 text-[11px] text-fg"
              data-testid="model-sources">
              {sources.map((repo) => (
                <li key={repo} className="max-w-[320px] truncate" title={repo}>
                  {repo}
                </li>
              ))}
            </ul>
          )}
        </StatusRow>
        <StatusRow
          label={t("settings.brief.about.update")}
          help={t("settings.brief.about.updateHelp")}>
          <UpdateControls compact />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.about.source")}
          help={t("settings.brief.about.sourceHelp")}>
          <Button
            size="sm"
            icon="external"
            aria-label={t("settings.brief.about.source")}
            onClick={() => {
              open("source");
            }}>
            {t("settings.brief.about.open")}
          </Button>
        </StatusRow>
        {/* The same 反馈 dialog the sidebar opens, in this dialog's place (user feedback and
            decision 2026-09-28: this row used to send people to GitHub instead). */}
        <StatusRow
          label={t("settings.brief.about.feedback")}
          help={t("settings.brief.about.feedbackHelp")}>
          <Button
            size="sm"
            icon="chat"
            data-testid="about-feedback"
            onClick={() => {
              navigate({ name: "feedback" });
            }}>
            {t("settings.brief.about.writeFeedback")}
          </Button>
        </StatusRow>
      </SettingsRows>
    </SettingsPane>
  );
}
