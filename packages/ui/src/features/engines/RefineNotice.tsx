import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Banner } from "../../components/Banner";
import { Button } from "../../components/Button";
import { useT } from "../../i18n/I18nProvider";

export interface RefineNoticeProps {
  /** Show the AI models page (each shell navigates its own way). */
  onOpen: () => void;
  /** Extra classes for the button (the phone's touch target). */
  buttonClassName?: string;
}

/** The built-in AI polish service turned a take down for want of capacity (docs/dictation.md
 *  §3.6): what happened, and the page where a provider of one's own is set up. The core keeps the
 *  notice: it goes once a take is polished again, the clean-up moves to another provider, or the
 *  close button asks the core to drop it for a day. Home on the desktop, the talk screens on the
 *  phone. */
export function RefineNotice({ onOpen, buttonClassName }: RefineNoticeProps) {
  const { refine_notice: notice } = useUiState();
  const { backend } = useBackend();
  const t = useT();
  if (notice === undefined) return null;
  return (
    <Banner
      tone="warn"
      marker="bar"
      title={
        notice.failure === "quota"
          ? t("refineNotice.title.quota")
          : t("refineNotice.title.rate_limited")
      }
      onDismiss={() => {
        void backend.invoke("refine_notice_close");
      }}>
      <p data-testid="refine-notice">{t("refineNotice.body")}</p>
      <Button size="sm" variant="outline" className={buttonClassName ?? "mt-2"} onClick={onOpen}>
        {t("refineNotice.open")}
      </Button>
    </Banner>
  );
}
