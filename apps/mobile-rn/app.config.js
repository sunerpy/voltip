// The app's Expo config: app.json, with the repository's version (the root package.json, which
// release-please writes and the Tauri apps read too) and its version code, and with
// VOLTIP_RN_UNSIGNED=1 (scripts/build-android-rn.sh --unsigned) an unsigned release build
// (plugins/release.js).
const { version } = require("../../package.json");
const { versionCode, withUnsignedRelease } = require("./plugins/release");

module.exports = ({ config }) => {
  const app = { ...config, version, android: { ...config.android, versionCode: versionCode(version) } };
  return process.env.VOLTIP_RN_UNSIGNED === "1" ? withUnsignedRelease(app) : app;
};
