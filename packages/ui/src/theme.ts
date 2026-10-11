import { THEME_IDS, type Settings, type ThemeId } from "@voltip/shared";

export { THEME_IDS };
export type { ThemeId };

export type Density = "compact" | "default";

/** The accent choices of 设置 › 外观 (ChatGPT's and Codex's accent colours, tokens.css):
 *  `default` is the theme's own accent (Codex's #339cff in the blue themes). */
export const ACCENT_IDS = [
  "default",
  "blue",
  "green",
  "yellow",
  "pink",
  "orange",
  "purple",
  "ink",
] as const;
export type AccentId = (typeof ACCENT_IDS)[number];

export function isAccentId(value: unknown): value is AccentId {
  return typeof value === "string" && (ACCENT_IDS as readonly string[]).includes(value);
}

export interface Appearance {
  theme: ThemeId;
  accent: AccentId;
  density: Density;
  fontSizePx: number;
  reduceMotion: boolean;
}

export const FONT_SIZE_MIN = 12;
export const FONT_SIZE_MAX = 18;
export const FONT_SIZE_DEFAULT = 14;

export function isThemeId(value: string): value is ThemeId {
  return (THEME_IDS as readonly string[]).includes(value);
}

/** The pair following the system switches between (user decision 2026-10-11): 澄空 for a light
 *  system, its dark twin 夜空 for a dark one. */
export const SYSTEM_LIGHT_THEME: ThemeId = "glass";
export const SYSTEM_DARK_THEME: ThemeId = "glass_dark";
/** A new install's theme (`voltip_core::Settings::default()`), and what an unknown value reads as. */
export const DEFAULT_THEME: ThemeId = "glass";

/** Whether `theme` is one of the dark themes (`color-scheme: dark`). */
export function isDarkTheme(theme: ThemeId): boolean {
  return theme === "dark" || theme === "graphite" || theme === "glass_dark";
}

/** Which theme the settings resolve to, honouring follow-system (澄空 ↔ 夜空). */
export function resolveTheme(
  settings: Pick<Settings, "theme" | "follow_system_theme">,
  systemDark: boolean,
): ThemeId {
  if (settings.follow_system_theme) return systemDark ? SYSTEM_DARK_THEME : SYSTEM_LIGHT_THEME;
  return settings.theme;
}

export interface MediaQueryHost {
  matchMedia?: Window["matchMedia"];
}

export function systemPrefersDark(win: MediaQueryHost = globalThis.window): boolean {
  if (typeof win.matchMedia !== "function") return false;
  return win.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function systemPrefersReducedMotion(win: MediaQueryHost = globalThis.window): boolean {
  if (typeof win.matchMedia !== "function") return false;
  return win.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Writes `data-theme` on `<html>`; every colour in the app follows from CSS variables. */
export function applyTheme(theme: ThemeId, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = theme;
}

export function applyAppearance(
  appearance: Appearance,
  root: HTMLElement = document.documentElement,
): void {
  applyTheme(appearance.theme, root);
  root.dataset.accent = appearance.accent;
  root.dataset.density = appearance.density;
  root.dataset.reduceMotion = appearance.reduceMotion ? "true" : "false";
  const size = Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, appearance.fontSizePx));
  root.style.setProperty("--ui-font-size", `${size}px`);
}

export function readTheme(root: HTMLElement = document.documentElement): ThemeId {
  const current = root.dataset.theme ?? DEFAULT_THEME;
  return isThemeId(current) ? current : DEFAULT_THEME;
}
