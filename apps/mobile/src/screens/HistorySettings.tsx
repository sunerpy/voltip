import {
  HISTORY_KEEP_OPTIONS,
  HISTORY_LIMIT,
  HISTORY_MIN_KEEP,
  errorText,
  formatCount,
} from "@voltip/shared";
import {
  Button,
  Card,
  Select,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { Lede, TOUCH, TOUCH_TOGGLE } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

/** 历史记录 under the phone's settings (docs/dictation.md §4, §20.7): whether takes are recorded,
 *  how many are kept (`settings_set_history`, the desktop's choices), and 清空历史 after a
 *  confirmation. */
export function HistorySettings() {
  const { backend } = useBackend();
  const shell = useMobileShell();
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
  const clear = () => {
    shell.confirm({
      title: t("history.confirm.clearTitle", { n: total }),
      body: t("mobile.historySettings.clearBody", { n: formatCount(total) }),
      confirmLabel: t("history.confirm.clear"),
      onConfirm: () => {
        backend.invoke("history_clear").catch(fail);
      },
    });
  };
  return (
    <div className="flex flex-col gap-4 p-4" data-testid="phone-history-settings">
      <Lede>{t("mobile.historySettings.lede")}</Lede>
      <Card padding="none" className="px-4">
        <StatusRow
          label={t("settings.brief.privacy.record")}
          help={t("settings.brief.privacy.recordHelp")}>
          <Toggle
            checked={history.enabled}
            ariaLabel={t("settings.brief.privacy.record")}
            className={TOUCH_TOGGLE}
            onChange={(enabled) => {
              set({ enabled });
            }}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.privacy.keep")}
          help={t("settings.brief.privacy.keepHelp")}>
          <Select
            aria-label={t("settings.brief.privacy.keep")}
            value={String(keep)}
            options={options.map((n) => ({
              value: String(n),
              label: t("settings.brief.privacy.keepOption", { n: formatCount(n) }),
            }))}
            onChange={(value) => {
              set({ keep: Number(value) });
            }}
          />
        </StatusRow>
      </Card>
      <div className="flex items-center gap-3 pl-1">
        <span className="mono flex-1 text-[11px] text-fg-subtle" data-testid="phone-history-count">
          {t("settings.brief.privacy.historyCount", {
            n: formatCount(total),
            keep: formatCount(keep),
          })}
        </span>
        <Button variant="text-danger" className={TOUCH} disabled={total === 0} onClick={clear}>
          {t("settings.brief.privacy.clear")}
        </Button>
      </div>
    </div>
  );
}
