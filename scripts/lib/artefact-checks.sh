# shellcheck shell=bash
# Checks on a built binary, shared by scripts/build-{linux,windows}-x64.sh and
# release-candidate.yml (the macOS leg too: `strings -a` makes Apple's strings read the whole
# Mach-O, not just __TEXT).
#
# `grep` reads the whole stream here on purpose: under `set -o pipefail`, `grep -q` stops at the
# first match, the producer (`strings`, `readelf`) dies of SIGPIPE, and the pipeline counts as
# failed. The provider-key scan then passed exactly when a key was found early in a large binary.

# Fail when a provider API key (Groq `gsk_…`, OpenAI `sk-…`) is baked into <file>. The client may
# carry the revocable app token for the built-in edge; provider keys live on the edge hosts.
# voltip_scan_provider_keys <file> <label>
voltip_scan_provider_keys() {
  # A binary that is not there is not clean: strings would fail and the pipeline read as "no key".
  [ -f "$1" ] && [ -r "$1" ] || {
    echo "$2: $1 is missing or unreadable — nothing to scan" >&2
    return 1
  }
  if strings -a -n 20 "$1" | grep -E "gsk_[A-Za-z0-9]{20,}|sk-[A-Za-z0-9]{32,}" >/dev/null; then
    echo "$2: provider API key found inside $1 — refusing to ship" >&2
    return 1
  fi
}

# Whether the output of a command matches an extended regex, read to the end:
# voltip_output_has <regex> <command> [args…]
voltip_output_has() {
  local regex=$1
  shift
  "$@" | grep -E -- "$regex" >/dev/null
}

# The shared libraries voltip-desktop may take from a Linux system: each one is named by the deb
# and rpm depends in apps/desktop/src-tauri/tauri.linux.conf.json (or pulled in by them), shipped
# in the package, or part of every system. 0.0.4's binary linked libblas.so.3, which the build
# machine had and the depends did not name, so the installed deb did not start; a soname outside
# this list now stops the build until the depends name its package.
VOLTIP_LINUX_SONAMES=(
  # libwebkit2gtk-4.1-0 and libgtk-3-0, and what they pull in
  libwebkit2gtk-4.1.so.0 libjavascriptcoregtk-4.1.so.0 libgtk-3.so.0 libgdk-3.so.0 libcairo.so.2
  libgdk_pixbuf-2.0.so.0 libsoup-3.0.so.0 libgio-2.0.so.0 libgobject-2.0.so.0 libglib-2.0.so.0
  # libxkbcommon0, libasound2t64 | libasound2, libvulkan1, libblas3 | libblas.so.3
  libxkbcommon.so.0 libasound.so.2 libvulkan.so.1 libblas.so.3
  # shipped in /usr/lib/voltip
  libsherpa-onnx-c-api.so
  # the C and C++ runtime
  libc.so.6 libm.so.6 libgcc_s.so.1 libstdc++.so.6 ld-linux-x86-64.so.2
)

# Fail when <exe> links a shared library outside VOLTIP_LINUX_SONAMES:
# voltip_linux_sonames_accounted <exe> <label>
voltip_linux_sonames_accounted() {
  local exe=$1 label=$2 soname known needed=0 unexpected=()
  [ -f "$exe" ] || {
    echo "$label: $exe is missing" >&2
    return 1
  }
  while IFS= read -r soname; do
    needed=$((needed + 1))
    for known in "${VOLTIP_LINUX_SONAMES[@]}"; do
      [ "$soname" = "$known" ] && continue 2
    done
    unexpected+=("$soname")
  done < <(readelf -d "$exe" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
  if [ "$needed" = 0 ]; then
    echo "$label: $exe names no shared library (not a dynamic ELF?)" >&2
    return 1
  fi
  if [ "${#unexpected[@]}" -gt 0 ]; then
    echo "$label: $exe links ${unexpected[*]}, which no Linux package depends on: name the package in the deb and rpm depends of tauri.linux.conf.json, then add the soname to VOLTIP_LINUX_SONAMES" >&2
    return 1
  fi
}
