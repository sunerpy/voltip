import {
  HISTORY_KEEP_OPTIONS,
  HISTORY_LIMIT,
  HISTORY_MIN_KEEP,
  type MessageKey,
  formatCount,
} from "@voltip/shared";
import {
  Button,
  LampText,
  Select,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";
import { serviceTarget } from "./engines/helpers";

/** The retention choices the select offers (moved to `@voltip/shared`, shared with the phone). */
export const KEEP_OPTIONS = HISTORY_KEEP_OPTIONS;

/** A secret-store backend name (`SecretStore::backend_name`) as the user knows it. */
const BACKEND_KEYS: ReadonlyMap<string, MessageKey> = new Map<string, MessageKey>([
  ["keychain", "settings.brief.privacy.backend.keychain"],
  ["credential-manager", "settings.brief.privacy.backend.credential-manager"],
  ["secret-service", "settings.brief.privacy.backend.secret-service"],
  ["android-keystore", "settings.brief.privacy.backend.android-keystore"],
  ["memory", "settings.brief.privacy.backend.memory"],
]);

function backendKey(name: string): MessageKey {
  return BACKEND_KEYS.get(name) ?? "settings.brief.privacy.backend.unknown";
}

/** Settings · 隐私: what leaves this computer right now (from `state.engines` and the context
 *  switches), the history switch and retention (`settings_set_history`), 清空历史 (with a confirm)
 *  and where the provider keys live (`state.secret_backend`). */
export function PrivacyPane() {
  const { backend } = useBackend();
  const { navigate } = useRouter();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const engines = state.engines;
  const history = state.settings.history;
  const sharing = state.settings.context_sharing;
  const audioTarget = serviceTarget(engines.asr_provider, engines.asr_host, t);
  const textTarget =
    engines.refine_enabled && engines.llm_provider !== undefined
      ? serviceTarget(engines.llm_provider, engines.refine_host, t)
      : undefined;
  const textSent = engines.refine_enabled && engines.llm_provider !== undefined;
  const contextParts = [
    ...(sharing.app_name ? [t("settings.brief.privacy.contextApp")] : []),
    ...(sharing.window_title ? [t("settings.brief.privacy.contextTitle")] : []),
  ];
  const keep = Math.min(HISTORY_LIMIT, Math.max(HISTORY_MIN_KEEP, history.keep));
  // The saved retention is always offered, even when it is not one of the presets.
  const keepOptions: number[] = [...KEEP_OPTIONS];
  if (!keepOptions.includes(keep)) {
    keepOptions.push(keep);
    keepOptions.sort((a, b) => a - b);
  }
  const setHistory = (patch: Partial<typeof history>) => {
    void backend.invoke("settings_set_history", { ...history, ...patch });
  };
  const clear = () => {
    shell.confirm({
      title: t("settings.brief.privacy.clearConfirmTitle"),
      body: t("settings.brief.privacy.clearConfirmBody", { n: formatCount(state.history_total) }),
      confirmLabel: t("settings.brief.privacy.clearConfirm"),
      tone: "danger",
      onConfirm: () => {
        void backend.invoke("history_clear");
        shell.toast({ message: t("settings.brief.privacy.cleared"), duration: 3000 });
      },
    });
  };

  return (
    <SettingsPane
      title={t("settings.brief.privacy.title")}
      lede={t("settings.brief.privacy.lede")}
      data-testid="privacy-pane">
      <SettingsSection title={t("settings.brief.privacy.sentTitle")} data-testid="privacy-sent">
        <SettingsRows>
          <StatusRow
            label={t("settings.brief.privacy.audio")}
            help={t("settings.brief.privacy.audioHelp")}>
            <LampText tone={audioTarget === undefined ? "ok" : "warn"} size="sm">
              <span data-testid="privacy-audio">
                {audioTarget === undefined
                  ? t("settings.brief.privacy.stays")
                  : t("settings.brief.privacy.sentTo", { target: audioTarget })}
              </span>
            </LampText>
          </StatusRow>
          <StatusRow
            label={t("settings.brief.privacy.text")}
            help={t("settings.brief.privacy.textHelp")}>
            <LampText tone={!textSent || textTarget === undefined ? "ok" : "warn"} size="sm">
              <span data-testid="privacy-text">
                {!textSent
                  ? t("settings.brief.privacy.notSent")
                  : textTarget === undefined
                    ? t("settings.brief.privacy.stays")
                    : t("settings.brief.privacy.sentTo", { target: textTarget })}
              </span>
            </LampText>
          </StatusRow>
          <StatusRow
            label={t("settings.brief.privacy.context")}
            help={t("settings.brief.privacy.contextHelp")}>
            <div className="flex items-center gap-3">
              <span className="text-[12px] text-fg" data-testid="privacy-context">
                {!textSent || contextParts.length === 0
                  ? t("settings.brief.privacy.contextNone")
                  : contextParts.join(" · ")}
              </span>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  navigate({ name: "settings", section: "scene" });
                }}>
                {t("settings.brief.privacy.openScenes")}
              </Button>
            </div>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection
        title={t("settings.brief.privacy.historyTitle")}
        data-testid="privacy-history"
        aside={
          <span className="mono text-[11px] text-fg-muted" data-testid="history-count">
            {t("settings.brief.privacy.historyCount", {
              n: formatCount(state.history_total),
              keep: formatCount(keep),
            })}
          </span>
        }>
        <SettingsRows>
          <StatusRow
            label={t("settings.brief.privacy.record")}
            help={t("settings.brief.privacy.recordHelp")}>
            <Toggle
              checked={history.enabled}
              ariaLabel={t("settings.brief.privacy.record")}
              onChange={(enabled) => {
                setHistory({ enabled });
              }}
            />
          </StatusRow>
          <StatusRow
            label={t("settings.brief.privacy.keep")}
            help={t("settings.brief.privacy.keepHelp")}>
            <Select
              label={t("settings.brief.privacy.keep")}
              size="sm"
              value={String(keep)}
              onChange={(value) => {
                setHistory({ keep: Number.parseInt(value, 10) });
              }}
              options={keepOptions.map((n) => ({
                value: String(n),
                label: t("settings.brief.privacy.keepOption", { n: formatCount(n) }),
              }))}
            />
          </StatusRow>
          <StatusRow
            label={t("settings.brief.privacy.clear")}
            help={t("settings.brief.privacy.clearHelp")}>
            <Button
              size="sm"
              variant="outline"
              icon="trash"
              className="text-danger"
              disabled={state.history_total === 0}
              onClick={clear}>
              {t("settings.brief.privacy.clear")}
            </Button>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection title={t("settings.brief.privacy.keysTitle")}>
        <SettingsRows>
          <StatusRow
            label={t("settings.brief.privacy.keys")}
            help={t("settings.brief.privacy.keysHelp")}>
            <span className="text-[12px] text-fg" data-testid="privacy-backend">
              {t(backendKey(state.secret_backend))}
            </span>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>
    </SettingsPane>
  );
}
