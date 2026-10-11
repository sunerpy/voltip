import { THEME_IDS, type ThemeId } from "@voltip/shared";
import { useT } from "../i18n/I18nProvider";
import { Icon, type IconName } from "./Icon";
import { Menu } from "./Menu";
import { SIDEBAR_FOOTER_BUTTON_CLASS } from "./Sidebar";

/** A theme, or following the system's light / dark preference. */
export type ThemeChoice = ThemeId | "system";

/** The menu's order: following the system, then 澄空 and 夜空, then the classic four. */
export const THEME_CHOICES: readonly ThemeChoice[] = ["system", ...THEME_IDS];

/** The choice after `choice` in the menu's order, wrapping around (the command palette's 下一个). */
export function nextThemeChoice(choice: ThemeChoice): ThemeChoice {
  const i = THEME_CHOICES.indexOf(choice);
  return THEME_CHOICES[(i + 1) % THEME_CHOICES.length] ?? "system";
}

const GLYPH: Record<ThemeChoice, IconName> = {
  system: "monitor",
  glass: "sky",
  glass_dark: "nightSky",
  light: "sun",
  dark: "moon",
  warm: "paper",
  graphite: "gauge",
};

export interface ThemeSwitchProps {
  value: ThemeChoice;
  onChange: (choice: ThemeChoice) => void;
}

/** The sidebar's theme button (board 01, between 反馈 and 设置): the glyph of the current choice,
 *  opening a menu of the seven choices above it (the button sits at the window's bottom edge). Its
 *  name says the current choice, so the glyph alone never has to. */
export function ThemeSwitch({ value, onChange }: ThemeSwitchProps) {
  const t = useT();
  const name = (choice: ThemeChoice) =>
    choice === "system" ? t("theme.followSystem") : t(`theme.name.${choice}`);
  const label = `${t("ui.themeSwitch.label")} · ${name(value)}`;
  return (
    <Menu
      data-testid="theme-switch"
      label={t("ui.themeSwitch.label")}
      triggerLabel={label}
      title={label}
      side="top"
      className="w-full"
      triggerClassName={SIDEBAR_FOOTER_BUTTON_CLASS}
      trigger={<Icon name={GLYPH[value]} size={18} />}
      sections={[
        {
          items: THEME_CHOICES.map((choice) => ({
            kind: "radio" as const,
            id: choice,
            label: name(choice),
            checked: choice === value,
          })),
        },
      ]}
      onSelect={(id) => {
        const choice = THEME_CHOICES.find((c) => c === id);
        if (choice !== undefined) onChange(choice);
      }}
    />
  );
}
