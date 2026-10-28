// The window's side of the Expo config (plugins/display.js, docs/mobile-rn.md §5): AppTheme names no
// system bar colours, no activity is held to one orientation, and app.config.js applies both to
// every build.
const { SCANNER_ACTIVITY, withScannerUnlocked, withoutSystemBarColors } = require("./display");

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
  });
});
