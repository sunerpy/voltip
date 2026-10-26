import {
  AUDIO_CACHE_KEEP_OPTIONS,
  AUDIO_CACHE_MB_OPTIONS,
  clampAudioKeep,
  formatCount,
  megabytesLabel,
  storageSize,
  withSavedOption,
} from "@voltip/shared";
import {
  Button,
  Select,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useShell } from "../../app/shell-context";

/** Settings · 隐私与历史 · 录音缓存 (docs/dictation.md §26): keep the latest takes' recordings so a
 *  take can be recognised again, how many and how much space (`settings_set_audio_cache`), and
 *  清理录音 (`audio_cache_clear`, with a confirm). Nothing is kept while the history is off. */
export function RecordingsSection() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const cache = state.audio_cache;
  const settings = state.settings.audio_cache;
  const historyOn = state.settings.history.enabled;
  if (!cache.available) return null;
  const keep = clampAudioKeep(settings.keep);
  const set = (patch: Partial<typeof settings>) => {
    const next = { ...settings, ...patch };
    void backend.invoke("settings_set_audio_cache", {
      enabled: next.enabled,
      keep: next.keep,
      maxMb: next.max_mb,
    });
  };
  const n = cache.recordings.length;
  const size = storageSize(cache.bytes);
  const clear = () => {
    shell.confirm({
      title: t("settings.brief.privacy.recordings.clearConfirmTitle"),
      body: t("settings.brief.privacy.recordings.clearConfirmBody", { n: formatCount(n), size }),
      confirmLabel: t("settings.brief.privacy.recordings.clearConfirm"),
      tone: "danger",
      onConfirm: () => {
        void backend.invoke("audio_cache_clear");
        shell.toast({ message: t("settings.brief.privacy.recordings.cleared"), duration: 3000 });
      },
    });
  };
  const on = settings.enabled && historyOn;
  return (
    <SettingsSection
      title={t("settings.brief.privacy.recordings.title")}
      data-testid="privacy-recordings"
      aside={
        <span className="mono text-[11px] text-fg-muted" data-testid="recordings-count">
          {t("settings.brief.privacy.recordings.count", { n: formatCount(n), size })}
        </span>
      }>
      <SettingsRows>
        <StatusRow
          label={t("settings.brief.privacy.recordings.keep")}
          help={
            historyOn
              ? t("settings.brief.privacy.recordings.keepHelp")
              : t("settings.brief.privacy.recordings.historyOff")
          }>
          <Toggle
            checked={settings.enabled}
            disabled={!historyOn}
            ariaLabel={t("settings.brief.privacy.recordings.keep")}
            onChange={(enabled) => {
              set({ enabled });
            }}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.privacy.recordings.limit")}
          help={t("settings.brief.privacy.recordings.limitHelp")}>
          <Select
            label={t("settings.brief.privacy.recordings.limit")}
            size="sm"
            disabled={!on}
            value={String(keep)}
            onChange={(value) => {
              set({ keep: Number.parseInt(value, 10) });
            }}
            options={withSavedOption(AUDIO_CACHE_KEEP_OPTIONS, keep).map((k) => ({
              value: String(k),
              label: t("settings.brief.privacy.recordings.limitOption", { n: formatCount(k) }),
            }))}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.privacy.recordings.size")}
          help={t("settings.brief.privacy.recordings.sizeHelp")}>
          <Select
            label={t("settings.brief.privacy.recordings.size")}
            size="sm"
            disabled={!on}
            value={String(settings.max_mb)}
            onChange={(value) => {
              set({ max_mb: Number.parseInt(value, 10) });
            }}
            options={withSavedOption(AUDIO_CACHE_MB_OPTIONS, settings.max_mb).map((mb) => ({
              value: String(mb),
              label: megabytesLabel(mb),
            }))}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.brief.privacy.recordings.clear")}
          help={t("settings.brief.privacy.recordings.clearHelp")}>
          <Button
            size="sm"
            variant="outline"
            icon="trash"
            className="text-danger"
            disabled={n === 0}
            onClick={clear}>
            {t("settings.brief.privacy.recordings.clear")}
          </Button>
        </StatusRow>
      </SettingsRows>
      <p className="mt-2 text-[12px] leading-[18px] text-fg-muted" data-testid="recordings-note">
        {t("settings.brief.privacy.recordings.note")}
      </p>
    </SettingsSection>
  );
}
