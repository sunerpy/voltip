import { LOCALE_SETTINGS, type LocaleSetting, THEME_IDS, type ThemeId } from "@voltip/shared";
import {
  Card,
  Segmented,
  ThemeTile,
  Toggle,
  systemPrefersDark,
  resolveTheme,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";

/** 外观与语言 on the phone (user decision 2026-10-01): the interface language and the theme, the
 *  same settings the desktop's 通用 and 外观 write (`settings_set_locale`, `settings_set_theme`). */
export function Appearance() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { settings } = useUiState();
  const shown = resolveTheme(settings, systemPrefersDark());
  const setTheme = (theme: ThemeId, followSystem: boolean) => {
    void backend.invoke("settings_set_theme", { theme, followSystem });
  };
  return (
    <div className="flex flex-col gap-4 p-4" data-testid="phone-appearance">
      <Card className="flex flex-col gap-2">
        <span className="text-[14px] font-medium text-fg">{t("settings.general.language")}</span>
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
          className="self-start"
        />
        <p className="text-[12px] leading-5 text-fg-muted">{t("settings.general.languageHelp")}</p>
      </Card>
      <Card className="flex flex-col gap-3">
        <span className="text-[14px] font-medium text-fg">
          {t("settings.appearance.themeGroup")}
        </span>
        <div
          role="radiogroup"
          aria-label={t("settings.appearance.themeGroup")}
          className="grid grid-cols-2 justify-items-center gap-3">
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
        <div className="flex items-center justify-between gap-3">
          <span className="text-[13px] text-fg">{t("settings.appearance.followSystem")}</span>
          <Toggle
            checked={settings.follow_system_theme}
            onChange={(follow) => {
              setTheme(settings.theme, follow);
            }}
            label={settings.follow_system_theme ? t("settings.appearance.followingNow") : undefined}
          />
        </div>
      </Card>
    </div>
  );
}
