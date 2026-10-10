// The 记录 tab's stat tile colours (user decision 2026-10-10, option C with total ①).
import { appTheme } from "../theme/themes";
import { alpha, mix, statTileColors } from "./statTile";

describe("stat tile colours", () => {
  it("mixes hex colours and leaves anything else alone", () => {
    expect(mix("#ffffff", "#000000", 0.5)).toBe("#808080");
    expect(mix("#ffffff", "#339cff", 0)).toBe("#ffffff");
    expect(mix("rgba(0, 0, 0, 0.5)", "#000000", 0.5)).toBe("rgba(0, 0, 0, 0.5)");
    expect(alpha("#ffffff", 0.12)).toBe("rgba(255, 255, 255, 0.12)");
    expect(alpha("transparent", 0.5)).toBe("transparent");
  });

  it("mixes today's, this week's and this month's hue into the card, more in a dark theme", () => {
    for (const id of ["light", "warm", "dark", "graphite"] as const) {
      const theme = appTheme(id);
      const [tint, edge] = theme.dark ? [0.08, 0.28] : [0.05, 0.22];
      const today = statTileColors(theme, "today");
      expect(today.background).toBe(mix(theme.colors.surface, theme.colors.primary, tint));
      expect(today.chip).toBe(theme.colors.primaryContainer);
      expect(statTileColors(theme, "week").border).toBe(
        mix(theme.colors.surface, theme.voltip.ok, edge),
      );
      expect(statTileColors(theme, "week").icon).toBe(theme.voltip.okText);
      expect(statTileColors(theme, "month").chip).toBe(theme.voltip.warningSoft);
      expect(today.fg).toBe(theme.colors.onSurface);
    }
  });

  it("makes the total the lit ink card in a light theme and the accent-washed card in a dark one", () => {
    for (const id of ["light", "warm"] as const) {
      const theme = appTheme(id);
      const total = statTileColors(theme, "total");
      expect(total.background).toBeUndefined();
      expect(total.gradient).toEqual({
        from: mix(theme.colors.inverseSurface, theme.colors.primary, 0.12),
        to: theme.colors.inverseSurface,
        glow: theme.colors.primary,
        glowOpacity: 0.55,
      });
      expect(total.fg).toBe(theme.colors.inverseOnSurface);
      expect(total.icon).toBe(theme.colors.inversePrimary);
    }
    for (const id of ["dark", "graphite"] as const) {
      const theme = appTheme(id);
      const total = statTileColors(theme, "total");
      expect(total.gradient?.from).toBe(theme.colors.primaryContainer);
      expect(total.gradient?.to).toBe(theme.colors.surface);
      expect(total.chip).toBe(theme.colors.primary);
      expect(total.icon).toBe(theme.colors.onPrimary);
      expect(total.fg).toBe(theme.colors.onSurface);
    }
  });
});
