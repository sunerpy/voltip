// 外观与语言 (apps/mobile's Appearance): the interface language and the theme, the settings the
// desktop's 通用 and 外观 write (`settings_set_locale`, `settings_set_theme`). Each theme is shown
// as a small preview in its own colours.
import { LOCALE_SETTINGS, THEME_IDS, type ThemeId } from "@voltip/shared";
import { View, useColorScheme } from "react-native";
import { Icon, SegmentedButtons, Text, TouchableRipple } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { themeId, themePreview } from "../theme/themes";
import { CARD_RADIUS, Hint, Page, Section, SwitchRow, useAppTheme } from "../ui/kit";

function ThemeTile({
  id,
  selected,
  disabled,
  caption,
  onSelect,
}: {
  id: ThemeId;
  selected: boolean;
  disabled: boolean;
  caption: string;
  onSelect: () => void;
}) {
  const theme = useAppTheme();
  const tokens = themePreview(id);
  return (
    <TouchableRipple
      testID={`theme-${id}`}
      disabled={disabled}
      onPress={onSelect}
      accessibilityRole="radio"
      accessibilityState={{ checked: selected, disabled }}
      accessibilityLabel={caption}
      style={{ width: "47%", borderRadius: CARD_RADIUS, opacity: disabled && !selected ? 0.5 : 1 }}
      borderless>
      <View style={{ gap: 8 }}>
        <View
          style={{
            height: 96,
            borderRadius: CARD_RADIUS,
            borderWidth: selected ? 3 : 1,
            borderColor: selected ? theme.colors.primary : theme.colors.outlineVariant,
            backgroundColor: tokens.page,
            padding: 10,
            gap: 6,
          }}>
          <View style={{ height: 14, width: "60%", borderRadius: 4, backgroundColor: tokens.fg }} />
          <View
            style={{
              flex: 1,
              borderRadius: 8,
              backgroundColor: tokens.card,
              borderWidth: 1,
              borderColor: tokens.border,
              padding: 6,
              gap: 4,
            }}>
            <View
              style={{ height: 6, width: "80%", borderRadius: 3, backgroundColor: tokens.fgMuted }}
            />
            <View
              style={{ height: 10, width: 36, borderRadius: 5, backgroundColor: tokens.accent }}
            />
          </View>
          {selected && (
            <View style={{ position: "absolute", right: 8, top: 8 }}>
              <Icon source="check-circle" size={22} color={theme.colors.primary} />
            </View>
          )}
        </View>
        <Text variant="labelLarge" style={{ textAlign: "center" }}>
          {caption}
        </Text>
      </View>
    </TouchableRipple>
  );
}

export function Appearance() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { settings } = useUiState();
  const systemDark = useColorScheme() === "dark";
  const shown = themeId(settings, systemDark);
  const setTheme = (theme: ThemeId, followSystem: boolean) => {
    void backend.invoke("settings_set_theme", { theme, followSystem });
  };
  return (
    <Page testID="phone-appearance" gap={20}>
      <View style={{ gap: 8 }}>
        <Text variant="titleSmall" style={{ paddingHorizontal: 4 }}>
          {t("settings.general.language")}
        </Text>
        <SegmentedButtons
          value={settings.locale}
          onValueChange={(locale) => {
            void backend.invoke("settings_set_locale", { locale });
          }}
          buttons={LOCALE_SETTINGS.map((value) => ({
            value,
            label: t(`settings.general.locale.${value}`),
            testID: `locale-${value}`,
          }))}
        />
        <Hint>{t("settings.general.languageHelp")}</Hint>
      </View>
      <Section title={t("settings.appearance.themeGroup")} padded>
        <View
          accessibilityRole="radiogroup"
          accessibilityLabel={t("settings.appearance.themeGroup")}
          style={{
            flexDirection: "row",
            flexWrap: "wrap",
            justifyContent: "space-between",
            rowGap: 16,
          }}>
          {THEME_IDS.map((id) => (
            <ThemeTile
              key={id}
              id={id}
              selected={shown === id}
              disabled={settings.follow_system_theme}
              caption={
                id === "light" ? t("settings.appearance.lightCaption") : t(`theme.name.${id}`)
              }
              onSelect={() => {
                setTheme(id, false);
              }}
            />
          ))}
        </View>
      </Section>
      <Section>
        <SwitchRow
          icon="theme-light-dark"
          title={t("settings.appearance.followSystem")}
          description={t("settings.appearance.followSystemHelp", {
            scheme: t(`settings.appearance.scheme.${systemDark ? "dark" : "light"}`),
          })}
          value={settings.follow_system_theme}
          onValueChange={(follow) => {
            setTheme(settings.theme, follow);
          }}
          testID="follow-system-theme"
        />
      </Section>
    </Page>
  );
}
