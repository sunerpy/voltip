// The release side of the Expo config (plugins/release.js, app.config.js, docs/mobile-rn.md §6):
// the version is the repository's with the version code of Tauri's phone app, and the release
// build type loses its debug key only when the build asks for an unsigned APK.
const { unsignedReleaseGradle, versionCode } = require("./release");

// The signing parts of app/build.gradle as Expo SDK 57's template writes them.
const TEMPLATE = `android {
    signingConfigs {
        debug {
            storeFile file('debug.keystore')
            storePassword 'android'
            keyAlias 'androiddebugkey'
            keyPassword 'android'
        }
    }
    buildTypes {
        debug {
            signingConfig signingConfigs.debug
        }
        release {
            // Caution! In production, you need to generate your own keystore file.
            // see https://reactnative.dev/docs/signed-apk-android.
            signingConfig signingConfigs.debug
            def enableShrinkResources = findProperty('android.enableShrinkResourcesInReleaseBuilds') ?: 'false'
            shrinkResources enableShrinkResources.toBoolean()
            minifyEnabled enableMinifyInReleaseBuilds
            proguardFiles getDefaultProguardFile("proguard-android.txt"), "proguard-rules.pro"
        }
    }
}
`;

describe("the release config", () => {
  it("numbers a version as the Tauri phone app does", () => {
    expect(versionCode("0.0.45")).toBe(45);
    expect(versionCode("1.2.3")).toBe(1_002_003);
    expect(() => versionCode("0.0.45-rc.1")).toThrow("not major.minor.patch");
  });

  it("removes the debug key from the release build type and nowhere else", () => {
    const gradle = unsignedReleaseGradle(TEMPLATE);
    expect(gradle.slice(gradle.indexOf("release {"))).not.toContain("signingConfig");
    expect(gradle).toContain("debug {\n            signingConfig signingConfigs.debug\n        }");
    expect(gradle).toContain("storeFile file('debug.keystore')");
    expect(gradle.split("\n")).toHaveLength(TEMPLATE.split("\n").length - 1);
  });

  it("refuses a template whose release build type it does not know", () => {
    expect(() => unsignedReleaseGradle(unsignedReleaseGradle(TEMPLATE))).toThrow(
      "the template changed",
    );
    expect(() => unsignedReleaseGradle("android {\n}\n")).toThrow("no release build type");
  });

  it("takes the repository's version and stays signed unless asked", () => {
    const appJson = require("../app.json").expo;
    const { version } = require("../../../package.json");
    const appConfig = require("../app.config");
    const signed = appConfig({ config: appJson });
    expect(signed.version).toBe(version);
    expect(signed.android.versionCode).toBe(versionCode(version));
    expect(signed.android.package).toBe("dev.voltip.mobile.rn");
    expect(signed.mods).toBeUndefined();
    process.env.VOLTIP_RN_UNSIGNED = "1";
    try {
      expect(appConfig({ config: appJson }).mods.android.appBuildGradle).toEqual(
        expect.any(Function),
      );
    } finally {
      delete process.env.VOLTIP_RN_UNSIGNED;
    }
  });
});
