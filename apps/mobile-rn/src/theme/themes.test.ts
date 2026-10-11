// The "简约中性" schemes (docs/mobile-rn.md §5): every theme keeps MD3's contrast, the surfaces stay
// near-neutral whatever the seed, and the accent is the wallpaper's only while the appearance
// follows the system.
import { Hct, argbFromHex } from "@material/material-color-utilities";
import { THEME_IDS } from "@voltip/shared";

import { VOLTIP_SEED, appTheme, auroraOf, isGlassTheme, themeId, themeSeed } from "./themes";

/** WCAG 2's relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const channel = (i: number) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

/** WCAG 2's contrast ratio of two `#rrggbb` colours. */
function contrast(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const hct = (hex: string) => Hct.fromInt(argbFromHex(hex));

/** `#rrggbb` or `rgba(r, g, b, a)` → channels and alpha. */
function rgba(colour: string): [number, number, number, number] {
  const m = /^rgba\((\d+), (\d+), (\d+), ([\d.]+)\)$/.exec(colour);
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3]), Number(m[4])];
  return [1, 3, 5].map((i) => parseInt(colour.slice(i, i + 2), 16)).concat(1) as never;
}

/** `top` (with its alpha, or `alpha`) laid over the opaque `bottom`. */
function over(top: string, bottom: string, alpha?: number): string {
  const [r, g, b, a] = rgba(top);
  const [R, G, B] = rgba(bottom);
  const k = alpha ?? a;
  const mix = (x: number, y: number) =>
    Math.round(x * k + y * (1 - k))
      .toString(16)
      .padStart(2, "0");
  return `#${mix(r, R)}${mix(g, G)}${mix(b, B)}`;
}

/** The angle between two hues, 0–180. */
function hueDistance(a: number, b: number): number {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
}

const WALLPAPERS = ["#4c8b5f", "#b5446e", "#c08a2e", VOLTIP_SEED];

describe("the 简约中性 themes", () => {
  it.each(THEME_IDS)("%s keeps MD3's contrast for text, the accent and the status shades", (id) => {
    const t = appTheme(id);
    const c = t.colors;
    expect(contrast(c.onSurface, c.background)).toBeGreaterThanOrEqual(7);
    expect(contrast(c.onSurface, c.surface)).toBeGreaterThanOrEqual(7);
    expect(contrast(c.onSurfaceVariant, c.surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(t.voltip.subtle, c.surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(c.onPrimary, c.primary)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(c.onPrimaryContainer, c.primaryContainer)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(c.onSecondaryContainer, c.secondaryContainer)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(t.voltip.okText, t.voltip.okSoft)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(t.voltip.danger, t.voltip.dangerSoft)).toBeGreaterThanOrEqual(4.5);
    // A field's outline stands off the card (WCAG's 3:1 for a control's boundary).
    expect(contrast(c.outline, c.surface)).toBeGreaterThanOrEqual(3);
    // The card is not the page, and its hairline shows on it.
    expect(c.surface).not.toBe(c.background);
    expect(contrast(c.outlineVariant, c.surface)).toBeGreaterThan(1.2);
  });

  it.each(WALLPAPERS)(
    "draws near-neutral surfaces from any seed (%s), with the accent at its hue",
    (seed) => {
      for (const id of ["light", "dark"] as const) {
        const c = appTheme(id, seed).colors;
        // The page and the cards on the neutral palette (chroma 2), the hairlines on its variant (4).
        for (const surface of [c.background, c.surface])
          expect(hct(surface).chroma).toBeLessThan(3);
        expect(hct(c.outlineVariant).chroma).toBeLessThan(5);
        expect(hueDistance(hct(c.primary).hue, hct(seed).hue)).toBeLessThan(15);
        expect(contrast(c.onPrimary, c.primary)).toBeGreaterThanOrEqual(4.5);
      }
    },
  );

  it("follows the wallpaper only while the appearance follows the system", () => {
    const wallpaper = "#4c8b5f";
    expect(themeSeed({ theme: "light", follow_system_theme: true }, wallpaper)).toBe(wallpaper);
    // Before Android 12 the system has no colour to give: Voltip's own.
    expect(themeSeed({ theme: "light", follow_system_theme: true }, null)).toBe(VOLTIP_SEED);
    // A theme picked by hand keeps its own accent.
    expect(themeSeed({ theme: "dark", follow_system_theme: false }, wallpaper)).toBe(VOLTIP_SEED);
    expect(themeSeed({ theme: "warm", follow_system_theme: false }, wallpaper)).not.toBe(wallpaper);
    // User decision 2026-10-11: following the system switches between 澄空 and 夜空.
    expect(themeId({ theme: "warm", follow_system_theme: true }, true)).toBe("glass_dark");
    expect(themeId({ theme: "warm", follow_system_theme: true }, false)).toBe("glass");
    expect(themeId({ theme: "light", follow_system_theme: false }, true)).toBe("light");
  });

  it("draws 澄空 on a blue-tinted light page and 夜空 on a night one, both with Voltip's accent", () => {
    const clear = appTheme("glass");
    const night = appTheme("glass_dark");
    expect([clear.dark, night.dark]).toEqual([false, true]);
    expect(isGlassTheme("glass") && isGlassTheme("glass_dark")).toBe(true);
    expect(isGlassTheme("light") || isGlassTheme("dark")).toBe(false);
    expect(hueDistance(hct(clear.colors.background).hue, hct("#eef2fa").hue)).toBeLessThan(25);
    expect(hct(clear.colors.background).tone).toBeGreaterThan(90);
    expect(hct(night.colors.background).tone).toBeLessThan(10);
    for (const t of [clear, night]) {
      expect(contrast(t.colors.onSurface, t.colors.surface)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(t.colors.onSurfaceVariant, t.colors.surface)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(t.colors.onPrimary, t.colors.primary)).toBeGreaterThanOrEqual(4.5);
    }
  });

  // Board 03's aurora at its own 40–85 % put muted text on the page under 3:1 where the blue light
  // is strongest; the phone's is as faint as the desktop's.
  it.each(["glass", "glass_dark"] as const)(
    "regression: %s keeps 4.5:1 for text on the page where each light of the aurora is strongest, and on a card over it (the quietest text sits on cards only)",
    (id) => {
      const t = appTheme(id);
      const spots = auroraOf(id);
      expect(spots).toHaveLength(4);
      for (const spot of spots) {
        const lit = over(spot.color, t.colors.background, spot.opacity);
        const card = over(t.voltip.card, lit);
        // On the page: titles, the line under them, section titles, links (全部记录).
        for (const text of [t.colors.onBackground, t.colors.onSurfaceVariant, t.colors.primary])
          expect(contrast(text, lit)).toBeGreaterThanOrEqual(4.5);
        // On a card the quietest text too (a result's time, a tile's caption).
        for (const text of [t.colors.onSurface, t.colors.onSurfaceVariant, t.voltip.subtle])
          expect(contrast(text, card)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(t.colors.primary, card)).toBeGreaterThanOrEqual(4.5);
      }
    },
  );

  it.each(THEME_IDS)("%s: the talk button's icon keeps 3:1 on it, held or not", (id) => {
    const t = appTheme(id);
    expect(contrast(t.voltip.onHero, t.voltip.hero)).toBeGreaterThanOrEqual(3);
    expect(contrast(t.voltip.onHero, t.voltip.heroPressed)).toBeGreaterThanOrEqual(3);
    expect(t.voltip.glass).toBe(isGlassTheme(id));
  });

  it.each(["glass", "glass_dark"] as const)(
    "%s puts translucent cards with board 03's corners on the aurora, under an opaque dock",
    (id) => {
      const t = appTheme(id);
      expect(t.voltip.card).toMatch(/^rgba\(255, 255, 255, 0\.\d+\)$/);
      expect(t.voltip.radius).toEqual({ card: 20, hero: 28 });
      // Regression (Pixel 8, 2026-10-11): nothing blurs what scrolls under the dock, and at 96 %
      // the text under it showed through.
      expect(t.voltip.dock).toMatch(/^#[0-9a-f]{6}$/);
    },
  );

  it.each(["light", "dark", "warm", "graphite"] as const)(
    "%s keeps its flat page: no aurora, cards on the surface with the classic corners",
    (id) => {
      const t = appTheme(id);
      expect(auroraOf(id)).toEqual([]);
      expect(t.voltip.card).toBe(t.colors.surface);
      expect(t.voltip.radius).toEqual({ card: 14, hero: 14 });
    },
  );

  it("keeps 暖纸 cream and 石墨 a grey lifted off black", () => {
    const warm = hct(appTheme("warm").colors.background);
    expect(warm.chroma).toBeGreaterThan(3);
    expect(hueDistance(warm.hue, hct("#f5efe4").hue)).toBeLessThan(20);
    const graphite = hct(appTheme("graphite").colors.background).tone;
    const dark = hct(appTheme("dark").colors.background).tone;
    expect(graphite).toBeGreaterThan(dark + 6);
    expect(appTheme("graphite").dark).toBe(true);
  });
});
