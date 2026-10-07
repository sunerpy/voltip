// 历史记录 under the phone's settings (docs/dictation.md §4, §20.7): whether takes are recorded, how
// many are kept (`settings_set_history`, the desktop's choices), and 清空历史 after a confirmation.
import {
  HISTORY_KEEP_OPTIONS,
  HISTORY_LIMIT,
  HISTORY_MIN_KEEP,
  errorText,
  formatCount,
} from "@voltip/shared";
import { View } from "react-native";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { Lede, Mono, Page, Rows, Section, SwitchRow, useAppTheme } from "../ui/kit";
import { SelectRow } from "../ui/Select";

export function HistorySettings() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const history = state.settings.history;
  const total = state.history_total;
  const keep = Math.min(HISTORY_LIMIT, Math.max(HISTORY_MIN_KEEP, history.keep));
  const options: number[] = [...HISTORY_KEEP_OPTIONS];
  if (!options.includes(keep)) {
    options.push(keep);
    options.sort((a, b) => a - b);
  }
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const set = (patch: Partial<typeof history>) => {
    backend.invoke("settings_set_history", { ...history, keep, ...patch }).catch(fail);
  };
  return (
    <Page testID="phone-history-settings">
      <Lede>{t("mobile.historySettings.lede")}</Lede>
      <Section>
        <Rows>
          <SwitchRow
            icon="content-save-outline"
            title={t("settings.brief.privacy.record")}
            description={t("settings.brief.privacy.recordHelp")}
            value={history.enabled}
            onValueChange={(enabled) => {
              set({ enabled });
            }}
          />
          <SelectRow
            icon="counter"
            label={t("settings.brief.privacy.keep")}
            value={String(keep)}
            options={options.map((n) => ({
              value: String(n),
              label: t("settings.brief.privacy.keepOption", { n: formatCount(n) }),
            }))}
            onChange={(value) => {
              set({ keep: Number(value) });
            }}
          />
        </Rows>
      </Section>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 4 }}>
        <Mono style={{ flex: 1, color: theme.voltip.subtle }}>
          {t("settings.brief.privacy.historyCount", {
            n: formatCount(total),
            keep: formatCount(keep),
          })}
        </Mono>
        <Button
          textColor={theme.colors.error}
          disabled={total === 0}
          onPress={() => {
            shell.confirm({
              title: t("history.confirm.clearTitle", { n: total }),
              body: t("mobile.historySettings.clearBody", { n: formatCount(total) }),
              confirmLabel: t("history.confirm.clear"),
              onConfirm: () => {
                backend.invoke("history_clear").catch(fail);
              },
            });
          }}>
          {t("settings.brief.privacy.clear")}
        </Button>
      </View>
    </Page>
  );
}
