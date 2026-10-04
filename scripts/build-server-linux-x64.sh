#!/usr/bin/env bash
# Build voltip-server, the headless local speech service (docs/dictation.md §23.5), as a Linux x64
# tar.gz and record what was built: bin/voltip-server, lib/ with the sherpa-onnx runtime and the
# Vulkan loader (the binary's RUNPATH is `$ORIGIN:$ORIGIN/../lib`, apps/server/build.rs), the licence,
# the third-party notices (the desktop's: its dependency graph holds the server's) and a systemd
# user unit example. No GTK, WebKit or audio library: what the binary takes from the system is
# VOLTIP_SERVER_SONAMES (scripts/lib/artefact-checks.sh), and install.sh --server checks for it.
# The package goes to dist/server-linux-x64 (git-ignored) with SHA256SUMS.txt and the record,
# build-info.txt; release-candidate.yml ships it as the Linux leg's extra asset.
# Usage: scripts/build-server-linux-x64.sh [out-dir] [build-info]
# VOLTIP_BUILD_REUSE=1 skips the cargo build and re-packages target/release/voltip-server.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo cargo-about node readelf ldd sha256sum tar strings; do
  command -v "$tool" >/dev/null || { echo "build-server-linux-x64: $tool not installed"; exit 2; }
done
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/require-builtin-engines.sh && voltip_require_builtin_engines
. scripts/lib/head-commit.sh
# TRANSCRIBE_CMAKE_ARGS for the Vulkan backend and VOLTIP_VULKAN_LIB_DIR, whose loader the package ships.
. scripts/lib/vulkan-sdk.sh && voltip_vulkan_env linux
version=$(node -p 'require("./package.json").version')
name=voltip-server-${version}-linux-x64
out=${1:-dist/server-linux-x64}
info=${2:-$out/build-info.txt}

if [ -z "${VOLTIP_BUILD_REUSE:-}" ]; then cargo build --release --locked -p voltip-server --features gpu-vulkan; fi
exe=target/release/voltip-server
[ -x "$exe" ] || { echo "build-server-linux-x64: $exe was not produced"; exit 1; }
# Artefact secret scan, the Vulkan linkage, and nothing taken from the system the package does not
# ship or check for (scripts/lib/artefact-checks.sh).
. scripts/lib/artefact-checks.sh
voltip_scan_provider_keys "$exe" build-server-linux-x64
voltip_output_has 'NEEDED.*\[libvulkan\.so\.1\]' readelf -d "$exe" || { echo "build-server-linux-x64: voltip-server does not link libvulkan.so.1 (built without the Vulkan backend)"; exit 1; }
voltip_linux_sonames_accounted "$exe" build-server-linux-x64 VOLTIP_SERVER_SONAMES || exit 1

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage=$work/stage/$name
mkdir -p "$stage/bin" "$stage/lib"
cp "$exe" "$stage/bin/voltip-server"
for lib in libsherpa-onnx-c-api.so libonnxruntime.so; do
  cp -L "target/release/$lib" "$stage/lib/$lib"
done
cp -L "$VOLTIP_VULKAN_LIB_DIR/libvulkan.so.1" "$stage/lib/libvulkan.so.1"
cp LICENSE "$stage/LICENSE"
python3 scripts/release/third-party-notices.py --out "$stage/THIRD-PARTY-NOTICES.txt"
cp apps/server/voltip-server.service "$stage/voltip-server.service.example"
# A reproducible archive: sorted names, root-owned, the commit's time on every file.
stamp=$(git log -1 --format=%ct 2>/dev/null || date +%s)
rm -rf "$out"
mkdir -p "$out" "$(dirname "$info")"
tar -C "$work/stage" --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$stamp" -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" >SHA256SUMS.txt)

# Unpack the archive elsewhere and run it headless: every library resolves, the sherpa-onnx runtime
# and the Vulkan loader come from the package (LD_LIBRARY_PATH unset: only the RUNPATH counts).
check=$work/check
mkdir -p "$check" "$work/data"
tar -C "$check" -xzf "$out/$name.tar.gz"
packaged=$check/$name/bin/voltip-server
run() { env -u DISPLAY -u WAYLAND_DISPLAY -u LD_LIBRARY_PATH XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/config" RUST_LOG=off timeout 60 "$packaged" "$@"; }

report() {
  local bad=0 resolved missing loaded models compute
  echo "packaged: $(date -u -r "$out/$name.tar.gz" +%Y-%m-%dT%H:%M:%SZ) commit=$(voltip_head_commit) host=$(uname -srm) $(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release)"
  echo "toolchain: $(rustc --version)"
  echo "command: cargo build --release --locked -p voltip-server --features gpu-vulkan${VOLTIP_BUILD_REUSE:+ — this record re-packaged the binary already in target/release}"
  echo "vulkan: SDK $VOLTIP_VULKAN_SDK_VERSION (link); the package ships lib/libvulkan.so.1, the GPU driver brings its own ICD"
  echo
  echo "$name.tar.gz: $(stat -c %s "$out/$name.tar.gz") bytes  sha256 $(sha256sum "$out/$name.tar.gz" | cut -d' ' -f1)"
  echo "contents:"
  tar -tzvf "$out/$name.tar.gz" | awk '{ print "  " $6 " (" $3 " bytes)" }'
  echo
  resolved=$(env -u LD_LIBRARY_PATH ldd "$packaged")
  missing=$(printf '%s\n' "$resolved" | grep -c "not found" || true)
  echo "bin/voltip-server: RUNPATH=$(readelf -d "$packaged" | sed -n 's/.*(RUNPATH).*\[\(.*\)\]/\1/p')"
  echo "  from the package: $(printf '%s\n' "$resolved" | awk -v r="$check" '$3 ~ "^"r { sub(r"/[^/]*/", "", $3); printf "%s ", $1 }')"
  echo "  from the system: $(printf '%s\n' "$resolved" | awk -v r="$check" '$2 == "=>" && $3 !~ "^"r { print $1 }' | sort | tr '\n' ' ')"
  echo "  not found: $missing"
  [ "$missing" = 0 ] || bad=1
  echo
  echo "runtime (headless, no DISPLAY, no LD_LIBRARY_PATH):"
  if version_line=$(run --version 2>/dev/null) && [ "$version_line" = "voltip-server $version" ]; then
    echo "  --version: $version_line"
  else
    echo "  --version FAILED (${version_line:-no output})"
    bad=1
  fi
  if models=$(run --list-models 2>/dev/null); then
    echo "  --list-models: exit 0, $(printf '%s\n' "$models" | grep -c .) catalogue entries"
  else
    echo "  --list-models FAILED"
    bad=1
  fi
  loaded=$(env -u DISPLAY -u LD_LIBRARY_PATH LD_DEBUG=libs XDG_DATA_HOME="$work/data" RUST_LOG=off timeout 60 "$packaged" --list-models 2>&1 >/dev/null | sed -n 's/.*calling init: //p' | grep -E 'sherpa|onnxruntime|vulkan' | sed "s#$check/##" || true)
  [ -n "$loaded" ] && printf '%s\n' "$loaded" | sed 's/^/    loaded /'
  if printf '%s\n' "$loaded" | grep -Eq "$name/(bin/\.\./)?lib/libsherpa-onnx-c-api\.so"; then :; else
    echo "  the sherpa-onnx runtime did not come from the package"
    bad=1
  fi
  if compute=$(run --list-compute 2>/dev/null); then
    echo "  --list-compute: $(printf '%s\n' "$compute" | tr '\t' ' ' | paste -sd ';' -)"
  else
    echo "  --list-compute FAILED"
    bad=1
  fi
  return "$bad"
}

status=0
report >"$info" || status=1
cat "$info"
[ "$status" = 0 ] || { echo "build-server-linux-x64: the package does not run as built (see $info)"; exit 1; }
echo "build-server-linux-x64: OK → $out/$name.tar.gz ($(voltip_head_commit)), record → $info"
