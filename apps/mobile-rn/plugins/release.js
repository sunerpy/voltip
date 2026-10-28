// The release side of the app's Expo config (app.config.js, docs/mobile-rn.md §6): Android's
// version code for the repository's version, the config plugin that gives Gradle's release build
// R8's optimizing defaults, and the one that leaves the release build unsigned for the release
// candidate and CI, which sign the APK in a step of their own
// (.github/scripts/sign-android-package.sh) so that no key reaches the build.

/** major × 1 000 000 + minor × 1 000 + patch, the version code of Tauri's phone app for the same version. */
function versionCode(version) {
  const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!parts) throw new Error(`version ${version} is not major.minor.patch`);
  const [major, minor, patch] = parts.slice(1).map(Number);
  return major * 1_000_000 + minor * 1_000 + patch;
}

/** Where `release {` of `app/build.gradle`'s build types starts, and the brace that closes it. */
function releaseBlock(text) {
  const types = text.indexOf("buildTypes {");
  const release = types < 0 ? -1 : text.indexOf("release {", types);
  if (release < 0) throw new Error("app/build.gradle: no release build type");
  let depth = 0;
  let end = -1;
  for (let i = text.indexOf("{", release); i < text.length && end < 0; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) end = i;
  }
  if (end < 0) throw new Error("app/build.gradle: the release build type does not close");
  return { start: release, end };
}

/** `text` with the release build type's block replaced by `change(block)`. */
function editRelease(text, change) {
  const { start, end } = releaseBlock(text);
  return text.slice(0, start) + change(text.slice(start, end)) + text.slice(end);
}

/**
 * `app/build.gradle` without the debug signing the template gives the release build type. Throws
 * when the template no longer has it: the build would otherwise come out signed with the debug
 * key, which the signing step refuses later and less clearly.
 */
function unsignedReleaseGradle(text) {
  return editRelease(text, (block) => {
    const signing = /^[ \t]*signingConfig signingConfigs\.debug[ \t]*\r?\n/m;
    if (!signing.test(block)) {
      throw new Error(
        "app/build.gradle: the release build type does not sign with the debug key; the template changed",
      );
    }
    return block.replace(signing, "");
  });
}

/**
 * `app/build.gradle` whose release build type runs R8 with Android's optimizing defaults
 * (`proguard-android-optimize.txt`). The template names `proguard-android.txt`, which carries
 * `-dontoptimize`: R8 would then shrink and obfuscate but not optimize, and Google Play measures all
 * three (help answer 17492799). R8 itself is switched on by `enableMinifyInReleaseBuilds` in
 * app.json. Throws when the template no longer names the file, as `unsignedReleaseGradle` does.
 */
function optimizedReleaseGradle(text) {
  return editRelease(text, (block) => {
    const plain = 'getDefaultProguardFile("proguard-android.txt")';
    if (!block.includes(plain)) {
      throw new Error(
        "app/build.gradle: the release build type does not name proguard-android.txt; the template changed",
      );
    }
    return block.replace(plain, 'getDefaultProguardFile("proguard-android-optimize.txt")');
  });
}

/** A config plugin that edits the Groovy `app/build.gradle` with `edit`. */
function withReleaseGradle(config, edit) {
  // Loaded here so that the functions above need nothing but Node.
  const { withAppBuildGradle } = require("expo/config-plugins");
  return withAppBuildGradle(config, (mod) => {
    if (mod.modResults.language !== "groovy") {
      throw new Error(`app/build.gradle: expected Groovy, found ${mod.modResults.language}`);
    }
    mod.modResults.contents = edit(mod.modResults.contents);
    return mod;
  });
}

/** The config plugin: Gradle builds the release APK unsigned. */
function withUnsignedRelease(config) {
  return withReleaseGradle(config, unsignedReleaseGradle);
}

/** The config plugin: the release build optimizes with R8 as well as shrinking and obfuscating. */
function withOptimizedRelease(config) {
  return withReleaseGradle(config, optimizedReleaseGradle);
}

module.exports = {
  versionCode,
  unsignedReleaseGradle,
  optimizedReleaseGradle,
  withUnsignedRelease,
  withOptimizedRelease,
};
