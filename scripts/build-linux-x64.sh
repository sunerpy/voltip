#!/usr/bin/env bash
# Build the Linux x64 desktop packages (deb, rpm, AppImage) with the Tauri CLI and record what was
# built (docs/dictation.md §14): sizes, sha256, package metadata, and an ldd of the packaged
# executable against the sherpa-onnx runtime the package ships in /usr/lib/voltip (the binary's
# RUNPATH is `$ORIGIN:$ORIGIN/../lib/voltip`, apps/desktop/src-tauri/build.rs). The local models run
# on a GPU through Vulkan and fall back to the CPU (docs/dictation.md §10.6): scripts/lib/vulkan-sdk.sh
# fetches the SDK; the deb and rpm depend on the system's loader, the AppImage carries its own.
# Packages go to dist/linux-x64 (git-ignored) with SHA256SUMS.txt and the record, build-info.txt.
# Usage: scripts/build-linux-x64.sh [out-dir] [build-info]
# VOLTIP_BUILD_NOTE adds one free-text line to the record (e.g. "uncommitted B5 changes on top").
# VOLTIP_BUILD_REUSE=1 skips `cargo tauri build` and re-checks the bundles already in target/.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo cargo-about pnpm dpkg-deb rpm2cpio cpio patchelf readelf ldd sha256sum strings; do
  command -v "$tool" >/dev/null || { echo "build-linux-x64: $tool not installed"; exit 2; }
done
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/require-builtin-engines.sh && voltip_require_builtin_engines
. scripts/lib/head-commit.sh
# TRANSCRIBE_CMAKE_ARGS / VOLTIP_VULKAN_LIB_DIR for the Vulkan backend, and the loader the AppImage
# ships staged into src-tauri/resources/linux/ for tauri.linux.conf.json.
. scripts/lib/vulkan-sdk.sh && voltip_vulkan_env linux
# The packages carry the third-party licence texts (/usr/share/doc/voltip/THIRD-PARTY-NOTICES.txt).
python3 scripts/release/third-party-notices.py --out apps/desktop/src-tauri/resources/THIRD-PARTY-NOTICES.txt
out=${1:-dist/linux-x64}
info=${2:-$out/build-info.txt}

if [ -z "${VOLTIP_BUILD_REUSE:-}" ]; then (cd apps/desktop && cargo tauri build --ci --bundles deb,rpm,appimage --features gpu-vulkan); fi
bundle=target/release/bundle
# Only this version's bundles (package.json): the bundle directory keeps those of earlier builds.
version=$(node -p 'require("./package.json").version')
deb=$bundle/deb/Voltip_${version}_amd64.deb
rpm=$bundle/rpm/Voltip-${version}-1.x86_64.rpm
appimage=$bundle/appimage/Voltip_${version}_amd64.AppImage
for f in "$deb" "$rpm" "$appimage"; do
  [ -f "$f" ] || { echo "build-linux-x64: $f was not produced"; exit 1; }
done
# Artefact secret scan and the Vulkan linkage (scripts/lib/artefact-checks.sh).
. scripts/lib/artefact-checks.sh
voltip_scan_provider_keys target/release/voltip-desktop build-linux-x64
voltip_output_has 'NEEDED.*\[libvulkan\.so\.1\]' readelf -d target/release/voltip-desktop || { echo "build-linux-x64: voltip-desktop does not link libvulkan.so.1 (built without the Vulkan backend)"; exit 1; }
rm -rf "$out"
mkdir -p "$out" "$(dirname "$info")"
cp "$deb" "$rpm" "$appimage" "$out"/
(cd "$out" && sha256sum ./*.deb ./*.rpm ./*.AppImage > SHA256SUMS.txt)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The installed trees, unpacked without installing anything.
mkdir -p "$work/deb" "$work/rpm"
dpkg-deb -x "$deb" "$work/deb"
# Ubuntu 24.04's rpm2cpio (rpm 4.18) exits 1 on the rpm-rs packages Tauri writes although the
# payload it prints is complete (`rpm -K`: digests OK); the extracted tree is what gets checked.
rpm2cpio_rc=0
rpm2cpio "$rpm" >"$work/rpm.cpio" || rpm2cpio_rc=$?
(cd "$work/rpm" && cpio -idm --quiet <"$work/rpm.cpio")
[ -x "$work/rpm/usr/bin/voltip-desktop" ] || { echo "build-linux-x64: the rpm payload has no usr/bin/voltip-desktop (rpm2cpio exit $rpm2cpio_rc)"; exit 1; }
(cd "$work" && "$OLDPWD/$appimage" --appimage-extract >/dev/null)
appdir=$work/squashfs-root

# ldd of one tree's executable: every library resolved, and which ones come from the tree itself.
ldd_report() { # <label> <root> <binary-in-root>
  local label=$1 root=$2 exe=$2/$3 resolved missing own
  resolved=$(ldd "$exe")
  missing=$(printf '%s\n' "$resolved" | grep -c "not found" || true)
  own=$(printf '%s\n' "$resolved" | awk -v r="$root" '$3 ~ "^"r { sub(r, "", $3); print "    " $1 " => " $3 }')
  echo "$label: $3 RUNPATH=$(readelf -d "$exe" | sed -n 's/.*(RUNPATH).*\[\(.*\)\]/\1/p')"
  echo "  libraries: $(printf '%s\n' "$resolved" | grep -c '=>' || true) resolved through the loader, not found: $missing"
  if [ "$(printf '%s\n' "$own" | grep -c .)" -le 8 ]; then
    echo "  from the package:"
    printf '%s\n' "${own:-    (none)}"
  else
    echo "  from the package: $(printf '%s\n' "$own" | grep -c .) libraries, among them:"
    printf '%s\n' "$own" | grep -E "sherpa|onnxruntime|vulkan|webkit2gtk|gtk-3" || true
  fi
  echo "  from the system: $(printf '%s\n' "$resolved" | awk -v r="$root" '$2 == "=>" && $3 !~ "^"r { print $1 }' | sort | tr '\n' ' ')"
  [ "$missing" = 0 ]
}

report() {
  local bad=0
  echo "bundled: $(date -u -r "$deb" +%Y-%m-%dT%H:%M:%SZ) checked: $(date -u +%Y-%m-%dT%H:%M:%SZ) commit=$(voltip_head_commit) host=$(uname -srm) $(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release)"
  [ -n "${VOLTIP_BUILD_NOTE:-}" ] && echo "note: $VOLTIP_BUILD_NOTE"
  echo "toolchain: $(rustc --version) · $(cargo tauri --version) · webkit2gtk $(pkg-config --modversion webkit2gtk-4.1 2>/dev/null || echo '?')"
  echo "command: (cd apps/desktop && cargo tauri build --ci --bundles deb,rpm,appimage --features gpu-vulkan)${VOLTIP_BUILD_REUSE:+ — this record re-checked the bundles already in target/release/bundle}"
  echo "vulkan: SDK $VOLTIP_VULKAN_SDK_VERSION (link), the deb / rpm take the system's libvulkan.so.1, the AppImage ships usr/lib/voltip/libvulkan.so.1"
  echo
  for f in "$deb" "$rpm" "$appimage"; do
    echo "$(basename "$f"): $(stat -c %s "$f") bytes  sha256 $(sha256sum "$f" | cut -d' ' -f1)"
  done
  echo
  echo "deb control:"
  dpkg-deb -f "$deb" Package Version Architecture Depends Recommends | sed 's/^/  /'
  echo "deb files (outside usr/share):"
  dpkg-deb -c "$deb" | awk '$1 !~ /^d/ && $6 !~ /^(\.\/)?usr\/share\// { print "  " $6 " (" $3 " bytes)" }'
  if command -v rpm >/dev/null; then
    echo "rpm requires (rpm -qpR; $(rpm -K "$rpm" 2>&1 | sed 's/.*: //')):"
    rpm -qpR "$rpm" 2>/dev/null | grep -v '^rpmlib(' | sed 's/^/  /'
    echo "rpm recommends: $(rpm -qp --recommends "$rpm" 2>/dev/null | tr '\n' ' ')"
  else
    echo "rpm requires: (rpm not installed here; tauri.linux.conf.json + the CLI's webkit2gtk / gtk3 requires)"
  fi
  echo "rpm payload: rpm2cpio exit $rpm2cpio_rc, extracted tree checked below"
  echo
  ldd_report "deb tree" "$work/deb" usr/bin/voltip-desktop || bad=1
  ldd_report "rpm tree" "$work/rpm" usr/bin/voltip-desktop || bad=1
  ldd_report "AppImage tree" "$appdir" usr/bin/voltip-desktop || bad=1
  echo "AppImage bundles $(find "$appdir/usr/lib" -name '*.so*' | wc -l) shared objects (linuxdeploy + gtk plugin); linuxdeploy rewrote the RUNPATH and deployed the sherpa-onnx pair into usr/lib next to the usr/lib/voltip copies"
  return "$bad"
}
# Start the packaged executables headless (`--list-models`: no window, no display) and show where
# the loader took the sherpa-onnx runtime from: proof that RUNPATH and the packaged files agree.
runtime_check() { # <label> <executable> [env…]
  local label=$1 exe=$2 out loaded
  shift 2
  if out=$(env -u DISPLAY -u WAYLAND_DISPLAY "$@" XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/config" RUST_LOG=off timeout 60 "$exe" --list-models 2>/dev/null); then
    echo "  $label: --list-models exit 0, $(printf '%s\n' "$out" | grep -c .) catalogue entries"
  else
    echo "  $label: --list-models FAILED"
    return 1
  fi
  loaded=$(env -u DISPLAY -u WAYLAND_DISPLAY "$@" LD_DEBUG=libs XDG_DATA_HOME="$work/data" RUST_LOG=off timeout 60 "$exe" --list-models 2>&1 >/dev/null | sed -n 's/.*calling init: //p' | grep -E 'sherpa|onnxruntime|vulkan' | sed "s#$work/##; s#/tmp/appimage_extracted_[^/]*/#<AppImage mount>/#" || true)
  [ -n "$loaded" ] && printf '%s\n' "$loaded" | sed 's/^/    loaded /'
  # The compute devices the Vulkan backend sees here (none without a Vulkan GPU driver: CPU only).
  if out=$(env -u DISPLAY -u WAYLAND_DISPLAY "$@" XDG_DATA_HOME="$work/data" RUST_LOG=off timeout 60 "$exe" --list-compute 2>/dev/null); then
    echo "  $label: --list-compute: $(printf '%s\n' "$out" | tr '\t' ' ' | paste -sd ';' -)"
  else
    echo "  $label: --list-compute FAILED"
    return 1
  fi
  return 0
}

report_runtime() {
  local bad=0
  echo
  echo "runtime (headless, no DISPLAY / WAYLAND_DISPLAY):"
  runtime_check "deb tree" "$work/deb/usr/bin/voltip-desktop" || bad=1
  runtime_check "AppImage" "$PWD/$appimage" APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR="$work" || bad=1
  return "$bad"
}

status=0
{ report && report_runtime; } >"$info" || status=1
cat "$info"
[ "$status" = 0 ] || { echo "build-linux-x64: an executable has unresolved libraries (see $info)"; exit 1; }
echo "build-linux-x64: OK → $out ($(voltip_head_commit)), record → $info"
