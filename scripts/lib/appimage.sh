# shellcheck shell=bash
# The AppRun the AppImage starts through and the linuxdeploy that builds it, placed in
# tauri-bundler's tools directory before it bundles, and a check that every user can read and run
# what the AppImage carries. Sourced by scripts/build-linux-x64.sh and the Linux leg of
# release-candidate.yml.
#
# tauri-bundler (2.9.4 in tauri-cli 2.11.4 and 2.11.5, 2.10.4 in tauri-cli 2.12.4) downloads
# AppRun-x86_64 into its tools directory, writes it with mode 0770 (`write_and_make_executable`)
# and copies it into the AppDir with that mode; linuxdeploy keeps it as AppRun.wrapped. The
# squashfs records root as the owner, and where the AppImage is mounted as recorded (firejail
# --appimage, in the AppImage catalogue's test of 0.2.1) a user who is not root cannot run it:
# "AppRun.wrapped: Permission denied". The bundler downloads the file only when it is missing, so
# the pinned copy placed here first, with mode 0755, is the one it packages. The check stops a
# package that still carries an entry other users cannot read or run.
#
# The bundler also downloads linuxdeploy-07333c6 (bundle/linux/appimage/linuxdeploy.rs, from
# tauri-apps/binary-releases, when its tools directory has no linuxdeploy-07333c6-x86_64.AppImage,
# with no checksum), and every such download is a single try: the release candidate's Linux leg
# downloaded it while bundling (run 42514933736, job 128835826249), the way the Windows cross-build
# failed on a 504 for its NSIS plugin (run 42567630080, scripts/lib/tauri-tools.sh). Both files come
# through fetch-pinned.sh instead, retried for a bounded time. After every AppImage the bundler
# zeroes linuxdeploy's bytes 8-10 in its tools directory (`dd if=/dev/zero bs=1 count=3 seek=8
# conv=notrunc`, the AppImage magic), so the file it left is accepted too. Not placed here:
#   - the optional linuxdeploy-plugin-appimage, which the bundler fetches from that project's
#     `continuous` release: its bytes change with every upload (last on 2026-10-01), so no pin
#     would hold; when the download fails, linuxdeploy uses the plugin it carries.
#   - the AppImage runtime, which appimagetool inside either plugin downloads on every AppImage,
#     once, from AppImage/type2-runtime's `continuous` release ("Downloading runtime file from …",
#     in linuxdeploy's output, which the bundler shows only with --verbose). When that download
#     fails, the bundle stops with "failed to bundle project: `failed to run linuxdeploy`" and
#     nothing more; a new attempt is the remedy. The plugin takes a file instead through
#     LDAI_RUNTIME_FILE in the environment of the bundle step; which runtime the AppImage carries
#     is not this library's choice (kept as it is, 2026-11-09).

# shellcheck source=scripts/lib/tauri-tools.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/tauri-tools.sh"

VOLTIP_APPRUN_URL=https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64
VOLTIP_APPRUN_SHA256=f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f
# The digest GitHub lists for the asset, and the same bytes with 8-10 zeroed as the bundler leaves
# them: `{ head -c 8 f; head -c 3 /dev/zero; tail -c +12 f; } | sha256sum`.
VOLTIP_LINUXDEPLOY_URL=https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy-07333c6/linuxdeploy-x86_64.AppImage
VOLTIP_LINUXDEPLOY_FILE=linuxdeploy-07333c6-x86_64.AppImage
VOLTIP_LINUXDEPLOY_SHA256=36a2d7e274d12e1050d0e9ecfe11d339ed54720b2bec464c286d53f8b07f5c62
VOLTIP_LINUXDEPLOY_PATCHED_SHA256=bd9521cd5ff3ca351fecb78cd9b236a4e37b3621014ac05cac0c2d767ffa0474

# Leave <file> holding <sha256>'s bytes with mode 0755, downloading it from <url> through
# fetch-pinned.sh when it is missing. A file already there is kept when it has <sha256>, or
# <also> when given (the bytes the bundler leaves); with other contents it fails and stays as it
# is: the bundler packages or runs whatever its tools directory holds for the AppImage every user
# runs, so a stranger there is for someone to look at, not to replace quietly.
voltip_place_pinned_executable() { # <url> <sha256> <file> [<also>]
  local url=$1 sha256=$2 file=$3 also=${4:-} actual
  if [ -e "$file" ] || [ -L "$file" ]; then
    if [ -L "$file" ] || [ ! -f "$file" ]; then
      echo "appimage: $file is not a regular file — refusing to package it" >&2
      return 1
    fi
    actual=$(voltip_tauri_digest 256 "$file") || return 1
    if [ "$actual" != "$sha256" ] && { [ -z "$also" ] || [ "$actual" != "$also" ]; }; then
      echo "appimage: $file has SHA-256 $actual, not the pinned $sha256${also:+ or $also} — refusing to package it; look at it, delete it and build again to download the pinned one" >&2
      return 1
    fi
  else
    # fetch-pinned.sh writes beside the file and renames only the pinned bytes into place.
    echo "appimage: downloading $url" >&2
    bash "$VOLTIP_FETCH_PINNED" "$url" "$sha256" "$file" >&2 || return 1
  fi
  chmod 0755 "$file"
}

# Place the pinned AppRun-x86_64 and linuxdeploy, mode 0755, in the bundler's tools directory.
# Call it before `cargo tauri build` or `cargo tauri bundle` builds an AppImage.
voltip_appimage_apprun() {
  local dir
  dir=$(voltip_tauri_tools_dir) || return 1
  voltip_place_pinned_executable "$VOLTIP_APPRUN_URL" "$VOLTIP_APPRUN_SHA256" "$dir/AppRun-x86_64" || return 1
  echo "appimage: $dir/AppRun-x86_64 is the pinned AppRun, mode 0755"
  voltip_place_pinned_executable "$VOLTIP_LINUXDEPLOY_URL" "$VOLTIP_LINUXDEPLOY_SHA256" "$dir/$VOLTIP_LINUXDEPLOY_FILE" "$VOLTIP_LINUXDEPLOY_PATCHED_SHA256" || return 1
  echo "appimage: $dir/$VOLTIP_LINUXDEPLOY_FILE is the pinned linuxdeploy"
}

# Fail when an entry of <AppImage>'s file system other than a symlink cannot be read by every user,
# or can be run by its owner and not by every user (for a directory: entered). The runtime reports
# where the file system starts (--appimage-offset); unsquashfs lists it with numeric owners.
voltip_check_appimage_permissions() { # <AppImage> <label>
  local appimage=$1 label=$2 offset listing entries bad
  [ -f "$appimage" ] || {
    echo "$label: $appimage is missing — nothing to check" >&2
    return 1
  }
  case $appimage in */*) ;; *) appimage=./$appimage ;; esac
  if ! offset=$("$appimage" --appimage-offset) || ! [[ $offset =~ ^[0-9]+$ ]]; then
    echo "$label: $appimage did not report where its file system starts (--appimage-offset)" >&2
    return 1
  fi
  if ! listing=$(unsquashfs -lln -o "$offset" "$appimage"); then
    echo "$label: unsquashfs could not list $appimage from offset $offset" >&2
    return 1
  fi
  # Only the entry lines, whose first field is the mode; nothing else counts as an entry. A filter
  # that fails fails the check: callers write `… || exit 1`, and errexit does not apply in here.
  if ! entries=$(awk '$1 ~ /^[-dlbcps][-r][-w][-xsS][-r][-w][-xsS][-r][-w][-xtT]$/' <<<"$listing"); then
    echo "$label: could not read the entries of $appimage from unsquashfs's listing" >&2
    return 1
  fi
  if [ -z "$entries" ]; then
    echo "$label: unsquashfs listed no entries in $appimage" >&2
    return 1
  fi
  if ! bad=$(awk '
    substr($1, 1, 1) == "l" { next }
    substr($1, 8, 1) != "r" || (substr($1, 4, 1) ~ /[xs]/ && substr($1, 10, 1) !~ /[xt]/) { print "  " $0 }' <<<"$entries"); then
    echo "$label: could not check the permissions of the entries of $appimage" >&2
    return 1
  fi
  if [ -n "$bad" ]; then
    echo "$label: $appimage carries entries a user other than their owner cannot read or run — refusing to ship:" >&2
    printf '%s\n' "$bad" >&2
    return 1
  fi
  echo "$label: $appimage: $(wc -l <<<"$entries") entries, all readable by every user, the executable ones runnable by every user"
}
