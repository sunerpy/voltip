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

/** Which theme the settings resolve to, honouring follow-system (light ↔ dark only). */
export function resolveTheme(
  settings: Pick<Settings, "theme" | "follow_system_theme">,
  systemDark: boolean,
): ThemeId {
  if (settings.follow_system_theme) return systemDark ? "dark" : "light";
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
  const current = root.dataset.theme ?? "light";
  return isThemeId(current) ? current : "light";
}
