#!/usr/bin/env bash
# The Android packages before they go anywhere (docs/runbook.md 发布 · Android), on CI's throwaway
# key and on the release candidate's real one:
#   1. the APK has exactly one signer, the certificate named;
#   2. the AAB verifies and carries that certificate too (Play's upload key is the app's own);
#   3. the APK is aligned for 16 KB pages (zipalign -P 16): Play requires it from Android 15 on;
#   4. every shared library in the APK and in the AAB has its LOAD segments 16 KB aligned;
#   5. package name, version name and code, and target SDK are the release's: the version code is
#      Tauri's major × 1 000 000 + minor × 1 000 + patch, so every release code is higher;
#   6. no provider API key is baked into the native library (scripts/lib/artefact-checks.sh);
#   7. both carry this release's licence texts (third-party-notices.py --app mobile, put in the
#      assets by tauri.package-android.conf.json).
# The React Native phone app (`--app mobile-rn`, docs/mobile-rn.md §6) is an APK alone: the same
# checks without the AAB, for its package dev.voltip.mobile.rn and its library libvoltip_rn.so.
#
# Usage: check-android-package.sh <apk> <aab> <certificate SHA-256, hex, colons optional> <version>
#        check-android-package.sh --app mobile-rn <apk> <certificate SHA-256> <version>
# Needs ANDROID_BUILD_TOOLS (a build-tools directory: apksigner, zipalign, aapt2; default the newest
# under ANDROID_HOME), NDK_HOME (llvm-readelf) and a JDK on PATH (jarsigner, keytool).
# .github/scripts/android-toolchain.sh sets them up on CI.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
usage() {
  echo "usage: $0 <apk> <aab> <certificate sha256> <version>" >&2
  echo "       $0 --app mobile-rn <apk> <certificate sha256> <version>" >&2
  exit 2
}
if [ "${1:-}" = --app ]; then
  [ "$#" -eq 5 ] && [ "$2" = mobile-rn ] || usage
  apk=$3 aab="" certificate=$4 version=$5
  expected_package=dev.voltip.mobile.rn app_library=libvoltip_rn.so
else
  [ "$#" -eq 4 ] || usage
  apk=$1 aab=$2 certificate=$3 version=$4
  expected_package=dev.voltip.mobile app_library=libvoltip_mobile_lib.so
fi
expected=$(tr -d ':' <<<"$certificate" | tr 'A-F' 'a-f')
fail() {
  echo "::error title=Android package::$*" >&2
  exit 1
}
[[ $expected =~ ^[0-9a-f]{64}$ ]] || fail "the expected certificate digest is not a SHA-256: $certificate"
[[ $version =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || fail "version $version is not major.minor.patch"
code=$((BASH_REMATCH[1] * 1000000 + BASH_REMATCH[2] * 1000 + BASH_REMATCH[3]))
[ -f "$apk" ] || fail "no APK at $apk"
[ -z "$aab" ] || [ -f "$aab" ] || fail "no AAB at $aab"
tools=${ANDROID_BUILD_TOOLS:-$(printf '%s\n' "${ANDROID_HOME:?}"/build-tools/* | sort -V | tail -1)}
readelf=${NDK_HOME:?}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf
for tool in "$tools/apksigner" "$tools/zipalign" "$tools/aapt2" "$readelf"; do
  [ -x "$tool" ] || fail "$tool is missing"
done
for tool in jarsigner keytool unzip; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

# 1. The APK's signer.
signers=$("$tools/apksigner" verify --print-certs "$apk") || fail "apksigner refuses $apk"
apk_digest=$(sed -n 's/^Signer #1 certificate SHA-256 digest: //p' <<<"$signers")
[ "$(grep -c '^Signer #[0-9]* certificate DN' <<<"$signers")" = 1 ] || fail "the APK has more than one signer"
[ "$apk_digest" = "$expected" ] || fail "the APK is signed with $apk_digest, not $expected"

# 2. The AAB's signature and certificate.
if [ -n "$aab" ]; then
  verified=$(jarsigner -verify "$aab" 2>&1) || fail "jarsigner refuses $aab: $verified"
  grep -F "jar verified." <<<"$verified" >/dev/null || fail "jarsigner does not verify $aab: $verified"
  aab_digest=$(keytool -printcert -jarfile "$aab" | sed -n 's/^[[:space:]]*SHA256: //p' | head -1 | tr -d ':' | tr 'A-F' 'a-f')
  [ "$aab_digest" = "$expected" ] || fail "the AAB is signed with ${aab_digest:-nothing}, not $expected"
fi

# 3. 16 KB page alignment of the APK's uncompressed native code.
"$tools/zipalign" -c -P 16 -v 4 "$apk" >"$scratch/zipalign.txt" || fail "the APK is not aligned for 16 KB pages: $(tail -3 "$scratch/zipalign.txt")"

# 4. The LOAD segments of every shared library.
unzip -q "$apk" 'lib/*' -d "$scratch/apk"
mkdir -p "$scratch/aab"
[ -z "$aab" ] || unzip -q "$aab" 'base/lib/*' -d "$scratch/aab"
libraries=0
while IFS= read -r library; do
  libraries=$((libraries + 1))
  aligns=$("$readelf" -lW "$library" | awk '$1 == "LOAD" { print $NF }')
  [ -n "$aligns" ] || fail "${library#"$scratch"/} has no LOAD segment"
  for align in $aligns; do
    ((align >= 0x4000)) || fail "${library#"$scratch"/} has a LOAD segment aligned to $align, below 16 KB"
  done
done < <(find "$scratch/apk" "$scratch/aab" -name '*.so' | sort)
[ "$libraries" -gt 0 ] || fail "no native library in the packages"
native=$scratch/apk/lib/arm64-v8a/$app_library
[ -f "$native" ] || fail "the APK has no arm64-v8a $app_library"

# 5. What the package says it is.
badging=$("$tools/aapt2" dump badging "$apk")
package=$(sed -n "s/^package: name='\([^']*\)'.*/\1/p" <<<"$badging")
version_name=$(sed -n "s/^package: .* versionName='\([^']*\)'.*/\1/p" <<<"$badging")
version_code=$(sed -n "s/^package: .* versionCode='\([^']*\)'.*/\1/p" <<<"$badging")
target_sdk=$(sed -n "s/^targetSdkVersion:'\([^']*\)'/\1/p" <<<"$badging")
[ "$package" = "$expected_package" ] || fail "package name $package, expected $expected_package"
[ "$version_name" = "$version" ] || fail "version name $version_name, expected $version"
[ "$version_code" = "$code" ] || fail "version code $version_code, expected $code"
[ "$target_sdk" = 36 ] || fail "target SDK $target_sdk, expected 36"

# 6. No provider key inside the native library.
# shellcheck source=scripts/lib/artefact-checks.sh
. "$here/../../scripts/lib/artefact-checks.sh"
voltip_scan_provider_keys "$native" android || fail "a provider API key is inside $app_library"

# 7. The licence texts, in the APK's assets and in the AAB's base module.
unzip -p "$apk" assets/THIRD-PARTY-NOTICES.txt >"$scratch/apk-notices.txt" 2>/dev/null || fail "the APK has no assets/THIRD-PARTY-NOTICES.txt"
notices_files=("$scratch/apk-notices.txt")
if [ -n "$aab" ]; then
  unzip -p "$aab" base/assets/THIRD-PARTY-NOTICES.txt >"$scratch/aab-notices.txt" 2>/dev/null || fail "the AAB has no base/assets/THIRD-PARTY-NOTICES.txt"
  notices_files+=("$scratch/aab-notices.txt")
fi
for notices in "${notices_files[@]}"; do
  [ "$(head -1 "$notices")" = "Voltip $version — third-party notices" ] || fail "the licence texts in the packages are not this release's: $(head -1 "$notices")"
done

echo "check-android-package: $package $version_name (code $version_code, target SDK $target_sdk); $([ -n "$aab" ] && echo "APK and AAB" || echo APK) signed by $expected; $libraries native libraries 16 KB aligned; no provider key; licence texts present"
