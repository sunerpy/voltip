#!/bin/sh
# One-line install of Voltip on Linux (x86_64) and macOS (Apple silicon or Intel) from a GitHub
# release (Windows: scripts/install.ps1):
#
#   curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
#
# It picks the package for this computer (the .deb where apt is, else the AppImage; on a Mac the
# dmg for its processor, also from a Rosetta shell), downloads it and SHA256SUMS from the same
# release, and installs nothing unless the package's SHA-256 matches its line there.
#
#   VOLTIP_VERSION=0.0.4       a given release instead of the latest
#   VOLTIP_PACKAGE=appimage    Linux: the AppImage even where apt is (or deb to insist on it)
#   VOLTIP_INSTALL_DIR=DIR     the AppImage's directory (default ~/.local/bin), or the Mac app's
#                              (default /Applications, ~/Applications when that is not writable),
#                              or the server's (default ~/.local/share/voltip-server)
#
# voltip-server, the local speech service for a Linux server without a desktop (docs/dictation.md
# §23.5), installs without root into ~/.local/share/voltip-server/<version>, linked as
# ~/.local/bin/voltip-server:
#
#   curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh -s -- --server
#
#   --server        (VOLTIP_PACKAGE=server) voltip-server instead of the app
#   --systemd-user  (VOLTIP_SYSTEMD=user) also write ~/.config/systemd/user/voltip-server.service
#   --enable        (VOLTIP_SYSTEMD_ENABLE=1) with --systemd-user: enable and start it now
#   VOLTIP_BIN_DIR=DIR         where the server's command is linked (default ~/.local/bin)
set -eu

REPO="sunerpy/voltip"
CHECKSUM_FILE="SHA256SUMS"

err() {
	printf 'voltip-install: %s\n' "$1" >&2
	exit 1
}

info() {
	printf 'voltip-install: %s\n' "$1" >&2
}

for arg in "$@"; do
	case "$arg" in
	--server) VOLTIP_PACKAGE=server ;;
	--systemd-user) VOLTIP_SYSTEMD=user ;;
	--enable) VOLTIP_SYSTEMD_ENABLE=1 ;;
	*) err "unknown option: $arg (options: --server, --systemd-user, --enable)" ;;
	esac
done

if command -v curl >/dev/null 2>&1; then
	download() { curl -fsSL --retry 3 "$1" -o "$2"; }
	fetch() { curl -fsSL --retry 3 "$1"; }
elif command -v wget >/dev/null 2>&1; then
	download() { wget -qO "$2" "$1"; }
	fetch() { wget -qO - "$1"; }
else
	err "curl or wget is required"
fi

# The package for this computer.
case "$(uname -s)" in
Linux)
	case "$(uname -m)" in
	x86_64 | amd64) ;;
	*) err "Voltip ships for x86_64 Linux only (this is $(uname -m))" ;;
	esac
	package=${VOLTIP_PACKAGE:-}
	if [ -z "$package" ]; then
		if command -v apt-get >/dev/null 2>&1; then package=deb; else package=appimage; fi
	fi
	case "$package" in
	deb | appimage | server) ;;
	*) err "VOLTIP_PACKAGE must be deb, appimage or server, not '$package'" ;;
	esac
	;;
Darwin)
	if [ "${VOLTIP_PACKAGE:-}" = server ]; then err "voltip-server ships for x86_64 Linux only; on a Mac, switch on the local service in Voltip's settings"; fi
	# A shell under Rosetta reports x86_64 on Apple silicon; the native build is the one to take.
	if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
		mac_arch=aarch64
	else
		case "$(uname -m)" in
		x86_64) mac_arch=x64 ;;
		*) err "unsupported Mac processor: $(uname -m)" ;;
		esac
	fi
	package=dmg
	;;
*) err "unsupported system: $(uname -s) (on Windows, use scripts/install.ps1)" ;;
esac

if [ -n "${VOLTIP_VERSION:-}" ]; then
	version=$(printf '%s' "$VOLTIP_VERSION" | sed 's/^v//')
else
	info "finding the latest release"
	tag=$(fetch "https://api.github.com/repos/${REPO}/releases/latest" |
		sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
	[ -n "$tag" ] || err "could not find the latest release"
	version=$(printf '%s' "$tag" | sed 's/^v//')
fi
# The version goes into URLs and file names: digits and dots, and an optional pre-release tail.
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' ||
	err "not a release version: '$version'"

case "$package" in
deb) asset="Voltip_${version}_amd64.deb" ;;
appimage) asset="Voltip_${version}_amd64.AppImage" ;;
server) asset="voltip-server-${version}-linux-x64.tar.gz" ;;
dmg) asset="Voltip_${version}_${mac_arch}.dmg" ;;
esac
base_url="https://github.com/${REPO}/releases/download/v${version}"

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t voltip)
cleanup() {
	if [ -n "${mounted:-}" ]; then hdiutil detach "$mounted" -quiet >/dev/null 2>&1 || true; fi
	rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

info "downloading ${asset} (Voltip ${version})"
download "${base_url}/${CHECKSUM_FILE}" "$tmp/$CHECKSUM_FILE" ||
	err "release v${version} has no ${CHECKSUM_FILE} (is ${version} a Voltip release?)"
expected=$(awk -v name="$asset" '{
	file = $2
	sub(/^\*/, "", file)
	if (file == name) { print $1; exit }
}' "$tmp/$CHECKSUM_FILE")
[ -n "$expected" ] || err "release v${version} has no ${asset}"
download "${base_url}/${asset}" "$tmp/$asset" || err "could not download ${asset}"

if command -v sha256sum >/dev/null 2>&1; then
	actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
	actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
else
	err "sha256sum or shasum is required"
fi
[ "$actual" = "$expected" ] || err "checksum mismatch for ${asset}: nothing was installed"
info "SHA-256 matches ${CHECKSUM_FILE}"

as_root() {
	if [ "$(id -u)" = 0 ]; then
		"$@"
	elif command -v sudo >/dev/null 2>&1; then
		sudo "$@"
	else
		err "installing the .deb needs root; run as root, or set VOLTIP_PACKAGE=appimage"
	fi
}

case "$package" in
deb)
	# apt reads the file as its own user: let it, instead of warning about the private temp dir.
	chmod 755 "$tmp"
	chmod 644 "$tmp/$asset"
	info "installing with apt (it asks for your password when it needs to)"
	# The dependencies come from the distribution's mirror. With package lists older than the
	# mirror (a computer that has not updated for a while, a fresh cloud image) apt asks for
	# versions the mirror no longer has and fails with 404, so refresh the lists first; if that
	# fails, the install still tries with the lists as they are.
	as_root apt-get update -qq || info "apt-get update failed; installing with the package lists as they are"
	as_root apt-get install -y "$tmp/$asset"
	info "installed Voltip ${version}; start it from the applications menu or run voltip-desktop"
	;;
appimage)
	dir=${VOLTIP_INSTALL_DIR:-$HOME/.local/bin}
	mkdir -p "$dir"
	install -m 0755 "$tmp/$asset" "$dir/Voltip.AppImage"
	data="${XDG_DATA_HOME:-$HOME/.local/share}"
	mkdir -p "$data/applications"
	# The menu icon, from inside the AppImage (its runtime extracts without FUSE); the entry goes
	# without one when that does not work.
	icon=
	icon_path="usr/share/icons/hicolor/128x128/apps/voltip-desktop.png"
	if (cd "$tmp" && "$dir/Voltip.AppImage" --appimage-extract "$icon_path" >/dev/null 2>&1) &&
		[ -s "$tmp/squashfs-root/$icon_path" ]; then
		mkdir -p "$data/icons/hicolor/128x128/apps"
		cp "$tmp/squashfs-root/$icon_path" "$data/icons/hicolor/128x128/apps/voltip.png"
		icon="Icon=voltip"
	fi
	cat >"$data/applications/voltip.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Voltip
Comment=Hold a hotkey, speak, let go: the text lands at your cursor
Exec="$dir/Voltip.AppImage"
$icon
Terminal=false
Categories=Utility;
EOF
	info "installed Voltip ${version} to $dir/Voltip.AppImage (and the applications menu)"
	info "an AppImage needs FUSE 2 (libfuse2) to start"
	;;
server)
	root=${VOLTIP_INSTALL_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/voltip-server}
	bindir=${VOLTIP_BIN_DIR:-$HOME/.local/bin}
	unpacked="voltip-server-${version}-linux-x64"
	tar -xzf "$tmp/$asset" -C "$tmp" || err "could not unpack ${asset}"
	[ -x "$tmp/$unpacked/bin/voltip-server" ] || err "${asset} holds no bin/voltip-server"
	mkdir -p "$root" "$bindir"
	rm -rf "${root:?}/$version"
	mv "$tmp/$unpacked" "$root/$version"
	ln -sfn "$root/$version/bin/voltip-server" "$bindir/voltip-server"
	# Start it once. What it takes from the system (VOLTIP_SERVER_SONAMES; whether BLAS is among
	# them depends on the build machine) is named when missing, never installed here; the package's
	# own lib/ is found through the binary's RUNPATH.
	if ! started=$("$root/$version/bin/voltip-server" --version 2>&1); then
		info "voltip-server does not start on this system: $(printf '%s\n' "$started" | head -1)"
		missing=$(ldd "$root/$version/bin/voltip-server" 2>/dev/null | awk '$2 == "=>" && $3 == "not" { printf " %s", $1 }' || true)
		if [ -n "$missing" ]; then
			info "these system libraries are missing:$missing"
			apt='' dnf=''
			for lib in $missing; do
				case "$lib" in
				libblas.so.3) apt="$apt libblas3" dnf="$dnf blas" ;;
				libstdc++.so.6) apt="$apt libstdc++6" dnf="$dnf libstdc++" ;;
				libgcc_s.so.1) apt="$apt libgcc-s1" dnf="$dnf libgcc" ;;
				esac
			done
			if [ -n "$apt" ] && command -v apt-get >/dev/null 2>&1; then
				info "install them with: sudo apt-get install$apt"
			elif [ -n "$dnf" ] && command -v dnf >/dev/null 2>&1; then
				info "install them with: sudo dnf install$dnf"
			fi
		fi
	fi
	if [ "${VOLTIP_SYSTEMD:-}" = user ]; then
		unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
		mkdir -p "$unit_dir"
		cat >"$unit_dir/voltip-server.service" <<UNIT
[Unit]
Description=Voltip local speech service

[Service]
ExecStart=$bindir/voltip-server
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
		info "wrote $unit_dir/voltip-server.service (its options: voltip-server --help)"
		if command -v systemctl >/dev/null 2>&1; then
			systemctl --user daemon-reload || info "systemctl --user daemon-reload failed (no user session?)"
			if [ "${VOLTIP_SYSTEMD_ENABLE:-}" = 1 ]; then
				systemctl --user enable --now voltip-server || err "could not start the voltip-server user service"
				info "voltip-server is running as a systemd user service"
			else
				info "start it with: systemctl --user enable --now voltip-server"
			fi
		fi
		info "to keep it running after you log out: loginctl enable-linger $(id -un)"
	fi
	info "installed voltip-server ${version} to $root/$version, command $bindir/voltip-server"
	info "check it with: voltip-server --check (the token: voltip-server --print-token)"
	;;
dmg)
	dir=${VOLTIP_INSTALL_DIR:-/Applications}
	if [ -z "${VOLTIP_INSTALL_DIR:-}" ] && [ ! -w "$dir" ]; then dir="$HOME/Applications"; fi
	mkdir -p "$dir"
	if pgrep -xq voltip-desktop 2>/dev/null; then err "Voltip is running: quit it (tray menu, Quit) and run this again"; fi
	mounted="$tmp/mnt"
	mkdir -p "$mounted"
	hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mounted" "$tmp/$asset" ||
		{ mounted=; err "could not open ${asset}"; }
	[ -d "$mounted/Voltip.app" ] || err "${asset} holds no Voltip.app"
	rm -rf "$dir/Voltip.app"
	ditto "$mounted/Voltip.app" "$dir/Voltip.app"
	hdiutil detach "$mounted" -quiet >/dev/null 2>&1 || true
	mounted=
	# Downloaded here rather than by a browser, so the app carries no quarantine flag and opens
	# without the Gatekeeper prompt; clear one anyway in case an older copy left it.
	xattr -dr com.apple.quarantine "$dir/Voltip.app" 2>/dev/null || true
	# Launchpad and Spotlight list the apps LaunchServices knows. Finder registers what it copies;
	# a copy made here would only be registered at its first launch, so register it now (and hand
	# it to Spotlight, a no-op where indexing is off).
	lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
	if command -v lsregister >/dev/null 2>&1; then lsregister=$(command -v lsregister); fi
	if [ -x "$lsregister" ]; then
		"$lsregister" -f "$dir/Voltip.app" || info "LaunchServices did not take the app; it appears in Launchpad after its first start"
	fi
	mdimport "$dir/Voltip.app" 2>/dev/null || true
	info "installed Voltip ${version} to $dir/Voltip.app; open it from Launchpad or with: open -a Voltip"
	;;
esac
