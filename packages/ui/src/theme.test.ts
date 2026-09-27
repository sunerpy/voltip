import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  FONT_SIZE_MAX,
  applyAppearance,
  applyTheme,
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

describe("theme helpers", () => {
  it("resolves follow-system and writes data attributes on <html>", () => {
    expect(resolveTheme({ theme: "warm", follow_system_theme: false }, true)).toBe("warm");
    expect(resolveTheme({ theme: "warm", follow_system_theme: true }, true)).toBe("dark");
    expect(resolveTheme({ theme: "warm", follow_system_theme: true }, false)).toBe("light");
    applyTheme("graphite");
    expect(document.documentElement.dataset.theme).toBe("graphite");
    expect(readTheme()).toBe("graphite");
    applyAppearance({ theme: "dark", density: "compact", fontSizePx: 40, reduceMotion: true });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.dataset.density).toBe("compact");
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe(
      `${FONT_SIZE_MAX}px`,
    );
    applyAppearance({ theme: "light", density: "default", fontSizePx: 14, reduceMotion: false });
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe("14px");
    document.documentElement.dataset.theme = "sepia";
    expect(readTheme()).toBe("light");
    delete document.documentElement.dataset.theme;
    expect(readTheme()).toBe("light");
    expect(isThemeId("dark")).toBe(true);
    expect(isThemeId("x")).toBe(false);
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
