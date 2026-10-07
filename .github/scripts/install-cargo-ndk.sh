#!/usr/bin/env bash
# Install cargo-ndk (`cargo ndk`), which scripts/build-android-rn.sh builds the React Native phone
# app's Rust shell with (CI's `android-rn` job, the release candidate's `bundle-android` leg), from
# the official bbqsrc/cargo-ndk release tarball, pinned by version and SHA-256. The tarball holds
# the subcommand and the three helpers it runs (cargo-ndk-env, -runner, -test); all four go to
# DEST_DIR, as `cargo install cargo-ndk` puts them.
#
# Usage: .github/scripts/install-cargo-ndk.sh [DEST_DIR]   (default: $HOME/.cargo/bin)
# Environment: none required. Exit 0 only when `cargo ndk --version` prints the pinned version.
set -euo pipefail

CARGO_NDK_VERSION="4.1.2"
# SHA-256 of cargo-ndk-x86_64-unknown-linux-gnu-v4.1.2.tgz at
# https://github.com/bbqsrc/cargo-ndk/releases/tag/v4.1.2 (recorded 2026-10-08; the release API's
# digest and a download agree).
CARGO_NDK_SHA256="9451622c4567e8abb2c8005001855e32901c0b716ccdd3a173a4375fd03426e1"
case "$(uname -s)-$(uname -m)" in
Linux-x86_64) triple=x86_64-unknown-linux-gnu ;;
*)
	echo "install-cargo-ndk: only x86_64 Linux is pinned (got $(uname -s) $(uname -m))" >&2
	exit 2
	;;
esac
asset="cargo-ndk-${triple}-v${CARGO_NDK_VERSION}.tgz"
dest=${1:-"$HOME/.cargo/bin"}

if [[ "$(cargo ndk --version 2>/dev/null)" == "cargo-ndk ${CARGO_NDK_VERSION}" ]]; then
	echo "install-cargo-ndk: cargo-ndk ${CARGO_NDK_VERSION} already installed"
	exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/bbqsrc/cargo-ndk/releases/download/v${CARGO_NDK_VERSION}/${asset}"
# GitHub's release downloads answer 500 now and then for several seconds in a row: retry for about
# half a minute (install-cargo-tauri.sh).
curl --fail --silent --show-error --location --retry 6 --retry-delay 5 --output "$tmp/$asset" "$url"
actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
[[ "$actual" == "$CARGO_NDK_SHA256" ]] || { echo "install-cargo-ndk: $asset has SHA-256 $actual, expected $CARGO_NDK_SHA256" >&2; exit 1; }
tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$dest"
for bin in cargo-ndk cargo-ndk-env cargo-ndk-runner cargo-ndk-test; do
	install -m 0755 "$tmp/cargo-ndk-${triple}-v${CARGO_NDK_VERSION}/$bin" "$dest/$bin"
done
case ":$PATH:" in
*":$dest:"*) ;;
*)
	if [[ -n "${GITHUB_PATH:-}" ]]; then echo "$dest" >>"$GITHUB_PATH"; fi
	export PATH="$dest:$PATH"
	;;
esac
installed=$(cargo ndk --version)
[[ "$installed" == "cargo-ndk ${CARGO_NDK_VERSION}" ]] || { echo "install-cargo-ndk: unexpected version '$installed'" >&2; exit 1; }
echo "install-cargo-ndk: $installed -> $dest/cargo-ndk"
