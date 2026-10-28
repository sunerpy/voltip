// The deprecated edge-to-edge calls of Android 15 (docs/mobile-rn.md §5), kept out of the app's own
// code: Google Play lists every call site it finds in the package, and the ones left are in
// libraries (React Native, androidx, Material, Expo) that guard them by Android version. The app
// draws edge to edge, gives the bars no colour, and sets only the navigation bar's button contrast
// (VoltipNativeModule.setNavigationBarButtons). `pnpm run lint` runs this over src/ and the native
// module's Kotlin; scripts/window-apis.test.js checks the patterns.
const { readdirSync, readFileSync, statSync } = require("node:fs");
const { join, relative } = require("node:path");

/** Each deprecated API, in Kotlin or Java and in React Native's JavaScript. */
const DEPRECATED = [
  [
    /\bsetStatusBarColor\s*\(|\.statusBarColor\s*=/,
    "Window.setStatusBarColor (deprecated in Android 15)",
  ],
  [
    /\bsetNavigationBarColor\s*\(|\.navigationBarColor\s*=/,
    "Window.setNavigationBarColor (deprecated in Android 15)",
  ],
  [
    /\bsetNavigationBarDividerColor\s*\(|\.navigationBarDividerColor\s*=/,
    "Window.setNavigationBarDividerColor (deprecated in Android 15)",
  ],
  [
    /\blayoutInDisplayCutoutMode\b|LAYOUT_IN_DISPLAY_CUTOUT_MODE_/,
    "WindowManager.LayoutParams.layoutInDisplayCutoutMode (deprecated in Android 15)",
  ],
  [
    /\bStatusBar\.setBackgroundColor\s*\(|<StatusBar\b[^>]*\bbackgroundColor\s*=/,
    "StatusBar background colour (no effect edge to edge, deprecated in Android 15)",
  ],
  [
    /\bStatusBar\.setTranslucent\s*\(|<StatusBar\b[^>]*\btranslucent\b/,
    "StatusBar translucency (deprecated in Android 15)",
  ],
];

const SOURCE = /\.(?:tsx?|jsx?|kt|java)$/;

/** Every deprecated call in `text`, with its line. */
function deprecatedCalls(text) {
  const found = [];
  text.split("\n").forEach((line, index) => {
    const code = line.replace(/\/\/.*$/, "");
    for (const [pattern, api] of DEPRECATED) {
      if (pattern.test(code)) found.push({ line: index + 1, api });
    }
  });
  return found;
}

/** The source files under `roots`, tests and generated bindings included (a test may not call
 *  them either), build output and dependencies left out. */
function sourceFiles(roots) {
  const files = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        if (name !== "node_modules" && name !== "build") walk(path);
      } else if (SOURCE.test(name)) {
        files.push(path);
      }
    }
  };
  roots.forEach(walk);
  return files.sort();
}

const ROOTS = ["src", "modules/voltip-native/src", "modules/voltip-native/android/src"];

if (require.main === module) {
  const app = join(__dirname, "..");
  const offenders = [];
  for (const file of sourceFiles(ROOTS.map((root) => join(app, root)))) {
    for (const { line, api } of deprecatedCalls(readFileSync(file, "utf8"))) {
      offenders.push(`${relative(app, file)}:${line}: ${api}`);
    }
  }
  if (offenders.length > 0) {
    console.error(
      `window-apis: the app's own code calls deprecated window APIs:\n${offenders.join("\n")}`,
    );
    process.exit(1);
  }
}

module.exports = { ROOTS, deprecatedCalls, sourceFiles };
