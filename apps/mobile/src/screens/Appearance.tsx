import { LOCALE_SETTINGS, type LocaleSetting, THEME_IDS, type ThemeId } from "@voltip/shared";
import {
  Card,
  Segmented,
  SettingsSection,
  StatusRow,
  ThemeTile,
  Toggle,
  systemPrefersDark,
  resolveTheme,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { TOUCH_TOGGLE } from "../app/phone-ui";

/** 外观与语言 on the phone (user decision 2026-10-01): the interface language and the theme, the
 *  same settings the desktop's 通用 and 外观 write (`settings_set_locale`, `settings_set_theme`),
 *  laid out as its sections. */
export function Appearance() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { settings } = useUiState();
  const systemDark = systemPrefersDark();
  const shown = resolveTheme(settings, systemDark);
  const setTheme = (theme: ThemeId, followSystem: boolean) => {
    void backend.invoke("settings_set_theme", { theme, followSystem });
  };
  return (
    <div className="flex flex-col gap-6 p-4" data-testid="phone-appearance">
      <SettingsSection
        title={t("settings.general.language")}
        description={t("settings.general.languageHelp")}>
        <Segmented<LocaleSetting>
          label={t("settings.general.language")}
          value={settings.locale}
          onChange={(locale) => {
            void backend.invoke("settings_set_locale", { locale });
          }}
          options={LOCALE_SETTINGS.map((value) => ({
            value,
            label: t(`settings.general.locale.${value}`),
          }))}
          className="h-11 w-full [&>button]:flex-1"
        />
      </SettingsSection>
      <SettingsSection title={t("settings.appearance.themeGroup")}>
        <Card>
          <div
            role="radiogroup"
            aria-label={t("settings.appearance.themeGroup")}
            className="grid grid-cols-2 justify-items-center gap-x-3 gap-y-4">
            {THEME_IDS.map((id) => (
              <ThemeTile
                key={id}
                theme={id}
                selected={shown === id}
                disabled={settings.follow_system_theme}
                caption={id === "light" ? t("settings.appearance.lightCaption") : id}
                onSelect={(theme) => {
                  setTheme(theme, false);
                }}
              />
            ))}
          </div>
        </Card>
        <Card padding="none" className="px-4">
          <StatusRow
            label={t("settings.appearance.followSystem")}
            help={t("settings.appearance.followSystemHelp", {
              scheme: t(`settings.appearance.scheme.${systemDark ? "dark" : "light"}`),
            })}>
            <Toggle
              checked={settings.follow_system_theme}
              className={TOUCH_TOGGLE}
              onChange={(follow) => {
                setTheme(settings.theme, follow);
              }}
              label={
                settings.follow_system_theme ? t("settings.appearance.followingNow") : undefined
              }
            />
          </StatusRow>
        </Card>
      </SettingsSection>
    </div>
  );
}
