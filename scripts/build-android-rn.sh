#!/usr/bin/env bash
# Build the React Native phone app's APK (docs/mobile-rn.md §6): the Rust shell for arm64 with
# cargo-ndk into the native module's jniLibs and its UniFFI Kotlin bindings beside the module's
# own Kotlin, the Android project from app.json and app.config.js with `expo prebuild` (generated
# every time, never committed), and Gradle's release build: Hermes bytecode, arm64-v8a only. The
# version is the repository's (the root package.json).
#
# Usage: build-android-rn.sh [--unsigned]
#   (default)   signed with the Android debug key: a build to install and try, not a release.
#   --unsigned  signed with nothing (app.config.js leaves Gradle's release build type unsigned):
#               the release candidate and CI sign it in a step of their own
#               (.github/scripts/sign-android-package.sh --app mobile-rn).
#
# Needs ANDROID_HOME (platforms 36), NDK_HOME (clang for the Rust shell), JAVA_HOME (17+), the
# Rust target aarch64-linux-android, cargo-ndk, and `pnpm install` at the root. React Native's
# Gradle plugin compiles against a JDK 17 toolchain: JAVA_TOOLCHAINS (comma-separated JDK homes)
# names one when JAVA_HOME is another version, since Gradle's own download of it comes from GitHub.
# Writes dist/android-rn/Voltip-RN_<version>_android_arm64.apk (with --unsigned
# Voltip-RN_<version>_android_arm64-unsigned.apk) and build-info.txt beside it.
set -euo pipefail
cd "$(dirname "$0")/.."
unsigned=0
case "${1:-}" in
  "") ;;
  --unsigned) unsigned=1 ;;
  *) echo "usage: $0 [--unsigned]" >&2; exit 2 ;;
esac
for v in ANDROID_HOME NDK_HOME JAVA_HOME; do
  [ -n "${!v:-}" ] || { echo "build-android-rn: $v is not set" >&2; exit 2; }
done
rustup target list --installed | grep -q '^aarch64-linux-android$' || { echo "build-android-rn: run: rustup target add aarch64-linux-android" >&2; exit 2; }
command -v cargo-ndk >/dev/null || { echo "build-android-rn: run: cargo install cargo-ndk (CI: .github/scripts/install-cargo-ndk.sh)" >&2; exit 2; }
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/require-builtin-engines.sh && voltip_require_builtin_engines
. scripts/lib/artefact-checks.sh

app=apps/mobile-rn
version=$(node -p "require('./package.json').version")
out=dist/android-rn
mkdir -p "$out"

# 1. The Rust shell and its bindings. cargo-ndk names the NDK's clang for the cc crate and strips
# nothing; the release profile leaves this crate's symbols in too (the root Cargo.toml).
export ANDROID_NDK_HOME=$NDK_HOME
jni=$app/modules/voltip-native/android/src/main/jniLibs
rm -rf "$jni"
cargo ndk -t arm64-v8a --platform 26 -o "$jni" build --release -p voltip-mobile-rn
lib=$jni/arm64-v8a/libvoltip_rn.so
[ -f "$lib" ] || { echo "build-android-rn: cargo-ndk wrote no $lib" >&2; exit 1; }
# cargo-ndk copies every cdylib of the build; android-native-keyring-store is one of its own, and
# its JNI entries are linked into libvoltip_rn.so already (Keyring.kt loads that library).
find "$jni" -name '*.so' ! -name libvoltip_rn.so -print -delete
# The Kotlin bindings of the shell's UniFFI interface (rust/src/ffi.rs), generated from the
# metadata UniFFI embeds in this very library (its symbols name it); the native module compiles
# them with its own Kotlin, and they check every function's checksum against the library when it
# loads. Then the packaged copy loses its symbols, as the release profile strips every other
# library: nothing is built for the build host, so it needs none of the host's audio libraries.
bindings=$app/modules/voltip-native/android/src/main/java
rm -rf "$bindings/dev/voltip/rn/uniffi"
cargo run -q -p voltip-uniffi-bindgen --bin uniffi-bindgen -- \
  generate --library "$lib" --language kotlin --out-dir "$bindings" --no-format
[ -s "$bindings/dev/voltip/rn/uniffi/voltip_rn.kt" ] || { echo "build-android-rn: UniFFI wrote no Kotlin bindings" >&2; exit 1; }
"$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip" --strip-all "$lib"
voltip_scan_provider_keys "$lib" build-android-rn

# 2. The Android project, regenerated from app.json, app.config.js and the local module, with the
# licence texts in its assets, as the Tauri phone app carries them (the About page names the file).
(cd "$app" && CI=1 VOLTIP_RN_UNSIGNED=$unsigned npx expo prebuild --platform android --clean --no-install)
python3 scripts/release/third-party-notices.py --app mobile-rn --version "$version" --out "$app/android/app/src/main/assets/THIRD-PARTY-NOTICES.txt"

# 3. Gradle. Four workers and a 4 GB heap: the Kotlin and C++ compiles of the RN libraries are heavy.
(cd "$app/android" && ./gradlew --no-daemon --max-workers=4 -Dorg.gradle.jvmargs=-Xmx4g \
  -Porg.gradle.java.installations.paths="$JAVA_HOME${JAVA_TOOLCHAINS:+,$JAVA_TOOLCHAINS}" \
  -Porg.gradle.java.installations.auto-download=false \
  -PreactNativeArchitectures=arm64-v8a assembleRelease)
if [ "$unsigned" = 1 ]; then
  apk=$out/Voltip-RN_${version}_android_arm64-unsigned.apk
  cp "$app/android/app/build/outputs/apk/release/app-release-unsigned.apk" "$apk"
else
  apk=$out/Voltip-RN_${version}_android_arm64.apk
  cp "$app/android/app/build/outputs/apk/release/app-release.apk" "$apk"
fi
notices=$(unzip -p "$apk" assets/THIRD-PARTY-NOTICES.txt | head -1) || true
[ "$notices" = "Voltip $version — third-party notices" ] || { echo "build-android-rn: the APK's assets/THIRD-PARTY-NOTICES.txt is missing or not this version's: $notices" >&2; exit 1; }

# 4. No provider key in the package. Every entry is scanned as it is, except the JS bundle: it is
# Hermes bytecode, whose string table packs strings back to back, so icon names such as
# `task-outline` run into their neighbours and read like `sk-…` keys. For it, the sources Metro
# bundled (the packager source map carries their text) take the same scan, and no secret value of
# the build env may appear in the bytecode itself.
scan=$(mktemp -d)
trap 'rm -rf "$scan"' EXIT
unzip -q "$apk" -d "$scan/apk"
while IFS= read -r -d '' entry; do
  [ "${entry#"$scan/apk/"}" = assets/index.android.bundle ] && continue
  voltip_scan_provider_keys "$entry" build-android-rn
done < <(find "$scan/apk" -type f -print0)
map=$app/android/app/build/intermediates/sourcemaps/react/release/index.android.bundle.packager.map
python3 - "$map" >"$scan/bundled-sources.txt" <<'PY'
import json, sys
sources = json.load(open(sys.argv[1], encoding="utf-8")).get("sourcesContent") or []
if not any(sources):
    sys.exit("no sourcesContent in the packager source map")
for text in sources:
    if text:
        print(text)
PY
voltip_scan_provider_keys "$scan/bundled-sources.txt" "build-android-rn (bundled sources)"
for name in $(compgen -A variable VOLTIP_); do
  case "$name" in *_TOKEN | *_KEY | *_SECRET | *_SALT) ;; *) continue ;; esac
  value=${!name}
  [ "${#value}" -ge 8 ] || continue
  if LC_ALL=C grep -qF -- "$value" "$scan/apk/assets/index.android.bundle"; then
    echo "build-android-rn: the value of $name is inside the JS bundle — refusing to ship" >&2
    exit 1
  fi
done

aapt2=$(printf '%s\n' "$ANDROID_HOME"/build-tools/*/aapt2 | sort -V | tail -1)
{
  echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ) commit=$(git rev-parse --short HEAD)$(git diff --quiet HEAD -- . || echo '+dirty') host=$(uname -srm)"
  # The uncommitted state the APK was built from: the diff against HEAD and the untracked files.
  echo "tree: $({ git diff HEAD --binary; git ls-files -z --others --exclude-standard -- . ':!gate-evidence' | sort -z | xargs -0 -r sha256sum; } | sha256sum | cut -c1-16)"
  echo "ndk: $(basename "$NDK_HOME")  java: $("$JAVA_HOME/bin/java" -version 2>&1 | head -1)"
  echo "apk: $apk"
  echo "size: $(stat -c %s "$apk") bytes  sha256: $(sha256sum "$apk" | cut -d' ' -f1)"
  "$aapt2" dump badging "$apk" | grep -E "^package:|^sdkVersion|^targetSdkVersion|^native-code|^application-label:"
  echo "native libs:"; unzip -l "$apk" | awk '/lib\//{print "  " $4 " (" $1 " bytes)"}'
} | tee "$out/build-info.txt"
