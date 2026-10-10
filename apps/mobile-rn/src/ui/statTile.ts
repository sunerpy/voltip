// The colours of the 记录 tab's stat tiles, the phone's side of the desktop's `stat-tile-*`
// (packages/ui/src/tokens.css; user decision 2026-10-10, Penpot 「Voltip 统计卡」 option C with
// total ①): today, this week and this month mix their own hue (the accent, ok, warning) into the
// card; the total is the inverse (ink) surface lit by the accent in a light theme, and the accent's
// container fading into the card in a dark one. Everything comes from the theme, so the four
// themes, the chosen accent and the wallpaper's colour all carry through.
import type { AppTheme } from "../theme/themes";

export type StatSpan = "today" | "week" | "month" | "total";

export interface StatTileColors {
  /** The card's fill; absent for the total, which draws `gradient`. */
  background?: string;
  border: string;
  /** The icon block and the icon on it. */
  chip: string;
  icon: string;
  /** The label and the figure, the units and the caption, the count. */
  fg: string;
  muted: string;
  subtle: string;
  gradient?: { from: string; to: string; glow: string; glowOpacity: number };
}

function channels(hex: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m?.[1]) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** `a` with `t` (0…1) of `b` mixed in, as `#rrggbb`; `a` unchanged when either is not one. */
export function mix(a: string, b: string, t: number): string {
  const [x, y] = [channels(a), channels(b)];
  if (!x || !y) return a;
  return `#${x
    .map((v, i) =>
      Math.round(v + ((y[i] ?? v) - v) * t)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

/** `hex` at `alpha` as `rgba(…)`; `hex` unchanged when it is not one. */
export function alpha(hex: string, a: number): string {
  const c = channels(hex);
  return c ? `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${a})` : hex;
}

export function statTileColors(theme: AppTheme, span: StatSpan): StatTileColors {
  const c = theme.colors;
  const v = theme.voltip;
  if (span === "total") {
    if (theme.dark) {
      return {
        border: mix(c.surface, c.primary, 0.35),
        chip: c.primary,
        icon: c.onPrimary,
        fg: c.onSurface,
        muted: c.onSurfaceVariant,
        subtle: v.subtle,
        gradient: { from: c.primaryContainer, to: c.surface, glow: c.primary, glowOpacity: 0.45 },
      };
    }
    return {
      border: c.inverseSurface,
      chip: alpha(c.inverseOnSurface, 0.12),
      icon: c.inversePrimary,
      fg: c.inverseOnSurface,
      muted: mix(c.inverseOnSurface, c.inverseSurface, 0.3),
      subtle: mix(c.inverseOnSurface, c.inverseSurface, 0.45),
      gradient: {
        from: mix(c.inverseSurface, c.primary, 0.12),
        to: c.inverseSurface,
        glow: c.primary,
        glowOpacity: 0.55,
      },
    };
  }
  const hue = { today: c.primary, week: v.ok, month: v.warning }[span];
  const chip = { today: c.primaryContainer, week: v.okSoft, month: v.warningSoft }[span];
  const icon = { today: c.primary, week: v.okText, month: v.warning }[span];
  const [tint, edge] = theme.dark ? [0.08, 0.28] : [0.05, 0.22];
  return {
    background: mix(c.surface, hue, tint),
    border: mix(c.surface, hue, edge),
    chip,
    icon,
    fg: c.onSurface,
    muted: c.onSurfaceVariant,
    subtle: v.subtle,
  };
}
