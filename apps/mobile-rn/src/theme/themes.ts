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

/** Voltip's accent (packages/ui/src/tokens.css): the seed of 澄空, 夜空, 明亮, 暗黑 and 石墨. */
export const VOLTIP_SEED = "#339cff";
/** 暖纸's accent, terracotta (tokens.css `--accent` of the warm theme). */
const WARM_SEED = "#b85c38";
/** 暖纸's paper: the hue (and a little of the chroma) of its cream. */
const PAPER = "#f5efe4";
/** 澄空's canvas and 夜空's night (tokens.css `--canvas`): the hue of the glass themes' neutrals. */
const CLEAR_SKY = "#eef2fa";
const NIGHT_SKY = "#0b1020";

/** Whether `id` is one of the glass pair (an aurora behind translucent cards, docs/mobile-rn.md). */
export function isGlassTheme(id: ThemeId): boolean {
  return id === "glass" || id === "glass_dark";
}

/** One soft light of the glass pair's aurora: a radial gradient from `color` at `opacity` in its
 *  centre to nothing at its edge. The centre and the radii are fractions of the page's width
 *  (`cx`, `rx`) and height (`cy`, `ry`). */
export interface AuroraSpot {
  color: string;
  opacity: number;
  cx: number;
  cy: number;
  rx: number;
  ry: number;
}

/** Penpot 「澄空 Glass · Voltip」 board 03 (390 × 844): blue and violet over the title, mint and
 *  rose low down. As faint as the desktop's (tokens.css `--canvas-image`), not the board's 40–85 %,
 *  so that muted text on the page keeps 4.5:1 where the light is strongest (themes.test.ts). */
const AURORA_SPOTS = [
  { cx: 0.22, cy: 0.09, rx: 0.6, ry: 0.25 },
  { cx: 0.92, cy: 0.05, rx: 0.51, ry: 0.23 },
  { cx: 0.9, cy: 0.83, rx: 0.54, ry: 0.24 },
  { cx: 0.05, cy: 0.64, rx: 0.41, ry: 0.19 },
] as const;
const CLEAR_SKY_LIGHTS = [
  ["#9cc3ff", 0.35],
  ["#d7c2ff", 0.4],
  ["#9ff0dc", 0.4],
  ["#ffd6e8", 0.35],
] as const;
const NIGHT_SKY_LIGHTS = [
  ["#2b5cff", 0.22],
  ["#7c4dff", 0.2],
  ["#14b8a6", 0.16],
  ["#db2777", 0.12],
] as const;

/** The aurora behind the tab pages of theme `id`: none outside the glass pair. */
export function auroraOf(id: ThemeId): readonly AuroraSpot[] {
  if (!isGlassTheme(id)) return [];
  const lights = id === "glass" ? CLEAR_SKY_LIGHTS : NIGHT_SKY_LIGHTS;
  return AURORA_SPOTS.map((spot, i) => {
    const [color, opacity] = lights[i] ?? lights[0];
    return { ...spot, color, opacity };
  });
}

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
  /** The glass pair: an aurora behind the tab pages and translucent cards on it. */
  glass: boolean;
  aurora: readonly AuroraSpot[];
  /** A card on the page (every Section, the talk card, a result row) and its edge: translucent
   *  white on the glass pair's aurora, the surface with a hairline elsewhere. */
  card: string;
  cardEdge: string;
  /** Quiet controls on the page: chips and round buttons. */
  control: string;
  controlEdge: string;
  /** The segmented control: its track and the selected segment on it. */
  track: string;
  thumb: string;
  /** The tab dock floating over the page: nearly opaque, as nothing blurs what scrolls under it. */
  dock: string;
  dockEdge: string;
  /** The tint of the soft shadows under cards, the dock and the talk button. */
  shadow: string;
  /** The talk button and its icon; darker while held. */
  hero: string;
  heroPressed: string;
  onHero: string;
  /** Corner radii: board 03's for the glass pair, the classic 14 elsewhere. */
  radius: { card: number; hero: number };
}

export type AppTheme = MD3Theme & { voltip: VoltipColors; id: ThemeId };

/** The theme the settings ask for: the system's light or dark while following it, as on the desktop. */
export function themeId(
  settings: Pick<Settings, "theme" | "follow_system_theme">,
  systemDark: boolean,
): ThemeId {
  // User decision 2026-10-11: following the system switches between 澄空 and 夜空.
  if (settings.follow_system_theme) return systemDark ? "glass_dark" : "glass";
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
    case "glass": {
      const sky = Hct.fromInt(argbFromHex(CLEAR_SKY));
      return {
        dark: false,
        neutral: [sky.hue, 6],
        page: 95,
        card: 100,
        inset: 96,
        levels: [100, 100, 100, 98, 96],
        hairline: 88,
        outline: 50,
        fg: 10,
        fgMuted: 38,
        subtle: 46,
      };
    }
    case "glass_dark": {
      const night = Hct.fromInt(argbFromHex(NIGHT_SKY));
      return {
        dark: true,
        neutral: [night.hue, 10],
        page: 5,
        card: 12,
        inset: 16,
        levels: [12, 14, 17, 20, 22],
        hairline: 24,
        outline: 60,
        fg: 94,
        fgMuted: 72,
        subtle: 66,
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
  const primary = role((m) => m.primary());
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
    primary,
    onPrimary: role((m) => m.onPrimary()),
    // The talk button: the accent itself on a light page; on a dark one a mid tone that keeps a
    // white icon (MD3's dark primary is too pale for it), darker while held (board 04).
    hero: s.dark ? hex(fidelity.primaryPalette.tone(55)) : primary,
    heroPressed: hex(fidelity.primaryPalette.tone(s.dark ? 45 : 30)),
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
      hero: c.hero,
      heroPressed: c.heroPressed,
      onHero: "#ffffff",
      ...materials(id, c),
    },
  };
}

/** The page's materials (board 03; 夜空 by board 07's recolouring): glass on the aurora for the
 *  pair, the classic surface and hairline elsewhere. */
function materials(id: ThemeId, c: ReturnType<typeof schemeColours>) {
  const aurora = auroraOf(id);
  if (id === "glass")
    return {
      glass: true,
      aurora,
      card: "rgba(255, 255, 255, 0.62)",
      cardEdge: "rgba(255, 255, 255, 0.9)",
      control: "rgba(255, 255, 255, 0.6)",
      controlEdge: "rgba(255, 255, 255, 0.9)",
      track: "rgba(255, 255, 255, 0.55)",
      thumb: "#ffffff",
      dock: "rgba(255, 255, 255, 0.96)",
      dockEdge: "rgba(255, 255, 255, 0.9)",
      shadow: "#2a3366",
      radius: { card: 20, hero: 28 },
    };
  if (id === "glass_dark")
    return {
      glass: true,
      aurora,
      card: "rgba(255, 255, 255, 0.06)",
      cardEdge: "rgba(255, 255, 255, 0.1)",
      control: "rgba(255, 255, 255, 0.06)",
      controlEdge: "rgba(255, 255, 255, 0.1)",
      track: "rgba(255, 255, 255, 0.05)",
      thumb: "rgba(255, 255, 255, 0.14)",
      dock: "rgba(26, 31, 46, 0.97)",
      dockEdge: "rgba(255, 255, 255, 0.1)",
      shadow: "#000000",
      radius: { card: 20, hero: 28 },
    };
  return {
    glass: false,
    aurora,
    card: c.card,
    cardEdge: c.hairline,
    control: c.card,
    controlEdge: c.hairline,
    track: c.inset,
    thumb: c.card,
    dock: c.card,
    dockEdge: c.hairline,
    shadow: "#000000",
    radius: { card: 14, hero: 14 },
  };
}
