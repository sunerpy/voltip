// The window's side of the Expo config (plugins/display.js, docs/mobile-rn.md §5): AppTheme names no
// system bar colours, no activity is held to one orientation, and app.config.js applies both to
// every build.
const {
  DOCUMENT_SCANNER,
  SCANNER_ACTIVITY,
  withScannerUnlocked,
  withoutDocumentScanner,
  withoutSystemBarColors,
} = require("./display");

/** styles.xml as Expo SDK 57's prebuild parses it, with what its system-bars plugin adds. */
function styles() {
  return {
    resources: {
      $: { "xmlns:tools": "http://schemas.android.com/tools" },
      style: [
        {
          $: { name: "AppTheme", parent: "Theme.AppCompat.DayNight.NoActionBar" },
          item: [
            { _: "@drawable/rn_edit_text_material", $: { name: "android:editTextBackground" } },
            { _: "@color/colorPrimary", $: { name: "colorPrimary" } },
            { _: "@android:color/transparent", $: { name: "android:statusBarColor" } },
            { _: "@android:color/transparent", $: { name: "android:navigationBarColor" } },
            { _: "@color/activityBackground", $: { name: "android:windowBackground" } },
          ],
        },
        {
          $: { name: "Theme.App.SplashScreen", parent: "Theme.SplashScreen" },
          item: [{ _: "@style/AppTheme", $: { name: "postSplashScreenTheme" } }],
        },
      ],
    },
  };
}

/** The app's AndroidManifest.xml as prebuild parses it. */
function manifest() {
  return {
    manifest: {
      $: { "xmlns:android": "http://schemas.android.com/apk/res/android" },
      application: [
        {
          $: { "android:name": ".MainApplication" },
          activity: [
            { $: { "android:name": ".MainActivity", "android:screenOrientation": "unspecified" } },
          ],
        },
      ],
    },
  };
}

describe("the window config", () => {
  it("leaves the system bar colours out of AppTheme and keeps the rest", () => {
    const result = withoutSystemBarColors(styles());
    const theme = result.resources.style[0].item.map((item) => item.$.name);
    expect(theme).toEqual([
      "android:editTextBackground",
      "colorPrimary",
      "android:windowBackground",
    ]);
    expect(result.resources.style[1].item).toHaveLength(1);
  });

  it("refuses styles without AppTheme", () => {
    expect(() => withoutSystemBarColors({ resources: { style: [] } })).toThrow("no AppTheme");
  });

  it("strips ML Kit's portrait lock from its scanner activity when the manifests merge", () => {
    const result = withScannerUnlocked(manifest());
    const root = result.manifest;
    expect(root.$["xmlns:tools"]).toBe("http://schemas.android.com/tools");
    const scanner = root.application[0].activity.find(
      (activity) => activity.$["android:name"] === SCANNER_ACTIVITY,
    );
    expect(scanner.$).toEqual({
      "android:name": SCANNER_ACTIVITY,
      "tools:remove": "android:screenOrientation",
    });
    // Run twice (a second prebuild of the same manifest): one declaration, one removal.
    const again = withScannerUnlocked(result);
    const scanners = again.manifest.application[0].activity.filter(
      (activity) => activity.$["android:name"] === SCANNER_ACTIVITY,
    );
    expect(scanners).toHaveLength(1);
    expect(scanners[0].$["tools:remove"]).toBe("android:screenOrientation");
    // The app's own activity is app.json's business.
    expect(again.manifest.application[0].activity[0].$["android:screenOrientation"]).toBe(
      "unspecified",
    );
  });

  it("regression: leaves out expo-camera's ML Kit document scanner, whose activity is held to portrait", () => {
    // 2026-11-07, Expo SDK 58: expo-camera 58 brings ML Kit's document scanner, and the scanner's
    // activity declares portrait (check-android-package.sh refused the package). The app scans no
    // documents. Expo builds the app with expo-camera's prebuilt module, which brings the library
    // whatever expo-camera's own Gradle switch says (expo.camera.document-scanner-enabled was false
    // and the activity was still there), so the app's dependencies leave the library out.
    const template =
      'allprojects {\n  repositories {\n    google()\n  }\n}\n\napply plugin: "expo-root-project"\n';
    const result = withoutDocumentScanner(template);
    expect(result.startsWith(template)).toBe(true);
    const exclusion = result.slice(template.length);
    expect(exclusion).toContain('project(":app") {');
    expect(exclusion).toContain(
      `exclude group: "${DOCUMENT_SCANNER.group}", module: "${DOCUMENT_SCANNER.module}"`,
    );
    // Run twice (a second prebuild of the same file): one exclusion.
    expect(withoutDocumentScanner(result)).toBe(result);
    // The library is the one expo-camera's prebuilt module brings at run time (its Gradle metadata,
    // which Gradle prefers to the POM).
    const path = require("node:path");
    const fs = require("node:fs");
    const camera = path.dirname(require.resolve("expo-camera/package.json"));
    const { version } = require("expo-camera/package.json");
    const { publication } = require("expo-camera/expo-module.config.json").android;
    const metadata = JSON.parse(
      fs.readFileSync(
        path.join(
          camera,
          publication.repository,
          ...publication.groupId.split("."),
          publication.artifactId,
          version,
          `${publication.artifactId}-${version}.module`,
        ),
        "utf8",
      ),
    );
    const runtime = metadata.variants.filter(
      (variant) =>
        variant.attributes["org.gradle.category"] === "library" &&
        variant.attributes["org.gradle.usage"] === "java-runtime",
    );
    expect(runtime).toHaveLength(1);
    expect(runtime[0].dependencies).toContainEqual(expect.objectContaining(DOCUMENT_SCANNER));
  });

  it("refuses a manifest without an application", () => {
    expect(() => withScannerUnlocked({ manifest: { $: {} } })).toThrow("no application element");
  });

  it("is applied to every build, and the app follows the device's orientation", async () => {
    const appJson = require("../app.json").expo;
    expect(appJson.orientation).toBe("default");
    const config = require("../app.config")({ config: structuredClone(appJson) });
    const styled = await config.mods.android.styles({
      ...config,
      modRequest: { platform: "android", modName: "styles" },
      modResults: styles(),
    });
    expect(styled.modResults.resources.style[0].item.map((item) => item.$.name)).not.toContain(
      "android:statusBarColor",
    );
    const merged = await config.mods.android.manifest({
      ...config,
      modRequest: { platform: "android", modName: "manifest" },
      modResults: manifest(),
    });
    expect(
      merged.modResults.manifest.application[0].activity.some(
        (activity) => activity.$["android:name"] === SCANNER_ACTIVITY,
      ),
    ).toBe(true);
    const project = await config.mods.android.projectBuildGradle({
      ...config,
      modRequest: { platform: "android", modName: "projectBuildGradle" },
      modResults: { language: "groovy", contents: 'apply plugin: "expo-root-project"\n' },
    });
    expect(project.modResults.contents).toContain(
      `exclude group: "${DOCUMENT_SCANNER.group}", module: "${DOCUMENT_SCANNER.module}"`,
    );
    // A Kotlin build script would not take the Groovy block: the template changed.
    await expect(
      config.mods.android.projectBuildGradle({
        ...config,
        modRequest: { platform: "android", modName: "projectBuildGradle" },
        modResults: { language: "kt", contents: "" },
      }),
    ).rejects.toThrow("not Groovy");
  });
});
