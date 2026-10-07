// The "简约中性" schemes (docs/mobile-rn.md §5): every theme keeps MD3's contrast, the surfaces stay
// near-neutral whatever the seed, and the accent is the wallpaper's only while the appearance
// follows the system.
import { Hct, argbFromHex } from "@material/material-color-utilities";
import { THEME_IDS } from "@voltip/shared";

import { VOLTIP_SEED, appTheme, themeId, themeSeed } from "./themes";

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
    expect(themeId({ theme: "warm", follow_system_theme: true }, true)).toBe("dark");
  });

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
