#!/usr/bin/env bash
# Sign the unsigned APK and AAB Gradle builds of the Android app (docs/runbook.md 发布 · Android;
# the React Native app since 0.0.50, scripts/build-android-rn.sh), in a step of their own: the
# build runs every dependency's build code, so the release key never sits in it. The APK
# is aligned for 16 KB pages, then signed with apksigner (v2 and v3: minSdk 26 needs no v1, and the
# v4 .idsig file is for incremental adb installs only); the AAB is signed with jarsigner, as Google
# Play expects of an upload. Both land under the names the
# release gives them, in the layout `updater-json.py collect` reads:
#   <out>/apk/Voltip_<version>_android_arm64.apk  <out>/aab/Voltip_<version>_android_arm64.aab
#
# Usage: sign-android-package.sh <unsigned apk> <unsigned aab> <out dir> <version>
# Environment: ANDROID_KEYSTORE (path), ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS,
# ANDROID_KEY_PASSWORD (read by the tools from the environment, never put on a command line);
# ANDROID_BUILD_TOOLS (default the newest build-tools under ANDROID_HOME); a JDK on PATH.
set -euo pipefail

usage() {
  echo "usage: $0 <unsigned apk> <unsigned aab> <out dir> <version>" >&2
  exit 2
}
[ "$#" -eq 4 ] || usage
apk=$1 aab=$2 out=$3 version=$4
fail() {
  echo "::error title=Android signing::$*" >&2
  exit 1
}
for name in ANDROID_KEYSTORE ANDROID_KEYSTORE_PASSWORD ANDROID_KEY_ALIAS ANDROID_KEY_PASSWORD; do
  [ -n "${!name:-}" ] || fail "$name is empty"
done
[ -f "$ANDROID_KEYSTORE" ] || fail "no keystore at $ANDROID_KEYSTORE"
[ -f "$apk" ] || fail "no APK at $apk"
[ -f "$aab" ] || fail "no AAB at $aab"
tools=${ANDROID_BUILD_TOOLS:-$(printf '%s\n' "${ANDROID_HOME:?}"/build-tools/* | sort -V | tail -1)}
for tool in "$tools/apksigner" "$tools/zipalign"; do
  [ -x "$tool" ] || fail "$tool is missing"
done
command -v jarsigner >/dev/null || fail "jarsigner is not on PATH"
# Gradle signs whatever its signing config names: an input that already carries a signature is a
# build that did not come out unsigned, and signing it again would leave two signers.
if "$tools/apksigner" verify "$apk" >/dev/null 2>&1; then
  fail "$apk is signed already"
fi
verified=$(jarsigner -verify "$aab" 2>&1 || true)
[[ $verified == *"jar is unsigned"* ]] || fail "$aab is signed already"
signed_apk=$out/apk/Voltip_${version}_android_arm64.apk
mkdir -p "$out/apk"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

"$tools/zipalign" -P 16 -f 4 "$apk" "$scratch/aligned.apk"
"$tools/apksigner" sign --ks "$ANDROID_KEYSTORE" --ks-key-alias "$ANDROID_KEY_ALIAS" \
  --ks-pass env:ANDROID_KEYSTORE_PASSWORD --key-pass env:ANDROID_KEY_PASSWORD --v4-signing-enabled false \
  --out "$signed_apk" "$scratch/aligned.apk"

signed_aab=$out/aab/Voltip_${version}_android_arm64.aab
mkdir -p "$out/aab"
cp "$aab" "$scratch/app.aab"
jarsigner -keystore "$ANDROID_KEYSTORE" -storepass:env ANDROID_KEYSTORE_PASSWORD -keypass:env ANDROID_KEY_PASSWORD \
  -sigalg SHA256withRSA -digestalg SHA-256 "$scratch/app.aab" "$ANDROID_KEY_ALIAS" >"$scratch/jarsigner.log" ||
  fail "jarsigner could not sign the AAB: $(tail -3 "$scratch/jarsigner.log")"
mv "$scratch/app.aab" "$signed_aab"
echo "sign-android-package: $signed_apk, $signed_aab"
