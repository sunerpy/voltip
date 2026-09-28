#!/usr/bin/env bash
# Make cargo run sherpa-onnx-sys's build script again after a rust-cache restore.
#
# Swatinem/rust-cache keeps a dependency's fingerprint (`.fingerprint/`, `build/`) but prunes what
# its build script left anywhere else in target/: the prebuilt runtime sherpa-onnx-sys downloads
# (`target/sherpa-onnx-prebuilt/`) and the copies it puts next to the binaries
# (`target/<triple>/<profile>/*.dll`, `*.so`, `*.dylib`). Cargo then takes the build script as
# fresh, never runs it again, and the package steps find no runtime to stage ("resource path
# `resources/windows/onnxruntime_providers_shared.dll` doesn't exist", CI 2026-09-27). Deleting its
# fingerprints and build directories makes the next cargo invocation run it again. The pruned
# download goes too: rust-cache deletes its files but leaves the directories, and the build script
# takes an existing directory for an extracted archive ("No shared runtime libraries found in
# …/sherpa-onnx-prebuilt/<archive>/lib"), so it has to download the archive again.
#
# Usage: .github/scripts/forget-sherpa-onnx-build.sh [target-dir]   (default $CARGO_TARGET_DIR or target)
set -euo pipefail
cd "$(dirname "$0")/../.."
target=${1:-${CARGO_TARGET_DIR:-target}}
if [ ! -d "$target" ]; then
  echo "forget-sherpa-onnx-build: no $target yet (cold cache), nothing to forget"
  exit 0
fi
count=0
while IFS= read -r -d '' dir; do
  rm -rf -- "$dir"
  count=$((count + 1))
done < <(find "$target" -type d -name 'sherpa-onnx-sys-*' \( -path '*/.fingerprint/*' -o -path '*/build/*' \) -prune -print0)
rm -rf -- "$target/sherpa-onnx-prebuilt"
echo "forget-sherpa-onnx-build: removed $count sherpa-onnx-sys build and fingerprint directories and the prebuilt download under $target"
