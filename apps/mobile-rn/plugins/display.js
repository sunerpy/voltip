// The window's side of the app's Expo config (app.config.js, docs/mobile-rn.md §5), for Google
// Play's Android 15 and 16 notices (2026-10-28):
// - The app's theme names no system bar colours. Expo's system-bars plugin writes transparent
//   status and navigation bar colours into AppTheme; an app that targets Android 15 is drawn edge
//   to edge whatever they say, and below 15 React Native's own edge-to-edge set-up makes both bars
//   transparent at start, so they only add two deprecated attributes to the package.
// - No activity of the app is held to one orientation. The app's own follows app.json
//   (`"orientation": "default"`); ML Kit's barcode-scanner activity declares portrait, though the app
//   never opens it (it scans with its own camera preview, src/screens/Scanner.tsx), so its
//   declaration loses the attribute when the manifests merge. expo-camera 58 also brings ML Kit's
//   document scanner, whose activity declares portrait too; the app scans no documents, so the
//   app's dependencies leave the library out, and its activity with it. expo-camera's own switch
//   (`expo.camera.document-scanner-enabled`) only reaches a build from source: Expo SDK 58 uses its
//   prebuilt module, whose Gradle metadata always lists the library.

const SYSTEM_BAR_COLORS = ["android:statusBarColor", "android:navigationBarColor"];

/** ML Kit's barcode-scanner activity, which the code-scanner library declares in portrait. */
const SCANNER_ACTIVITY =
  "com.google.mlkit.vision.codescanner.internal.GmsBarcodeScanningDelegateActivity";

/** ML Kit's document scanner, which expo-camera 58 brings and whose activity declares portrait. */
const DOCUMENT_SCANNER = {
  group: "com.google.android.gms",
  module: "play-services-mlkit-document-scanner",
};

const EXCLUSION_BEGIN =
  "// voltip: begin, ML Kit's document scanner (apps/mobile-rn/plugins/display.js)";
const EXCLUSION_END = "// voltip: end, ML Kit's document scanner";

/** The app project's dependencies without the document scanner, in Groovy for the root
 *  build.gradle. Scoped to the app: a build of expo-camera from source still compiles against it. */
const DOCUMENT_SCANNER_EXCLUSION = `${EXCLUSION_BEGIN}
project(":app") {
  configurations.configureEach {
    exclude group: "${DOCUMENT_SCANNER.group}", module: "${DOCUMENT_SCANNER.module}"
  }
}
${EXCLUSION_END}
`;

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

/** The project's root `build.gradle` with ML Kit's document scanner left out of the app: the
 *  exclusion goes at the end, once (an earlier one is replaced). */
function withoutDocumentScanner(buildGradle) {
  let rest = buildGradle;
  const begin = rest.indexOf(EXCLUSION_BEGIN);
  if (begin !== -1) {
    const end = rest.indexOf(EXCLUSION_END, begin);
    if (end === -1) throw new Error("build.gradle: the document scanner exclusion has no end");
    rest = rest.slice(0, begin) + rest.slice(end + EXCLUSION_END.length).replace(/^\n/, "");
  }
  return `${rest.replace(/\n*$/, "\n")}\n${DOCUMENT_SCANNER_EXCLUSION}`;
}

/** The config plugin: all of the above, for every build. */
function withFreeWindows(config) {
  // Loaded here so that the functions above need nothing but Node.
  const {
    withAndroidManifest,
    withAndroidStyles,
    withProjectBuildGradle,
  } = require("expo/config-plugins");
  const styled = withAndroidStyles(config, (mod) => {
    mod.modResults = withoutSystemBarColors(mod.modResults);
    return mod;
  });
  const unlocked = withAndroidManifest(styled, (mod) => {
    mod.modResults = withScannerUnlocked(mod.modResults);
    return mod;
  });
  return withProjectBuildGradle(unlocked, (mod) => {
    if (mod.modResults.language !== "groovy") {
      throw new Error("build.gradle is not Groovy; the template changed");
    }
    mod.modResults.contents = withoutDocumentScanner(mod.modResults.contents);
    return mod;
  });
}

module.exports = {
  DOCUMENT_SCANNER,
  SCANNER_ACTIVITY,
  withoutDocumentScanner,
  withoutSystemBarColors,
  withScannerUnlocked,
  withFreeWindows,
};
