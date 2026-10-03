import {
  OUTPUT_MODES,
  type OutputMode as OutputModeId,
  isStreamingOutputMode,
  outputModeDescription,
  outputModeLabel,
} from "@voltip/shared";
import {
  Badge,
  type IconName,
  CardGrid,
  LampText,
  OptionCard,
  SettingsSection,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";

const MODE_ICONS: Readonly<Record<OutputModeId, IconName>> = {
  whole_take: "mic",
  streaming_final: "wave",
  live_inject: "keyboard",
};

/** The 输出方式 block of the engines group (docs/dictation.md §12): three cards write
 *  `EngineSettings.output_mode` through `settings_set_engines` (the whole block, like every other
 *  engine setting). The status line and the 当前生效 badge read `state.engines.effective_output_mode`:
 *  a streaming mode picked while the streaming model is missing stays selected but runs as a whole
 *  take, and the card says so; 整段输出 on a realtime model runs as 边说边识别 (§11.9), and its card
 *  says that. */
export function OutputMode() {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const engines = state.engines;
  const effective = engines.effective_output_mode;
  const fallback = isStreamingOutputMode(settings.output_mode) && effective === "whole_take";
  const streamed = settings.output_mode === "whole_take" && engines.live_source === "stream";
  const setMode = (output_mode: OutputModeId) => {
    if (output_mode === settings.output_mode) return;
    void backend.invoke("settings_set_engines", { engines: { ...settings, output_mode } });
  };
  return (
    <SettingsSection
      title={t("engines.outputMode.title")}
      description={t("engines.outputMode.note")}
      data-testid="output-mode"
      data={{ "data-mode": settings.output_mode, "data-effective": effective }}
      aside={
        <LampText tone={fallback ? "warn" : "ok"} size="sm">
          <span data-testid="output-mode-state">
            {fallback ? t("outputMode.fallback") : outputModeLabel(effective, locale)}
          </span>
        </LampText>
      }>
      <CardGrid min={220} role="listbox" aria-label={t("engines.outputMode.label")}>
        {OUTPUT_MODES.map((mode) => {
          const selected = settings.output_mode === mode;
          const needsModel = isStreamingOutputMode(mode) && !engines.live_preview_ready;
          return (
            <OptionCard
              key={mode}
              icon={MODE_ICONS[mode]}
              title={outputModeLabel(mode, locale)}
              aria-label={outputModeLabel(mode, locale)}
              selected={selected}
              onSelect={() => {
                setMode(mode);
              }}
              badge={
                <>
                  {effective === mode && <Badge tone="ok">{t("outputMode.effective")}</Badge>}
                  {needsModel && (
                    <Badge tone="warn">{t("engines.livePreview.state.missing")}</Badge>
                  )}
                </>
              }>
              <p className="text-[12px] leading-4 text-fg-muted">
                {outputModeDescription(mode, locale)}
              </p>
              {selected && fallback && (
                <p
                  className="text-[12px] leading-4 text-warning"
                  data-testid="output-mode-fallback">
                  {t("outputMode.fallback")}
                </p>
              )}
              {selected && streamed && (
                <p
                  className="text-[12px] leading-4 text-fg-muted"
                  data-testid="output-mode-streamed">
                  {t("outputMode.streamed")}
                </p>
              )}
            </OptionCard>
          );
        })}
      </CardGrid>
    </SettingsSection>
  );
}

/** The 静音裁剪 switch (docs/dictation.md §12): `EngineSettings.vad_trim`, meaningful for local
 *  recognition only — under cloud recognition it is disabled and says why. */
export function VadTrim() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const local = state.engines.asr_provider === "local";
  const setVadTrim = (vad_trim: boolean) => {
    void backend.invoke("settings_set_engines", { engines: { ...settings, vad_trim } });
  };
  return (
    <SettingsSection
      title={t("engines.vadTrim.title")}
      description={t("engines.vadTrim.note")}
      data-testid="vad-trim"
      data={{ "data-state": local ? (settings.vad_trim ? "on" : "off") : "cloud" }}
      aside={
        <LampText tone={local && settings.vad_trim ? "ok" : "idle"} size="sm">
          <span data-testid="vad-trim-state">
            {local
              ? settings.vad_trim
                ? t("common.on")
                : t("common.off")
              : t("engines.provider.local")}
          </span>
        </LampText>
      }>
      <Toggle
        checked={settings.vad_trim}
        disabled={!local}
        onChange={setVadTrim}
        label={t("engines.vadTrim.toggle")}
      />
      {!local && (
        <p className="text-[12px] leading-4 text-fg-subtle" data-testid="vad-trim-cloud">
          {t("engines.vadTrim.cloud")}
        </p>
      )}
    </SettingsSection>
  );
}
