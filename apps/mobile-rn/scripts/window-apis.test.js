// The check `pnpm run lint` runs (scripts/window-apis.js, docs/mobile-rn.md §5): it finds each
// deprecated edge-to-edge call in Kotlin and in React Native's JavaScript, ignores comments, and
// the app's own sources have none.
const { join } = require("node:path");
const { readFileSync } = require("node:fs");

const { ROOTS, deprecatedCalls, sourceFiles } = require("./window-apis");

describe("the deprecated window API check", () => {
  it("finds each deprecated call", () => {
    const kotlin = [
      "window.statusBarColor = Color.TRANSPARENT",
      "window.setNavigationBarColor(color)",
      "window.navigationBarDividerColor = 0",
      "attributes.layoutInDisplayCutoutMode = mode",
      "val mode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES",
    ].join("\n");
    expect(deprecatedCalls(kotlin).map((found) => found.line)).toEqual([1, 2, 3, 4, 5]);
    const js = [
      '<StatusBar style="dark" backgroundColor="#fff" />',
      "StatusBar.setTranslucent(true);",
      "StatusBar.setBackgroundColor('#000');",
    ].join("\n");
    expect(deprecatedCalls(js).map((found) => found.api)).toEqual([
      expect.stringContaining("StatusBar background"),
      expect.stringContaining("StatusBar translucency"),
      expect.stringContaining("StatusBar background"),
    ]);
  });

  it("leaves the calls the app makes, and comments, alone", () => {
    const fine = [
      "WindowCompat.getInsetsController(this, decorView).isAppearanceLightNavigationBars = light",
      '<StatusBar style={theme.dark ? "light" : "dark"} />',
      "// the template set window.statusBarColor = transparent",
      "val color = theme.colors.statusBar",
    ].join("\n");
    expect(deprecatedCalls(fine)).toEqual([]);
  });

  it("finds none in the app's own code", () => {
    const app = join(__dirname, "..");
    const files = sourceFiles(ROOTS.map((root) => join(app, root)));
    expect(files.some((file) => file.endsWith("VoltipNativeModule.kt"))).toBe(true);
    expect(files.some((file) => file.endsWith("App.tsx"))).toBe(true);
    const offenders = files.flatMap((file) =>
      deprecatedCalls(readFileSync(file, "utf8")).map(
        (found) => `${file}:${found.line} ${found.api}`,
      ),
    );
    expect(offenders).toEqual([]);
  });
});
