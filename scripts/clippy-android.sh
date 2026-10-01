#!/usr/bin/env bash
# Clippy for the Android build (`make android-clippy`, CI's `android` job): every workspace crate
# the phone's shell links, for aarch64-linux-android, so the code behind cfg(target_os = "android")
# is linted too; the workspace gate builds for the host only. The C parts of the dependencies
# (aws-lc, SQLite) compile with the NDK's clang, as `cargo tauri android build` sets it up.
# Needs NDK_HOME and the Rust target aarch64-linux-android (rustup target add aarch64-linux-android).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${NDK_HOME:?NDK_HOME must name the Android NDK}"
target=aarch64-linux-android
bin=$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
# API level 26: the app's minSdk (apps/mobile/src-tauri/tauri.conf.json).
export CC_aarch64_linux_android=$bin/aarch64-linux-android26-clang
export CXX_aarch64_linux_android=$bin/aarch64-linux-android26-clang++
export AR_aarch64_linux_android=$bin/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$bin/aarch64-linux-android26-clang
[ -x "$CC_aarch64_linux_android" ] || { echo "clippy-android: no NDK clang at $CC_aarch64_linux_android" >&2; exit 2; }

mapfile -t crates < <(cargo tree --locked -p voltip-mobile --target "$target" -e normal --prefix none | awk '/^voltip-/ { print $1 }' | sort -u)
[ "${#crates[@]}" -gt 1 ] || { echo "clippy-android: found no workspace crates under voltip-mobile" >&2; exit 1; }
packages=()
for crate in "${crates[@]}"; do
  packages+=(-p "$crate")
done
echo "clippy-android: ${crates[*]}"
cargo clippy --locked --target "$target" "${packages[@]}" -- -D warnings
