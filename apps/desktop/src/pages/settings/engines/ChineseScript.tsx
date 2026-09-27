import { CHINESE_SCRIPTS, type ChineseScript as Script } from "@voltip/shared";
import { Segmented, SettingsSection, useBackend, useI18n, useUiState } from "@voltip/ui";

const LABEL_KEYS = {
  simplified: "engines.chineseScript.simplified",
  traditional: "engines.chineseScript.traditional",
  as_is: "engines.chineseScript.asIs",
} as const;

/** The 中文字形 choice of the engines group (docs/dictation.md §17): `EngineSettings.chinese_script`
 *  through `settings_set_engines` (the whole block, like every engine setting). The core brings the
 *  Chinese of every recogniser's text to this script right after recognition, before the
 *  dictionary; the default is Simplified. */
export function ChineseScript() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const setScript = (chinese_script: Script) => {
    if (chinese_script === settings.chinese_script) return;
    void backend.invoke("settings_set_engines", { engines: { ...settings, chinese_script } });
  };
  return (
    <SettingsSection
      title={t("engines.chineseScript.title")}
      description={t("engines.chineseScript.note")}
      data-testid="chinese-script"
      data={{ "data-script": settings.chinese_script }}>
      <Segmented<Script>
        size="sm"
        label={t("engines.chineseScript.label")}
        value={settings.chinese_script}
        onChange={setScript}
        options={CHINESE_SCRIPTS.map((script) => ({ value: script, label: t(LABEL_KEYS[script]) }))}
        className="self-start"
      />
    </SettingsSection>
  );
}
