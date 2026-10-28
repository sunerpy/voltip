// The window's side of the app's Expo config (app.config.js, docs/mobile-rn.md §5), for Google
// Play's Android 15 and 16 notices (2026-10-28):
// - The app's theme names no system bar colours. Expo's system-bars plugin writes transparent
//   status and navigation bar colours into AppTheme; an app that targets Android 15 is drawn edge
//   to edge whatever they say, and below 15 React Native's own edge-to-edge set-up makes both bars
//   transparent at start, so they only add two deprecated attributes to the package.
// - No activity of the app is held to one orientation. The app's own follows app.json
//   (`"orientation": "default"`); ML Kit's barcode-scanner activity declares portrait, though the app
//   never opens it (it scans with its own camera preview, src/screens/Scanner.tsx), so its
//   declaration loses the attribute when the manifests merge.

const SYSTEM_BAR_COLORS = ["android:statusBarColor", "android:navigationBarColor"];

/** ML Kit's barcode-scanner activity, which the code-scanner library declares in portrait. */
const SCANNER_ACTIVITY =
  "com.google.mlkit.vision.codescanner.internal.GmsBarcodeScanningDelegateActivity";

/** `styles.xml` (as Expo parses it) with no system bar colour in AppTheme. Throws without AppTheme:
 *  the template changed and the colours would be checked in the wrong place. */
function withoutSystemBarColors(styles) {
  const all = styles?.resources?.style ?? [];
  const theme = all.find((style) => style?.$?.name === "AppTheme");
  if (theme === undefined) throw new Error("styles.xml: no AppTheme; the template changed");
  theme.item = (theme.item ?? []).filter((item) => !SYSTEM_BAR_COLORS.includes(item?.$?.name));
  return styles;
}

/** `AndroidManifest.xml` (as Expo parses it) that strips the orientation from ML Kit's scanner
 *  activity when the manifests merge (`tools:remove`). */
function withScannerUnlocked(manifest) {
  const root = manifest?.manifest;
  const application = root?.application?.[0];
  if (application === undefined) throw new Error("AndroidManifest.xml: no application element");
  root.$ = { ...root.$, "xmlns:tools": "http://schemas.android.com/tools" };
  const activities = (application.activity ??= []);
  let scanner = activities.find((activity) => activity?.$?.["android:name"] === SCANNER_ACTIVITY);
  if (scanner === undefined) {
    scanner = { $: { "android:name": SCANNER_ACTIVITY } };
    activities.push(scanner);
  }
  const removed = (scanner.$["tools:remove"] ?? "").split(",").filter(Boolean);
  if (!removed.includes("android:screenOrientation")) removed.push("android:screenOrientation");
  scanner.$["tools:remove"] = removed.join(",");
  return manifest;
}

/** The config plugin: both of the above, for every build. */
function withFreeWindows(config) {
  // Loaded here so that the functions above need nothing but Node.
  const { withAndroidManifest, withAndroidStyles } = require("expo/config-plugins");
  const styled = withAndroidStyles(config, (mod) => {
    mod.modResults = withoutSystemBarColors(mod.modResults);
    return mod;
  });
  return withAndroidManifest(styled, (mod) => {
    mod.modResults = withScannerUnlocked(mod.modResults);
    return mod;
  });
}

module.exports = { SCANNER_ACTIVITY, withoutSystemBarColors, withScannerUnlocked, withFreeWindows };
