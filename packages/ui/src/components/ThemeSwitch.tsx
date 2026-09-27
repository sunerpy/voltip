import { THEME_IDS, type ThemeId } from "@voltip/shared";
import { useId } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon, type IconName } from "./Icon";

/** A theme, or following the system's light / dark preference. */
export type ThemeChoice = ThemeId | "system";

/** The order the glyph steps through, and the menu's order. */
export const THEME_CHOICES: readonly ThemeChoice[] = ["system", ...THEME_IDS];

export function nextThemeChoice(choice: ThemeChoice): ThemeChoice {
  const i = THEME_CHOICES.indexOf(choice);
  return THEME_CHOICES[(i + 1) % THEME_CHOICES.length] ?? "system";
}

const GLYPH: Record<ThemeChoice, IconName> = {
  system: "monitor",
  light: "sun",
  dark: "moon",
  warm: "paper",
  graphite: "gauge",
};

export interface ThemeSwitchProps {
  value: ThemeChoice;
  onChange: (choice: ThemeChoice) => void;
  /** In the icon rail only the glyph is left. */
  collapsed?: boolean;
}

/** The sidebar's theme entry: two controls in one row. The glyph steps to the next theme at a
 *  click (its name says which one comes next); the name next to it is a menu of all five. They
 *  stay two elements because one element with two gestures would open the menu at every step. */
export function ThemeSwitch({ value, onChange, collapsed = false }: ThemeSwitchProps) {
  const t = useT();
  const menuId = useId();
  const name = (choice: ThemeChoice) =>
    choice === "system" ? t("theme.followSystem") : t(`theme.name.${choice}`);
  const next = nextThemeChoice(value);
  const stepLabel = t("ui.themeSwitch.next", { theme: name(next) });
  return (
    <div
      data-testid="theme-switch"
      data-theme-choice={value}
      className={cx("flex h-9 items-center rounded-6", collapsed ? "justify-center" : "gap-1")}>
      <button
        type="button"
        aria-label={stepLabel}
        title={stepLabel}
        onClick={() => {
          onChange(next);
        }}
        className={cx(
          "flex h-9 shrink-0 items-center justify-center rounded-6 text-fg-subtle transition-colors hover:bg-nav-active hover:text-fg",
          collapsed ? "w-full" : "w-9",
        )}>
        <Icon name={GLYPH[value]} size={16} />
      </button>
      {!collapsed && (
        <div className="relative min-w-0 flex-1">
          <label htmlFor={menuId} className="sr-only">
            {t("ui.themeSwitch.label")}
          </label>
          <select
            id={menuId}
            value={value}
            onChange={(e) => {
              const choice = THEME_CHOICES.find((c) => c === e.target.value);
              if (choice !== undefined) onChange(choice);
            }}
            className="h-9 w-full cursor-pointer appearance-none truncate rounded-6 bg-transparent pr-6 pl-1 text-[13px] text-fg-muted outline-none hover:bg-nav-active hover:text-fg focus-visible:bg-nav-active">
            {THEME_CHOICES.map((c) => (
              <option key={c} value={c}>
                {name(c)}
              </option>
            ))}
          </select>
          <Icon
            name="chevronDown"
            size={14}
            className="pointer-events-none absolute top-1/2 right-1.5 -translate-y-1/2 text-fg-subtle"
          />
        </div>
      )}
    </div>
  );
}
