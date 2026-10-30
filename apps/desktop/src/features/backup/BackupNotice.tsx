import { Banner, Button, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useRouter } from "../../app/router";

/** Home's notice that the automatic backup before an update could not be written
 *  (docs/dictation.md §4.6.1): the update installed all the same, and the first run after it says
 *  so until the notice is closed. 查看备份设置 opens 隐私与历史, where the failure stays named until
 *  an automatic backup is written. */
export function BackupNotice() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { navigate } = useRouter();
  const notice = useUiState().auto_backup.notice;
  if (notice === null) return null;
  return (
    <Banner
      tone="warn"
      marker="bar"
      title={t("settings.brief.privacy.backup.auto.notice.title")}
      actions={
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            navigate({ name: "settings", section: "privacy" });
          }}>
          {t("settings.brief.privacy.backup.auto.notice.open")}
        </Button>
      }
      onDismiss={() => {
        void backend.invoke("backup_notice_dismiss");
      }}>
      <span data-testid="backup-notice">
        {t("settings.brief.privacy.backup.auto.notice.body", { detail: notice.detail })}
      </span>
    </Banner>
  );
}
