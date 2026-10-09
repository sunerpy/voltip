import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  ACCENT_IDS,
  FONT_SIZE_MAX,
  applyAppearance,
  applyTheme,
  isAccentId,
  isThemeId,
  readTheme,
  resolveTheme,
  systemPrefersDark,
  systemPrefersReducedMotion,
} from "./theme";

const tokens = readFileSync(resolve(__dirname, "tokens.css"), "utf8");

const ROLES = [
  "canvas",
  "surface",
  "inset",
  "inset2",
  "border",
  "borderStrong",
  "fg",
  "fgMuted",
  "fgSubtle",
  "primary",
  "primaryFg",
  "accent",
  "accentFg",
  "accentSoft",
  "accentText",
  "accentTextHover",
  "thumb",
  "ok",
  "okSoft",
  "okText",
  "danger",
  "dangerSoft",
  "warning",
  "warningSoft",
  "info",
  "infoSoft",
  "nav",
  "navActive",
  "ledOff",
  "ledOn",
  "ledPeak",
  "pillBg",
  "pillBorder",
  "pillFg",
  "pillMuted",
  "wave",
  "desktop",
  "desktopField",
  "keycapBg",
  "keycapBorder",
  "scrim",
  "diffAdd",
  "diffDel",
  "mark",
];

function kebab(role: string): string {
  return role.replaceAll(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
}

function themeBlock(id: string): string {
  const start = tokens.indexOf(`[data-theme="${id}"] {`);
  const end = tokens.indexOf("}", start);
  return tokens.slice(start, end);
}

describe("tokens.css", () => {
  it.each(["light", "dark", "warm", "graphite"])(
    "theme %s carries every role as a six-digit hex",
    (id) => {
      const block = themeBlock(id);
      for (const role of ROLES) {
        const match = new RegExp(`--${kebab(role)}: (#[0-9A-Fa-f]{6});`).exec(block);
        if (!match) throw new Error(`palette missing ${id}.${role}`);
      }
      for (const shadow of ["shadow-pop", "shadow-pill", "shadow-win"])
        expect(block).toContain(`--${shadow}:`);
    },
  );

  it("the four themes differ from each other on the canvas role", () => {
    const canvases = ["light", "dark", "warm", "graphite"].map((id) =>
      /--canvas: (#[0-9A-Fa-f]{6});/.exec(themeBlock(id))?.[1]?.toLowerCase(),
    );
    expect(new Set(canvases).size).toBe(4);
  });

  it("exposes every role to Tailwind through @theme inline and never hardcodes hex in components", () => {
    for (const role of ROLES)
      expect(tokens).toContain(`--color-${kebab(role)}: var(--${kebab(role)});`);
    // regression (Windows test 2026-09-24): fonts came from Google Fonts, so an offline or
    // firewalled machine fell back to the system UI font and the app no longer matched its design.
    // The three families are bundled (fontsource variable builds) and Windows / macOS CJK fonts
    // are named as fallbacks.
    expect(tokens).toContain('@import "@fontsource-variable/instrument-sans/index.css";');
    expect(tokens).toContain('@import "@fontsource-variable/jetbrains-mono/index.css";');
    expect(tokens).toContain('@import "@fontsource-variable/noto-sans-sc/index.css";');
    expect(tokens).toMatch(
      /--font-ui:\s*"Instrument Sans Variable", "Instrument Sans", "Noto Sans SC Variable"/,
    );
    expect(tokens).toMatch(/--font-mono:\s*"JetBrains Mono Variable", "JetBrains Mono"/);
    expect(tokens).toContain('"Microsoft YaHei UI"');
    expect(tokens).toContain('"PingFang SC"');
    expect(tokens).not.toContain("googleapis");
    expect(tokens).toContain("--radius-10: 10px");
    expect(tokens).toContain("--radius-pill: 999px");
  });
});

const THEMES = ["light", "dark", "warm", "graphite"] as const;
const DARK_THEMES: ReadonlySet<string> = new Set(["dark", "graphite"]);

function roleIn(block: string, role: string): string | undefined {
  return new RegExp(`--${kebab(role)}: (#[0-9A-Fa-f]{6});`).exec(block)?.[1]?.toLowerCase();
}

/** The roles of `theme` with `accent` chosen: the theme's block, then the accent's own block. */
function palette(theme: string, accent: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const role of ROLES) {
    const value = roleIn(themeBlock(theme), role);
    if (value !== undefined) out[role] = value;
  }
  if (accent !== "default") {
    const start = tokens.indexOf(`[data-accent="${accent}"][data-theme="${theme}"],`);
    if (start < 0) throw new Error(`no ${accent} block for ${theme}`);
    const block = tokens.slice(start, tokens.indexOf("}", start));
    for (const role of [
      "accent",
      "accentFg",
      "accentSoft",
      "accentText",
      "accentTextHover",
      "ledPeak",
    ]) {
      const value = roleIn(block, role);
      if (value === undefined) throw new Error(`${accent} × ${theme} lacks ${role}`);
      out[role] = value;
    }
  }
  return out;
}

function luminance(color: string): number {
  const channel = (i: number) => {
    const c = Number.parseInt(color.slice(1 + 2 * i, 3 + 2 * i), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
}

function contrast(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

describe("accent colours (Codex's blue, ChatGPT's accent choices)", () => {
  it("the blue themes default to Codex's #339cff; the warm theme keeps its own accent", () => {
    // User decisions 2026-09-29: Codex's blue for the blue roles, the desktop app's #339cff.
    for (const theme of ["light", "dark", "graphite"])
      expect(palette(theme, "default").accent).toBe("#339cff");
    expect(palette("warm", "default").accent).toBe("#b85c38");
    expect(tokens).toMatch(/:focus-visible \{\s*outline: 2px solid var\(--accent-text\);/);
  });

  it("every accent choice takes ChatGPT's light value in the light themes and its dark value in the dark ones", () => {
    const values: Record<string, readonly [string, string]> = {
      blue: ["#0285ff", "#339cff"],
      green: ["#04b84c", "#40c977"],
      yellow: ["#ffc300", "#ffd240"],
      pink: ["#ff66ad", "#ff8cc1"],
      orange: ["#fb6a22", "#ff8549"],
      purple: ["#924ff7", "#ad7bf9"],
    };
    expect([...ACCENT_IDS]).toEqual(["default", ...Object.keys(values), "ink"]);
    for (const theme of THEMES) {
      for (const [accent, [light, dark]] of Object.entries(values)) {
        expect(palette(theme, accent).accent, `${accent} × ${theme}`).toBe(
          DARK_THEMES.has(theme) ? dark : light,
        );
      }
      // 墨色 is the theme's own ink.
      expect(palette(theme, "ink").accent).toBe(palette(theme, "default").fg);
    }
  });

  it.each(THEMES)("text on and around every accent reads in the %s theme", (theme) => {
    for (const accent of ACCENT_IDS) {
      const p = palette(theme, accent);
      const at = `${accent} × ${theme}`;
      const need = (role: string) => {
        const value = p[role];
        if (value === undefined) throw new Error(`${at} lacks ${role}`);
        return value;
      };
      // Labels on an accent fill (the edit tag, the inserted check): 4.5:1.
      expect(contrast(need("accentFg"), need("accent")), `${at} accent-fg`).toBeGreaterThanOrEqual(
        4.5,
      );
      // Links and accent text, and their hover, on every surface they sit on: 4.5:1. The focus ring
      // is accent-text too, so it clears the 3:1 of a focus indicator.
      for (const background of ["surface", "canvas", "inset", "inset2", "accentSoft"]) {
        for (const role of ["accentText", "accentTextHover"]) {
          expect(
            contrast(need(role), need(background)),
            `${at} ${role} on ${background}`,
          ).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
    // The information colour stays Codex's blue whatever the accent.
    const base = palette(theme, "default");
    expect(contrast(base.info ?? "", base.surface ?? ""), `${theme} info`).toBeGreaterThanOrEqual(
      4.5,
    );
    expect(
      contrast(base.info ?? "", base.infoSoft ?? ""),
      `${theme} info on info-soft`,
    ).toBeGreaterThanOrEqual(4.5);
  });

  it("an element that names its own accent is not overridden by the page's", () => {
    // The appearance swatches carry data-accent + data-theme; nested theme previews only data-theme.
    for (const accent of ACCENT_IDS.filter((a) => a !== "default")) {
      for (const theme of THEMES) {
        expect(tokens).toContain(
          `[data-accent="${accent}"] [data-theme="${theme}"]:not([data-accent]) {`,
        );
      }
    }
  });
});

describe("theme helpers", () => {
  it("resolves follow-system and writes data attributes on <html>", () => {
    expect(resolveTheme({ theme: "warm", follow_system_theme: false }, true)).toBe("warm");
    expect(resolveTheme({ theme: "warm", follow_system_theme: true }, true)).toBe("dark");
    expect(resolveTheme({ theme: "warm", follow_system_theme: true }, false)).toBe("light");
    applyTheme("graphite");
    expect(document.documentElement.dataset.theme).toBe("graphite");
    expect(readTheme()).toBe("graphite");
    applyAppearance({
      theme: "dark",
      accent: "green",
      density: "compact",
      fontSizePx: 40,
      reduceMotion: true,
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.dataset.accent).toBe("green");
    expect(document.documentElement.dataset.density).toBe("compact");
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe(
      `${FONT_SIZE_MAX}px`,
    );
    applyAppearance({
      theme: "light",
      accent: "default",
      density: "default",
      fontSizePx: 14,
      reduceMotion: false,
    });
    expect(document.documentElement.dataset.accent).toBe("default");
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe("14px");
    document.documentElement.dataset.theme = "sepia";
    expect(readTheme()).toBe("light");
    delete document.documentElement.dataset.theme;
    expect(readTheme()).toBe("light");
    expect(isThemeId("dark")).toBe(true);
    expect(isThemeId("x")).toBe(false);
    expect(isAccentId("purple")).toBe(true);
    expect(isAccentId("red")).toBe(false);
    expect(isAccentId(undefined)).toBe(false);
  });

  it("reads system preferences defensively", () => {
    expect(systemPrefersDark({})).toBe(false);
    expect(systemPrefersDark({ matchMedia: () => ({ matches: true }) as MediaQueryList })).toBe(
      true,
    );
    expect(
      systemPrefersReducedMotion({ matchMedia: () => ({ matches: true }) as MediaQueryList }),
    ).toBe(true);
    expect(systemPrefersReducedMotion({})).toBe(false);
    expect(typeof systemPrefersDark()).toBe("boolean");
    expect(typeof systemPrefersReducedMotion()).toBe("boolean");
  });
});

describe("cursor", () => {
  const base = tokens.slice(tokens.indexOf("@layer base"));
  const rule = (cursor: string) =>
    new RegExp(`:where\\(([^{}]*)\\)\\s*\\{\\s*cursor:\\s*${cursor};`, "s").exec(base)?.[1] ?? "";

  it("regression: every control shows the hand cursor and a disabled one the not-allowed cursor, since Tailwind v4 leaves buttons on the arrow (user feedback 2026-09-28)", () => {
    const pointer = rule("pointer");
    for (const selector of [
      "button",
      "select",
      "summary",
      "a[href]",
      "label[for]",
      '[role="button"]',
      '[role="tab"]',
      '[role="option"]',
      '[role="radio"]',
      '[role="switch"]',
      '[role="checkbox"]',
      '[role="menuitem"]',
    ])
      expect(pointer).toContain(selector);
    expect(base).toMatch(
      /:where\(button, select, input, textarea\):disabled,\s*:where\(\[aria-disabled="true"\]\)\s*\{\s*cursor:\s*not-allowed;/,
    );
  });
});
