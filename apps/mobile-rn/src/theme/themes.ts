// Voltip's themes as Material Design 3 colour schemes, in the "简约中性" style the user picked on
// 2026-10-07 (docs/mobile-rn.md §5): near-neutral surfaces, white cards with a hairline, and one
// accent, Voltip's ink blue or, while the appearance follows the system on Android 12+, the
// wallpaper's colour. Material Color Utilities (the library Android's own dynamic colour comes
// from) derives every role from the seed, so the containers, the selected states and the text on
// them keep MD3's contrast whatever the seed is:
// - the accent from SchemeFidelity, which keeps the seed's own chroma, so a vivid seed stays vivid;
//   its container (notices, badges, the FAB, the hold button while held) is the soft tone of the
//   same palette, as in MD3's standard schemes, not Fidelity's seed-coloured one;
// - the surfaces and outlines from a neutral palette at the seed's hue with almost no chroma (the
//   same as SchemeNeutral's), warmer and creamier for 暖纸, lifted off black for 石墨;
// - ok / warning / danger nudged towards the seed's hue (MD3's harmonisation), so they sit with it.
import {
  Blend,
  type DynamicColor,
  DynamicScheme,
  Hct,
  MaterialDynamicColors,
  SchemeFidelity,
  TonalPalette,
  argbFromHex,
  hexFromArgb,
} from "@material/material-color-utilities";
import type { Settings, ThemeId } from "@voltip/shared";
import { MD3DarkTheme, MD3LightTheme, type MD3Theme } from "react-native-paper";

/** Voltip's accent (packages/ui/src/tokens.css): the seed of 明亮, 暗黑 and 石墨. */
export const VOLTIP_SEED = "#339cff";
/** 暖纸's accent, terracotta (tokens.css `--accent` of the warm theme). */
const WARM_SEED = "#b85c38";
/** 暖纸's paper: the hue (and a little of the chroma) of its cream. */
const PAPER = "#f5efe4";

/** The colours the screens use beyond MD3's roles (status lamps, the meter, notices). */
export interface VoltipColors {
  ok: string;
  okSoft: string;
  okText: string;
  warning: string;
  warningSoft: string;
  danger: string;
  dangerSoft: string;
  /** The meter's peak and other one-off marks in the accent. */
  accent: string;
  /** Secondary labels (stat captions, empty-state icons); text contrast on cards. */
  subtle: string;
  inset: string;
  hairline: string;
}

export type AppTheme = MD3Theme & { voltip: VoltipColors; id: ThemeId };

/** The theme the settings ask for: the system's light or dark while following it, as on the desktop. */
export function themeId(
  settings: Pick<Settings, "theme" | "follow_system_theme">,
  systemDark: boolean,
): ThemeId {
  if (settings.follow_system_theme) return systemDark ? "dark" : "light";
  return settings.theme;
}

/** The seed a theme is drawn from: the wallpaper's while following the system (Android 12+ hands it
 *  over, older ones have none), Voltip's own otherwise. 暖纸 always keeps its terracotta. */
export function themeSeed(
  settings: Pick<Settings, "theme" | "follow_system_theme">,
  wallpaper: string | null,
): string {
  if (settings.follow_system_theme && wallpaper !== null) return wallpaper;
  return settings.follow_system_theme || settings.theme !== "warm" ? VOLTIP_SEED : WARM_SEED;
}

const roles = new MaterialDynamicColors();
const hex = (argb: number) => hexFromArgb(argb);

type Levels<T> = readonly [T, T, T, T, T];

/** The surfaces of one theme, as tones of its neutral palettes. */
interface Surfaces {
  dark: boolean;
  /** Hue and chroma of the neutral palette (the variant palette has a little more). */
  neutral: [number, number];
  page: number;
  card: number;
  inset: number;
  /** Menus, dialogs and sheets: Paper's elevation levels 1–5. */
  levels: Levels<number>;
  hairline: number;
  outline: number;
  fg: number;
  fgMuted: number;
  subtle: number;
}

function surfaces(id: ThemeId, seed: Hct): Surfaces {
  const hue = seed.hue;
  switch (id) {
    case "light":
      return {
        dark: false,
        neutral: [hue, 2],
        page: 96,
        card: 100,
        inset: 94,
        levels: [100, 100, 100, 98, 96],
        hairline: 80,
        outline: 50,
        fg: 10,
        fgMuted: 30,
        subtle: 45,
      };
    case "dark":
      return {
        dark: true,
        neutral: [hue, 2],
        page: 6,
        card: 10,
        inset: 17,
        levels: [10, 12, 17, 20, 22],
        hairline: 30,
        outline: 60,
        fg: 90,
        fgMuted: 80,
        subtle: 65,
      };
    case "warm": {
      const paper = Hct.fromInt(argbFromHex(PAPER));
      return {
        dark: false,
        neutral: [paper.hue, 8],
        page: 95,
        card: 98.5,
        inset: 92,
        levels: [98.5, 98.5, 98.5, 97, 95],
        hairline: 82,
        outline: 50,
        fg: 12,
        fgMuted: 32,
        subtle: 45,
      };
    }
    case "graphite":
      return {
        dark: true,
        neutral: [hue, 4],
        page: 15,
        card: 20,
        inset: 26,
        levels: [20, 22, 26, 28, 30],
        hairline: 34,
        outline: 62,
        fg: 93,
        fgMuted: 78,
        subtle: 68,
      };
  }
}

/** `hex` nudged towards the seed's hue, as MD3 harmonises custom colours, in a palette of its own. */
function harmonized(colour: string, seed: Hct): TonalPalette {
  return TonalPalette.fromInt(Blend.harmonize(argbFromHex(colour), seed.toInt()));
}

/** The MD3 colours of theme `id` drawn from `seed` (a `#rrggbb`). */
export function schemeColours(id: ThemeId, seed: string = VOLTIP_SEED) {
  const source = Hct.fromInt(argbFromHex(seed));
  const s = surfaces(id, source);
  const fidelity = new SchemeFidelity(source, s.dark, 0, "2021");
  const neutral = TonalPalette.fromHueAndChroma(s.neutral[0], s.neutral[1]);
  const neutralVariant = TonalPalette.fromHueAndChroma(s.neutral[0], s.neutral[1] + 2);
  // The accent's roles as Fidelity has them, everything else on the neutral palettes; the selected
  // states (segments, chips) a quiet tint of the accent's hue.
  const scheme = new DynamicScheme({
    sourceColorHct: source,
    variant: fidelity.variant,
    contrastLevel: 0,
    isDark: s.dark,
    specVersion: "2021",
    primaryPalette: fidelity.primaryPalette,
    secondaryPalette: TonalPalette.fromHueAndChroma(source.hue, 8),
    tertiaryPalette: fidelity.tertiaryPalette,
    neutralPalette: neutral,
    neutralVariantPalette: neutralVariant,
  });
  const role = (pick: (colours: MaterialDynamicColors) => DynamicColor) =>
    hex(pick(roles).getArgb(scheme));
  const n = (tone: number) => hex(neutral.tone(tone));
  const nv = (tone: number) => hex(neutralVariant.tone(tone));
  const ok = harmonized("#2e9e62", source);
  const warning = harmonized("#b7791f", source);
  const tone = (palette: TonalPalette, light: number, dark: number) =>
    hex(palette.tone(s.dark ? dark : light));
  return {
    dark: s.dark,
    page: n(s.page),
    card: n(s.card),
    inset: n(s.inset),
    levels: [
      n(s.levels[0]),
      n(s.levels[1]),
      n(s.levels[2]),
      n(s.levels[3]),
      n(s.levels[4]),
    ] as const,
    hairline: nv(s.hairline),
    outline: nv(s.outline),
    fg: n(s.fg),
    fgMuted: nv(s.fgMuted),
    subtle: nv(s.subtle),
    primary: role((m) => m.primary()),
    onPrimary: role((m) => m.onPrimary()),
    primaryContainer: hex(fidelity.primaryPalette.tone(s.dark ? 30 : 90)),
    onPrimaryContainer: hex(fidelity.primaryPalette.tone(s.dark ? 90 : 10)),
    inversePrimary: role((m) => m.inversePrimary()),
    secondary: role((m) => m.secondary()),
    onSecondary: role((m) => m.onSecondary()),
    secondaryContainer: role((m) => m.secondaryContainer()),
    onSecondaryContainer: role((m) => m.onSecondaryContainer()),
    error: role((m) => m.error()),
    onError: role((m) => m.onError()),
    errorContainer: role((m) => m.errorContainer()),
    onErrorContainer: role((m) => m.onErrorContainer()),
    inverseSurface: n(s.dark ? 90 : 20),
    inverseOnSurface: n(s.dark ? 20 : 95),
    ok: tone(ok, 45, 78),
    okSoft: tone(ok, 94, 25),
    okText: tone(ok, 35, 85),
    warning: tone(warning, 55, 78),
    warningSoft: tone(warning, 94, 25),
  };
}

/** A theme's tile on 外观与语言: its page, a card on it, text and the accent. */
export function themePreview(id: ThemeId) {
  const c = schemeColours(id, id === "warm" ? WARM_SEED : VOLTIP_SEED);
  return {
    page: c.page,
    card: c.card,
    border: c.hairline,
    fg: c.fg,
    fgMuted: c.fgMuted,
    accent: c.primary,
  };
}

/** Theme `id` for Paper, React Navigation and the system bars, drawn from `seed`. */
export function appTheme(
  id: ThemeId,
  seed: string = id === "warm" ? WARM_SEED : VOLTIP_SEED,
): AppTheme {
  const c = schemeColours(id, seed);
  const base = c.dark ? MD3DarkTheme : MD3LightTheme;
  const fgRgb =
    c.fg
      .slice(1)
      .match(/../g)
      ?.map((h) => parseInt(h, 16))
      .join(", ") ?? "0, 0, 0";
  return {
    ...base,
    id,
    dark: c.dark,
    roundness: 4,
    colors: {
      ...base.colors,
      primary: c.primary,
      onPrimary: c.onPrimary,
      primaryContainer: c.primaryContainer,
      onPrimaryContainer: c.onPrimaryContainer,
      secondary: c.secondary,
      onSecondary: c.onSecondary,
      secondaryContainer: c.secondaryContainer,
      onSecondaryContainer: c.onSecondaryContainer,
      tertiary: c.okText,
      onTertiary: c.card,
      tertiaryContainer: c.okSoft,
      onTertiaryContainer: c.okText,
      // The page, and the cards on it (every Section is a card).
      background: c.page,
      onBackground: c.fg,
      surface: c.card,
      onSurface: c.fg,
      surfaceVariant: c.inset,
      onSurfaceVariant: c.fgMuted,
      surfaceDisabled: `rgba(${fgRgb}, 0.12)`,
      onSurfaceDisabled: `rgba(${fgRgb}, 0.38)`,
      outline: c.outline,
      outlineVariant: c.hairline,
      error: c.error,
      onError: c.onError,
      errorContainer: c.errorContainer,
      onErrorContainer: c.onErrorContainer,
      inverseSurface: c.inverseSurface,
      inverseOnSurface: c.inverseOnSurface,
      inversePrimary: c.inversePrimary,
      shadow: "#000000",
      scrim: "#000000",
      backdrop: c.dark ? "rgba(0, 0, 0, 0.5)" : `rgba(${fgRgb}, 0.4)`,
      elevation: {
        level0: "transparent",
        level1: c.levels[0],
        level2: c.levels[1],
        level3: c.levels[2],
        level4: c.levels[3],
        level5: c.levels[4],
      },
    },
    voltip: {
      ok: c.ok,
      okSoft: c.okSoft,
      okText: c.okText,
      warning: c.warning,
      warningSoft: c.warningSoft,
      danger: c.error,
      dangerSoft: c.errorContainer,
      accent: c.primary,
      subtle: c.subtle,
      inset: c.inset,
      hairline: c.hairline,
    },
  };
}
