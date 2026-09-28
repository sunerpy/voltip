#!/usr/bin/env bash
# Cross-build the Windows x64 desktop package from Linux: the NSIS installer and the portable zip
# (voltip-desktop.exe with the DLLs it loads). The local models run on a GPU through Vulkan, falling
# back to the CPU without one (docs/dictation.md §10.6): scripts/lib/vulkan-sdk.sh fetches the SDK
# and the Khronos loader vulkan-1.dll, which ships beside the exe. Needs cargo-xwin, lld-link,
# llvm-rc, llvm-readobj, llvm-dlltool, clang, makensis, zip and the Rust target
# x86_64-pc-windows-msvc. Output is copied to dist/windows-x64 with SHA256SUMS.txt.
# The result is not Authenticode-signed (SmartScreen will warn). The release workflow re-bundles the
# installer with the updater key to add its .sig when the updater is on (.github/workflows/release.yml).
set -euo pipefail
cd "$(dirname "$0")/.."
# The MSVC STL headers cargo-xwin downloads (current CRT) refuse clang older than 19
# ("STL1000: expected Clang 19.0.0 or newer"); prefer a versioned LLVM 19 install when present.
if [ -d /usr/lib/llvm-19/bin ]; then export PATH="/usr/lib/llvm-19/bin:$PATH"; fi
for tool in cargo-xwin cargo-about lld-link llvm-rc llvm-readobj llvm-dlltool clang makensis zip unzip; do
  command -v "$tool" >/dev/null || { echo "build-windows-x64: $tool not installed"; exit 2; }
done
clang_major=$(clang --version | head -1 | grep -oE '[0-9]+' | head -1)
[ "${clang_major:-0}" -ge 19 ] || { echo "build-windows-x64: clang $clang_major is too old for the MSVC STL (need 19+; apt install clang-19 lld-19 llvm-19)"; exit 2; }
rustup target list --installed | grep -q '^x86_64-pc-windows-msvc$' || { echo "build-windows-x64: run: rustup target add x86_64-pc-windows-msvc"; exit 2; }
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/require-builtin-engines.sh && voltip_require_builtin_engines
# TRANSCRIBE_CMAKE_ARGS / VOLTIP_VULKAN_LIB_DIR for the Vulkan backend, and vulkan-1.dll (+ its
# licence) staged into src-tauri/resources/windows/, and THIRD-PARTY-NOTICES.txt into
# src-tauri/resources/. The bundle picks them up through the tauri.package-windows.conf.json
# overlay: tauri-build checks every listed resource on every build, so a plain native
# `cargo build` (no SDK, nothing staged) must not list them.
. scripts/lib/vulkan-sdk.sh && voltip_vulkan_env windows
# VOLTIP_NOTICES_CACHED=1: CI restored the file from a cache keyed on every input of the
# generator (the lockfiles, manifests, about.toml, the script and its licence texts); cargo-about
# takes about two minutes on a runner. The release always regenerates it.
if [ "${VOLTIP_NOTICES_CACHED:-0}" = 1 ] && [ -s apps/desktop/src-tauri/resources/THIRD-PARTY-NOTICES.txt ]; then
  echo "build-windows-x64: THIRD-PARTY-NOTICES.txt from the CI cache"
else
  python3 scripts/release/third-party-notices.py --out apps/desktop/src-tauri/resources/THIRD-PARTY-NOTICES.txt
fi
out=${1:-dist/windows-x64}
# The version being built (package.json, which tauri.conf.json points at). The bundle directory
# keeps installers from earlier builds; only this version's is shipped.
version=$(node -p 'require("./package.json").version')
product=$(node -p 'require("./apps/desktop/src-tauri/tauri.conf.json").productName')
(cd apps/desktop && cargo tauri build --ci --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis --features gpu-vulkan --config src-tauri/tauri.package-windows.conf.json)
release=target/x86_64-pc-windows-msvc/release
exe=$release/voltip-desktop.exe
setup=$release/bundle/nsis/${product}_${version}_x64-setup.exe
[ -f "$setup" ] || { echo "build-windows-x64: $setup was not produced"; exit 1; }
rm -rf "$out"
mkdir -p "$out"
# Artefact secret scan and the Vulkan import (scripts/lib/artefact-checks.sh).
. scripts/lib/artefact-checks.sh
voltip_scan_provider_keys "$exe" build-windows-x64
voltip_output_has 'Name: vulkan-1\.dll' llvm-readobj --coff-imports "$exe" || { echo "build-windows-x64: voltip-desktop.exe does not import vulkan-1.dll (built without the Vulkan backend)"; exit 1; }
# The DLLs the exe loads from its own directory: the dynamic sherpa-onnx of the offline recogniser
# (its build script drops them next to the exe in the target dir) and the Vulkan loader. The NSIS
# bundle ships them through `bundle.resources` (tauri.windows.conf.json and the Vulkan overlay); the
# portable zip carries them beside the exe.
resources=apps/desktop/src-tauri/resources/windows
RUNTIME=("$release/sherpa-onnx-c-api.dll" "$release/onnxruntime.dll" "$release/onnxruntime_providers_shared.dll" "$resources/vulkan-1.dll" "$resources/vulkan-1-LICENSE.txt" "$resources/../THIRD-PARTY-NOTICES.txt")
for f in "${RUNTIME[@]}"; do
  [ -f "$f" ] || { echo "build-windows-x64: missing $f"; exit 1; }
done
cp "$exe" "$setup" "${RUNTIME[@]}" "$out"/
portable=${product}_${version}_x64-portable
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$portable"
cp "$exe" "${RUNTIME[@]}" "$stage/$portable/"
out_abs=$(cd "$out" && pwd)
(cd "$stage" && zip -q -r -X "$out_abs/$portable.zip" "$portable")
(cd "$out" && sha256sum ./*.exe ./*.dll ./*.zip > SHA256SUMS.txt && cat SHA256SUMS.txt)
echo "build-windows-x64: OK → $out (commit $(git rev-parse --short HEAD))"
