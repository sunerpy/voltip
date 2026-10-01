#!/usr/bin/env bash
# The Android toolchain on a GitHub-hosted Ubuntu runner, for CI's `android` job and the release
# candidate's `bundle-android` leg (docs/runbook.md 发布 · Android): JDK 21 from the runner image
# and the SDK packages the build and check-android-package.sh use, pinned: the platform of
# compileSdk / targetSdk, the build tools (apksigner, zipalign, aapt2) and the NDK (clang for the
# C parts of the dependencies, llvm-readelf). The image's own NDK and build tools change with
# every image update; these do not.
#
# Writes JAVA_HOME, ANDROID_HOME and NDK_HOME to $GITHUB_ENV and the JDK to $GITHUB_PATH.
set -euo pipefail

PLATFORM="platforms;android-36"
BUILD_TOOLS_VERSION="35.0.0"
NDK_VERSION="29.0.13846066"

fail() {
  echo "::error title=Android toolchain::$*" >&2
  exit 1
}
java_home=${JAVA_HOME_21_X64:-}
[ -x "$java_home/bin/java" ] || fail "the runner image has no JDK 21 (JAVA_HOME_21_X64)"
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
sdkmanager=$sdk/cmdline-tools/latest/bin/sdkmanager
[ -x "$sdkmanager" ] || fail "no Android SDK with cmdline-tools at '${sdk}'"

export JAVA_HOME=$java_home
# The image accepts the licences already; this answers for any package that asks again.
yes | "$sdkmanager" --licenses >/dev/null || true
"$sdkmanager" --install "$PLATFORM" "build-tools;$BUILD_TOOLS_VERSION" "ndk;$NDK_VERSION"
ndk=$sdk/ndk/$NDK_VERSION
for tool in "$sdk/build-tools/$BUILD_TOOLS_VERSION/apksigner" "$sdk/build-tools/$BUILD_TOOLS_VERSION/zipalign" \
  "$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/clang" "$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf"; do
  [ -x "$tool" ] || fail "$tool is missing after the install"
done
[ -d "$sdk/platforms/android-36" ] || fail "platforms/android-36 is missing after the install"

{
  echo "JAVA_HOME=$java_home"
  echo "ANDROID_HOME=$sdk"
  echo "NDK_HOME=$ndk"
  echo "ANDROID_BUILD_TOOLS=$sdk/build-tools/$BUILD_TOOLS_VERSION"
} >>"${GITHUB_ENV:?}"
echo "$java_home/bin" >>"${GITHUB_PATH:?}"
"$java_home/bin/java" -version 2>&1 | head -1
echo "android-toolchain: SDK $sdk, build tools $BUILD_TOOLS_VERSION, NDK $NDK_VERSION"
