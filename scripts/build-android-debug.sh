#!/usr/bin/env bash
# Build the Android debug APK (arm64) of the mobile shell and print what was built.
# Needs ANDROID_HOME (SDK with platforms + build-tools), NDK_HOME (NDK 27+), JAVA_HOME (JDK 17+),
# the Rust target aarch64-linux-android and `cargo tauri`. The record goes to dist/android/build-info.txt
# (git-ignored) unless a path is given. The generated Gradle project lives in
# apps/mobile/src-tauri/gen/android and is committed; `cargo tauri android init --ci` recreates it.
set -euo pipefail
cd "$(dirname "$0")/.."
for v in ANDROID_HOME NDK_HOME JAVA_HOME; do
  [ -n "${!v:-}" ] || { echo "build-android-debug: $v is not set"; exit 2; }
done
rustup target list --installed | grep -q '^aarch64-linux-android$' || { echo "build-android-debug: run: rustup target add aarch64-linux-android"; exit 2; }
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/require-builtin-engines.sh && voltip_require_builtin_engines
out=${1:-dist/android/build-info.txt}
mkdir -p "$(dirname "$out")"
(cd apps/mobile && cargo tauri android build --debug --apk --target aarch64)
apk=apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
aapt2=$(printf '%s\n' "$ANDROID_HOME"/build-tools/*/aapt2 | sort -V | tail -1)
{
  echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ) commit=$(git rev-parse --short HEAD) host=$(uname -srm)"
  echo "ndk: $(basename "$NDK_HOME")  java: $("$JAVA_HOME/bin/java" -version 2>&1 | head -1)"
  echo "apk: $apk"
  echo "size: $(stat -c %s "$apk") bytes  sha256: $(sha256sum "$apk" | cut -d' ' -f1)"
  "$aapt2" dump badging "$apk" | grep -E "^package:|^sdkVersion|^targetSdkVersion|^native-code|^application-label:"
  echo "native libs:"; unzip -l "$apk" | awk '/lib\//{print "  " $4 " (" $1 " bytes)"}'
} | tee "$out"
