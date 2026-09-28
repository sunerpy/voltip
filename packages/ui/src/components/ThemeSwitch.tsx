import { THEME_IDS, type ThemeId } from "@voltip/shared";
import { useId } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon, type IconName } from "./Icon";
import { SIDEBAR_GLYPH_SLOT_CLASS, SIDEBAR_ROW_CLASS } from "./Sidebar";

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

/** The sidebar's theme entry: two controls in one sidebar row. The glyph steps to the next theme
 *  at a click (its name says which one comes next); the name next to it is a menu of all five.
 *  They stay two elements because one element with two gestures would open the menu at every
 *  step. The row has the box of every other entry (`SIDEBAR_ROW_CLASS`) and one hover surface;
 *  the glyph button covers exactly the glyph slot, so the glyph and the name line up with 反馈 and
 *  设置 (user feedback 2026-09-28). */
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
      className={cx(
        SIDEBAR_ROW_CLASS,
        "group text-fg-muted hover:bg-nav-active hover:text-fg",
        collapsed && "justify-center",
      )}>
      <button
        type="button"
        aria-label={stepLabel}
        title={stepLabel}
        data-testid="theme-switch-step"
        onClick={() => {
          onChange(next);
        }}
        className={cx(
          "flex h-9 shrink-0 items-center rounded-6 text-fg-subtle outline-none group-hover:text-fg focus-visible:bg-nav-active",
          collapsed ? "w-full justify-center" : SIDEBAR_GLYPH_SLOT_CLASS,
        )}>
        <Icon name={GLYPH[value]} size={16} />
      </button>
      {!collapsed && (
        <div className="relative flex h-9 min-w-0 flex-1 items-center">
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
            className="h-9 w-full appearance-none truncate rounded-6 bg-transparent pr-8 pl-0 text-[13px] text-inherit outline-none focus-visible:bg-nav-active">
            {THEME_CHOICES.map((c) => (
              <option key={c} value={c}>
                {name(c)}
              </option>
            ))}
          </select>
          <Icon
            name="chevronDown"
            size={14}
            className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 text-fg-subtle"
          />
        </div>
      )}
    </div>
  );
}
