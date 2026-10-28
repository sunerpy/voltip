// The release side of the Expo config (plugins/release.js, app.config.js, docs/mobile-rn.md §6):
// the version is the repository's with the version code of Tauri's phone app, the release build
// type runs R8 with Android's optimizing defaults, and it loses its debug key only when the build
// asks for an unsigned APK.
const { optimizedReleaseGradle, unsignedReleaseGradle, versionCode } = require("./release");

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

  it("optimizes with R8 in the release build type and nowhere else", () => {
    const gradle = optimizedReleaseGradle(TEMPLATE);
    expect(gradle).toContain(
      'proguardFiles getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro"',
    );
    expect(gradle).not.toContain('"proguard-android.txt"');
    expect(gradle).toContain("minifyEnabled enableMinifyInReleaseBuilds");
    // The signing stays as it was: this edit only names the optimizing rules.
    expect(gradle.slice(gradle.indexOf("release {"))).toContain(
      "signingConfig signingConfigs.debug",
    );
    expect(gradle.split("\n")).toHaveLength(TEMPLATE.split("\n").length);
  });

  it("refuses a template whose release build type names no default R8 rules", () => {
    expect(() => optimizedReleaseGradle(optimizedReleaseGradle(TEMPLATE))).toThrow(
      "the template changed",
    );
    expect(() => optimizedReleaseGradle("android {\n}\n")).toThrow("no release build type");
    // The debug build type's rules are not the release build's.
    const debugOnly = TEMPLATE.replace(
      'proguardFiles getDefaultProguardFile("proguard-android.txt"), "proguard-rules.pro"',
      "",
    ).replace(
      "debug {\n",
      'debug {\n            proguardFiles getDefaultProguardFile("proguard-android.txt")\n',
    );
    expect(() => optimizedReleaseGradle(debugOnly)).toThrow("the template changed");
  });

  /** app/build.gradle as app.config.js's mods leave it, built with `env`. */
  async function builtGradle(env) {
    const appJson = require("../app.json").expo;
    const appConfig = require("../app.config");
    const before = { ...process.env };
    Object.assign(process.env, env);
    try {
      const config = appConfig({ config: structuredClone(appJson) });
      const result = await config.mods.android.appBuildGradle({
        ...config,
        modRequest: { platform: "android", modName: "appBuildGradle" },
        modResults: { language: "groovy", contents: TEMPLATE },
      });
      return result.modResults.contents;
    } finally {
      for (const key of Object.keys(env)) {
        if (before[key] === undefined) delete process.env[key];
        else process.env[key] = before[key];
      }
    }
  }

  it("takes the repository's version, optimizes every release build and stays signed unless asked", async () => {
    const appJson = require("../app.json").expo;
    const { version } = require("../../../package.json");
    const appConfig = require("../app.config");
    const signed = appConfig({ config: structuredClone(appJson) });
    expect(signed.version).toBe(version);
    expect(signed.android.versionCode).toBe(versionCode(version));
    expect(signed.android.package).toBe("dev.voltip.mobile");
    const plain = await builtGradle({});
    expect(plain).toContain('getDefaultProguardFile("proguard-android-optimize.txt")');
    expect(plain.slice(plain.indexOf("release {"))).toContain("signingConfig signingConfigs.debug");
    const unsigned = await builtGradle({ VOLTIP_RN_UNSIGNED: "1" });
    expect(unsigned).toContain('getDefaultProguardFile("proguard-android-optimize.txt")');
    expect(unsigned.slice(unsigned.indexOf("release {"))).not.toContain("signingConfig");
  });

  it("switches R8 on for release builds in app.json, without shrinking resources", () => {
    const appJson = require("../app.json").expo;
    const properties = appJson.plugins.find(
      (plugin) => Array.isArray(plugin) && plugin[0] === "expo-build-properties",
    )[1].android;
    expect(properties.enableMinifyInReleaseBuilds).toBe(true);
    // React Native looks resources up by name: shrinking them gains nothing Play measures.
    expect(properties.enableShrinkResourcesInReleaseBuilds).toBeUndefined();
  });
});
