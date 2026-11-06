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

// The same parts as Expo SDK 58's template writes them: the release build type already names
// Android's optimizing R8 rules, and crunches PNGs unless a property says otherwise.
const TEMPLATE_58 = TEMPLATE.replace(
  'proguardFiles getDefaultProguardFile("proguard-android.txt"), "proguard-rules.pro"\n',
  `proguardFiles getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro"
            def enablePngCrunchInRelease = findProperty('android.enablePngCrunchInReleaseBuilds') ?: 'true'
            crunchPngs enablePngCrunchInRelease.toBoolean()
`,
);

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

  it("regression: leaves Expo SDK 58's release build type, which already optimizes, as it is", () => {
    // `expo prebuild` stopped here with SDK 58 ("does not name proguard-android.txt; the template
    // changed"): its template names the optimizing rules itself.
    expect(TEMPLATE_58).not.toBe(TEMPLATE);
    expect(optimizedReleaseGradle(TEMPLATE_58)).toBe(TEMPLATE_58);
    // Optimizing twice changes nothing either.
    expect(optimizedReleaseGradle(optimizedReleaseGradle(TEMPLATE))).toBe(
      optimizedReleaseGradle(TEMPLATE),
    );
    // The unsigned build still loses the debug key there, and only there.
    const unsigned = unsignedReleaseGradle(TEMPLATE_58);
    expect(unsigned.slice(unsigned.indexOf("release {"))).not.toContain("signingConfig");
    expect(unsigned).toContain(
      "debug {\n            signingConfig signingConfigs.debug\n        }",
    );
    expect(unsigned.split("\n")).toHaveLength(TEMPLATE_58.split("\n").length - 1);
  });

  it("refuses a template whose release build type names no default R8 rules", () => {
    const noDefaults = TEMPLATE.replace('getDefaultProguardFile("proguard-android.txt"), ', "");
    expect(noDefaults).toContain('proguardFiles "proguard-rules.pro"');
    expect(() => optimizedReleaseGradle(noDefaults)).toThrow("the template changed");
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

  /** app/build.gradle as app.config.js's mods leave `template`, built with `env`. */
  async function builtGradle(env, template = TEMPLATE) {
    const appJson = require("../app.json").expo;
    const appConfig = require("../app.config");
    const before = { ...process.env };
    Object.assign(process.env, env);
    try {
      const config = appConfig({ config: structuredClone(appJson) });
      const result = await config.mods.android.appBuildGradle({
        ...config,
        modRequest: { platform: "android", modName: "appBuildGradle" },
        modResults: { language: "groovy", contents: template },
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
    // Expo SDK 58's template, which `expo prebuild` hands the mods today.
    expect(await builtGradle({}, TEMPLATE_58)).toBe(TEMPLATE_58);
    const unsigned58 = await builtGradle({ VOLTIP_RN_UNSIGNED: "1" }, TEMPLATE_58);
    expect(unsigned58).toContain('getDefaultProguardFile("proguard-android-optimize.txt")');
    expect(unsigned58.slice(unsigned58.indexOf("release {"))).not.toContain("signingConfig");
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

  it("regression: compiles against the API level of Expo's own modules and still targets 36", () => {
    // 2026-11-07, Expo SDK 58: its prebuilt modules ask for compileSdk 37 in their AAR metadata
    // (minCompileSdk), and Gradle's checkReleaseAarMetadata stopped the build at 36. The compile
    // level decides which APIs the code may call; how the app behaves follows targetSdk, which stays
    // 36 (check-android-package.sh holds the packages to it).
    const appJson = require("../app.json").expo;
    const properties = appJson.plugins.find(
      (plugin) => Array.isArray(plugin) && plugin[0] === "expo-build-properties",
    )[1].android;
    const path = require("node:path");
    const expo = path.dirname(require.resolve("expo/package.json"));
    const autolinking = path.dirname(require.resolve("expo-modules-autolinking/package.json", { paths: [expo] }));
    const plugin = require("node:fs").readFileSync(
      path.join(autolinking, "android/expo-gradle-plugin/expo-autolinking-plugin/src/main/kotlin/expo/modules/plugin/ExpoRootProjectPlugin.kt"),
      "utf8",
    );
    const expoDefault = plugin.match(/getVersionOrDefault\("compileSdk", "(\d+)"\)/);
    expect(expoDefault).not.toBeNull();
    expect(properties.compileSdkVersion).toBe(Number(expoDefault[1]));
    expect(properties.targetSdkVersion).toBe(36);
  });

  it("regression: keeps the stack traces' line numbers in the app's own rules, which Android Gradle Plugin 9 still reads", () => {
    // 2026-11-07, Expo SDK 58 (AGP 9.2): a global option in a library's consumer rules fails the
    // module's build ("Global keep option -renamesourcefileattribute was specified as a
    // consumerProguardFile"), and the app's R8 run leaves out those a library brings. The two that
    // keep the line numbers go to the app's proguard-rules.pro, through expo-build-properties.
    const appJson = require("../app.json").expo;
    const properties = appJson.plugins.find(
      (plugin) => Array.isArray(plugin) && plugin[0] === "expo-build-properties",
    )[1].android;
    expect(properties.extraProguardRules.split("\n")).toEqual([
      "-keepattributes SourceFile,LineNumberTable",
      "-renamesourcefileattribute SourceFile",
    ]);
    const consumer = require("node:fs")
      .readFileSync(require.resolve("../modules/voltip-native/android/consumer-rules.pro"), "utf8")
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "" && !line.startsWith("#"));
    expect(consumer.length).toBeGreaterThan(0);
    // Only rules about the module's own classes: keep rules (not -keepattributes) and -dontwarn.
    for (const rule of consumer) {
      expect(rule).toMatch(/^-(keep|keepclassmembers|keepclasseswithmembers|keepnames|keepclassmembernames|keepclasseswithmembernames|dontwarn) /);
    }
  });
});
