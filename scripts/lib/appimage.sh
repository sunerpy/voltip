# shellcheck shell=bash
# The AppRun the AppImage starts through, and a check that every user can read and run what the
# AppImage carries. Sourced by scripts/build-linux-x64.sh and the Linux leg of
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

VOLTIP_APPRUN_URL=https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64
VOLTIP_APPRUN_SHA256=f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f

# Print the bundler's tools directory: dirs::cache_dir() joined with tauri, that is
# $XDG_CACHE_HOME/tauri when XDG_CACHE_HOME is an absolute path, else $HOME/.cache/tauri.
voltip_tauri_tools_dir() {
  case ${XDG_CACHE_HOME:-} in
    /*) printf '%s/tauri\n' "$XDG_CACHE_HOME" ;;
    *)
      [ -n "${HOME:-}" ] || { echo "appimage: HOME is not set, so the bundler's tools directory is unknown" >&2; return 1; }
      printf '%s/.cache/tauri\n' "$HOME"
      ;;
  esac
}

# Leave <file> holding <sha256>'s bytes with mode 0755, downloading it from <url> when it is
# missing. A file already there with other contents fails and stays as it is: the bundler packages
# whatever its tools directory holds into the AppImage every user runs, so a stranger there is for
# someone to look at, not to replace quietly.
voltip_place_pinned_executable() { # <url> <sha256> <file>
  local url=$1 sha256=$2 file=$3 actual part
  if [ -e "$file" ] || [ -L "$file" ]; then
    if [ -L "$file" ] || [ ! -f "$file" ]; then
      echo "appimage: $file is not a regular file — refusing to package it" >&2
      return 1
    fi
    actual=$(sha256sum <"$file") || return 1
    if [ "${actual%% *}" != "$sha256" ]; then
      echo "appimage: $file has SHA-256 ${actual%% *}, not the pinned $sha256 — refusing to package it; look at it, delete it and build again to download the pinned one" >&2
      return 1
    fi
  else
    mkdir -p "$(dirname "$file")" || return 1
    # Beside the file, so that the rename is atomic; two builds at once each verify their own.
    part=$(mktemp "$file.XXXXXX") || return 1
    echo "appimage: downloading $url" >&2
    if ! curl -sSfL --retry 3 -o "$part" "$url"; then
      rm -f "$part"
      echo "appimage: download failed: $url" >&2
      return 1
    fi
    actual=$(sha256sum <"$part") || { rm -f "$part"; return 1; }
    if [ "${actual%% *}" != "$sha256" ]; then
      rm -f "$part"
      echo "appimage: $url has SHA-256 ${actual%% *}, not the pinned $sha256" >&2
      return 1
    fi
    if ! chmod 0755 "$part" || ! mv -f "$part" "$file"; then
      rm -f "$part"
      return 1
    fi
  fi
  chmod 0755 "$file"
}

# Place the pinned AppRun-x86_64, mode 0755, in the bundler's tools directory. Call it before
# `cargo tauri build` or `cargo tauri bundle` builds an AppImage.
voltip_appimage_apprun() {
  local dir
  dir=$(voltip_tauri_tools_dir) || return 1
  voltip_place_pinned_executable "$VOLTIP_APPRUN_URL" "$VOLTIP_APPRUN_SHA256" "$dir/AppRun-x86_64" || return 1
  echo "appimage: $dir/AppRun-x86_64 is the pinned AppRun, mode 0755"
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
