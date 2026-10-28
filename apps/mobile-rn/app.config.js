// The app's Expo config: app.json, with the repository's version (the root package.json, which
// release-please writes and the Tauri apps read too) and its version code, R8's optimizing defaults
// for the release build and a window held to no orientation or system bar colour (plugins/release.js,
// plugins/display.js), and with VOLTIP_RN_UNSIGNED=1 (scripts/build-android-rn.sh --unsigned) an
// unsigned release build.
const { version } = require("../../package.json");
const { withFreeWindows } = require("./plugins/display");
const { versionCode, withOptimizedRelease, withUnsignedRelease } = require("./plugins/release");

module.exports = ({ config }) => {
  const app = withFreeWindows(
    withOptimizedRelease({
      ...config,
      version,
      android: { ...config.android, versionCode: versionCode(version) },
    }),
  );
  return process.env.VOLTIP_RN_UNSIGNED === "1" ? withUnsignedRelease(app) : app;
};
