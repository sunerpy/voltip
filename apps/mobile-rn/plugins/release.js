// The release side of the app's Expo config (app.config.js, docs/mobile-rn.md §6): Android's
// version code for the repository's version, and the config plugin that leaves Gradle's release
// build unsigned for the release candidate and CI, which sign the APK in a step of their own
// (.github/scripts/sign-android-package.sh --app mobile-rn) so that no key reaches the build.

/** major × 1 000 000 + minor × 1 000 + patch, the version code of Tauri's phone app for the same version. */
function versionCode(version) {
  const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!parts) throw new Error(`version ${version} is not major.minor.patch`);
  const [major, minor, patch] = parts.slice(1).map(Number);
  return major * 1_000_000 + minor * 1_000 + patch;
}

/**
 * `app/build.gradle` without the debug signing the template gives the release build type. Throws
 * when the template no longer has it: the build would otherwise come out signed with the debug
 * key, which the signing step refuses later and less clearly.
 */
function unsignedReleaseGradle(text) {
  const types = text.indexOf("buildTypes {");
  const release = types < 0 ? -1 : text.indexOf("release {", types);
  if (release < 0) throw new Error("app/build.gradle: no release build type");
  // The brace that closes `release {`.
  let depth = 0;
  let end = -1;
  for (let i = text.indexOf("{", release); i < text.length && end < 0; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) end = i;
  }
  if (end < 0) throw new Error("app/build.gradle: the release build type does not close");
  const block = text.slice(release, end);
  const signing = /^[ \t]*signingConfig signingConfigs\.debug[ \t]*\r?\n/m;
  if (!signing.test(block)) {
    throw new Error("app/build.gradle: the release build type does not sign with the debug key; the template changed");
  }
  return text.slice(0, release) + block.replace(signing, "") + text.slice(end);
}

/** The config plugin: Gradle builds the release APK unsigned. */
function withUnsignedRelease(config) {
  // Loaded here so that the functions above need nothing but Node.
  const { withAppBuildGradle } = require("expo/config-plugins");
  return withAppBuildGradle(config, (mod) => {
    if (mod.modResults.language !== "groovy") {
      throw new Error(`app/build.gradle: expected Groovy, found ${mod.modResults.language}`);
    }
    mod.modResults.contents = unsignedReleaseGradle(mod.modResults.contents);
    return mod;
  });
}

module.exports = { versionCode, unsignedReleaseGradle, withUnsignedRelease };
