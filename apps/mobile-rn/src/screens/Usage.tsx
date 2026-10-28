// 设置 › 用量 (docs/dictation.md §27; Penpot 「用量与额度」 board 16b): this phone's day of the
// built-in recognition and clean-up against their limits, the last seven days, and the other
// providers this month. The page asks the service again as it opens.
import { dailyResetLine } from "@voltip/shared";
import { useEffect } from "react";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { OthersCard, ServiceUsageCard, useUsageNow } from "../features/Usage";
import { Hint, Lede, Page } from "../ui/kit";

export function Usage() {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const { devices } = useUiState();
  const now = useUsageNow();
  useEffect(() => {
    void backend.invoke("usage_refresh").catch(() => undefined);
  }, [backend]);
  return (
    <Page testID="phone-usage" gap={16}>
      <Lede>{t("mobile.usage.intro", { reset: dailyResetLine(now, locale) })}</Lede>
      <ServiceUsageCard kind="asr" />
      <ServiceUsageCard kind="llm" />
      <OthersCard />
      {devices.length > 0 && (
        <Hint testID="usage-computer-note">{t("mobile.usage.computerNote")}</Hint>
      )}
    </Page>
  );
}
