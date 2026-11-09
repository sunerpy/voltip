# shellcheck shell=bash
# The files tauri-bundler downloads into its tools directory while it bundles, placed there first
# through .github/scripts/fetch-pinned.sh (retried for a bounded time, kept only with the SHA-256
# pinned here). Sourced by scripts/build-windows-x64.sh, which ci.yml's windows-cross job and the
# Windows leg of release-candidate.yml run, and by scripts/lib/appimage.sh.
#
# The bundler downloads each of these files with a single GET and no retry (`download` in
# utils/http_utils.rs). On 2026-11-09 the cross-build of main stopped there: "Downloading
# …/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll", then "failed to bundle project: `http status:
# 504`" (run 42567630080, job 128997576493); GitHub answered 200 for the same address minutes later.
#
# What the bundler of tauri-cli 2.12.4 (tauri-bundler 2.10.4) downloads at bundle time, into
# dirs::cache_dir()/tauri (voltip_tauri_tools_dir; bundle.useLocalToolsDir, unset in this
# repository, would move it to <target>/.tauri):
#   - NSIS, off Windows (bundle/windows/nsis/mod.rs): NSIS/Plugins/x86-unicode/additional/
#     nsis_tauri_utils.dll from nsis-tauri-utils' release nsis_tauri_utils-v0.5.3, checked against
#     NSIS_TAURI_UTILS_SHA1. makensis is the system's; the NSIS toolset nsis-3.13.zip (NSIS_SHA1)
#     is downloaded on a Windows host only. Yet before every NSIS bundle the bundler checks
#     NSIS/NSIS.exe against NSIS_EXE_SHA1 on every host, and off Windows nothing ever writes that
#     file: the check failed each time, and the bundler removed NSIS/ and downloaded the dll again
#     ("NSIS.exe is missing. Recreating it." before the second bundle of the release candidate's
#     Windows leg, run 42514933736). voltip_tauri_nsis_tools places NSIS.exe, taken from
#     nsis-3.13.zip, and the dll; then the bundler compares the dll's SHA-1 and downloads nothing.
#   - The WebView2 bootstrapper (bundle/windows/util.rs), which webviewInstallMode embedBootstrapper
#     (tauri.windows.conf.json) packs into the installer: MicrosoftEdgeWebview2Setup.exe from
#     https://go.microsoft.com/fwlink/p/?LinkId=2124703, once, when the file is missing, with no
#     checksum. Not placed here: no pinned copy stays downloadable. Microsoft replaces the file
#     behind the link (on 2026-11-09 it served version 1.3.315.17, built 2026-10-29) and removes old
#     ones from its download host (the addresses tauri 1.4 and 1.5 pinned answer 404).
#   - AppImage: AppRun and linuxdeploy, placed by scripts/lib/appimage.sh; the optional
#     linuxdeploy-plugin-appimage and the runtime appimagetool downloads are not (see there).
#   - deb, rpm, app and dmg: nothing.
# The values below were read from that bundler's source; test_tauri_tools.py fails when
# install-cargo-tauri.sh installs another tauri-cli, until they are read again from its bundler.

VOLTIP_TAURI_TOOLS_CLI_VERSION=2.12.4

# NSIS.exe: the member of tauri-apps/binary-releases' nsis-3.13.zip whose SHA-1 is NSIS_EXE_SHA1.
# The zip's SHA-1 is the bundler's NSIS_SHA1; its SHA-256 is the digest GitHub lists for the asset.
VOLTIP_NSIS_ZIP_URL=https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip
VOLTIP_NSIS_ZIP_SHA256=ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8
VOLTIP_NSIS_ZIP_SHA1=db14f8af2ea346b786c6ae19343f61aaf5e09b8e
VOLTIP_NSIS_EXE_MEMBER=nsis-3.13/NSIS.exe
VOLTIP_NSIS_EXE_SHA256=78e17de55fa013be7620487e85b6676c204c6c51d7ebabb74ee8a9cfce6252a0
VOLTIP_NSIS_EXE_SHA1=6d45d4e8ccf18fe1fdac9164058f356102a2ecf2
# The NSIS plugin the installer template calls (NSIS_TAURI_UTILS_URL, NSIS_TAURI_UTILS_SHA1; the
# SHA-256 is the digest GitHub lists for the asset, 34304 bytes).
VOLTIP_NSIS_TAURI_UTILS_URL=https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll
VOLTIP_NSIS_TAURI_UTILS_SHA256=5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709
VOLTIP_NSIS_TAURI_UTILS_SHA1=75197fee3c6a814fe035788d1c34ead39349b860

# fetch-pinned.sh beside this library's repository, wherever it was sourced from and whatever the
# directory is when a function runs (the candidate's legs source .release-tooling/scripts/lib/).
VOLTIP_FETCH_PINNED="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.github/scripts" && pwd)/fetch-pinned.sh"

# Print the bundler's tools directory: dirs::cache_dir() joined with tauri, that is
# $XDG_CACHE_HOME/tauri when XDG_CACHE_HOME is an absolute path, else $HOME/.cache/tauri (Linux).
voltip_tauri_tools_dir() {
  case ${XDG_CACHE_HOME:-} in
    /*) printf '%s/tauri\n' "$XDG_CACHE_HOME" ;;
    *)
      [ -n "${HOME:-}" ] || { echo "tauri-tools: HOME is not set, so the bundler's tools directory is unknown" >&2; return 1; }
      printf '%s/.cache/tauri\n' "$HOME"
      ;;
  esac
}

# Print <file>'s SHA-1 or SHA-256 in lowercase hex.
voltip_tauri_digest() { # <1|256> <file>
  local out
  if command -v shasum >/dev/null 2>&1; then
    out=$(shasum -a "$1" <"$2") || return 1
  else
    out=$("sha$1sum" <"$2") || return 1
  fi
  printf '%s\n' "${out%% *}"
}

# True when <file> is a regular file with both digests.
voltip_tauri_holds() { # <file> <sha256> <sha1>
  [ -f "$1" ] && [ ! -L "$1" ] || return 1
  [ "$(voltip_tauri_digest 256 "$1")" = "$2" ] && [ "$(voltip_tauri_digest 1 "$1")" = "$3" ]
}

# Fail, naming <label>, unless <file> has the pinned SHA-256 and the SHA-1 tauri-bundler checks.
voltip_tauri_check() { # <file> <sha256> <sha1> <label>
  local have
  have=$(voltip_tauri_digest 256 "$1") || return 1
  if [ "$have" != "$2" ]; then
    echo "tauri-tools: $4 has SHA-256 $have, pinned $2" >&2
    return 1
  fi
  have=$(voltip_tauri_digest 1 "$1") || return 1
  if [ "$have" != "$3" ]; then
    echo "tauri-tools: $4 has SHA-1 $have, not the $3 tauri-bundler checks: the pins in scripts/lib/tauri-tools.sh disagree" >&2
    return 1
  fi
}

# Put <src> at <dest>, mode 0644, by a rename beside <dest>.
voltip_tauri_install() { # <src> <dest>
  local part
  if [ -L "$2" ] || { [ -e "$2" ] && [ ! -f "$2" ]; }; then
    echo "tauri-tools: $2 is not a regular file — look at it and remove it" >&2
    return 1
  fi
  mkdir -p "$(dirname "$2")" || return 1
  part=$2.part.$$
  if ! cp "$1" "$part" || ! chmod 0644 "$part" || ! mv -f "$part" "$2"; then
    rm -f "$part"
    return 1
  fi
}

# Leave <tools>/NSIS holding what tauri-bundler checks there off Windows, each file with the
# SHA-256 pinned above and the SHA-1 the bundler compares: NSIS.exe, unzipped from nsis-3.13.zip,
# and Plugins/x86-unicode/additional/nsis_tauri_utils.dll. A file already there with both digests
# is kept and nothing is downloaded; one with other bytes is replaced, as the bundler would replace
# it (it removes an NSIS.exe it does not know and downloads a dll with another SHA-1 again). Call
# it before `cargo tauri build` or `cargo tauri bundle` bundles NSIS.
voltip_tauri_nsis_tools() {
  local dir exe dll tmp
  dir=$(voltip_tauri_tools_dir) || return 1
  exe=$dir/NSIS/NSIS.exe
  dll=$dir/NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll
  tmp=$(mktemp -d) || return 1
  if ! voltip_tauri_holds "$exe" "$VOLTIP_NSIS_EXE_SHA256" "$VOLTIP_NSIS_EXE_SHA1"; then
    if ! command -v unzip >/dev/null; then
      echo "tauri-tools: unzip is not installed (NSIS.exe comes out of $VOLTIP_NSIS_ZIP_URL)" >&2
      rm -rf "$tmp"
      return 1
    fi
    if ! bash "$VOLTIP_FETCH_PINNED" "$VOLTIP_NSIS_ZIP_URL" "$VOLTIP_NSIS_ZIP_SHA256" "$tmp/nsis.zip" ||
      ! voltip_tauri_check "$tmp/nsis.zip" "$VOLTIP_NSIS_ZIP_SHA256" "$VOLTIP_NSIS_ZIP_SHA1" "$VOLTIP_NSIS_ZIP_URL"; then
      rm -rf "$tmp"
      return 1
    fi
    if ! unzip -p "$tmp/nsis.zip" "$VOLTIP_NSIS_EXE_MEMBER" >"$tmp/NSIS.exe" 2>/dev/null; then
      echo "tauri-tools: $VOLTIP_NSIS_ZIP_URL has no $VOLTIP_NSIS_EXE_MEMBER" >&2
      rm -rf "$tmp"
      return 1
    fi
    if ! voltip_tauri_check "$tmp/NSIS.exe" "$VOLTIP_NSIS_EXE_SHA256" "$VOLTIP_NSIS_EXE_SHA1" "$VOLTIP_NSIS_EXE_MEMBER" ||
      ! voltip_tauri_install "$tmp/NSIS.exe" "$exe"; then
      rm -rf "$tmp"
      return 1
    fi
  fi
  if ! voltip_tauri_holds "$dll" "$VOLTIP_NSIS_TAURI_UTILS_SHA256" "$VOLTIP_NSIS_TAURI_UTILS_SHA1"; then
    if ! bash "$VOLTIP_FETCH_PINNED" "$VOLTIP_NSIS_TAURI_UTILS_URL" "$VOLTIP_NSIS_TAURI_UTILS_SHA256" "$tmp/nsis_tauri_utils.dll" ||
      ! voltip_tauri_check "$tmp/nsis_tauri_utils.dll" "$VOLTIP_NSIS_TAURI_UTILS_SHA256" "$VOLTIP_NSIS_TAURI_UTILS_SHA1" "$VOLTIP_NSIS_TAURI_UTILS_URL" ||
      ! voltip_tauri_install "$tmp/nsis_tauri_utils.dll" "$dll"; then
      rm -rf "$tmp"
      return 1
    fi
  fi
  rm -rf "$tmp"
  echo "tauri-tools: $dir/NSIS holds NSIS.exe and nsis_tauri_utils.dll with the digests tauri-bundler checks (tauri-cli $VOLTIP_TAURI_TOOLS_CLI_VERSION)"
}
