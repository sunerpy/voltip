#!/usr/bin/env bash
# Install the Rust Tauri CLI (`cargo tauri`) that scripts/build-windows-x64.sh and the release
# bundle legs call, from the official tauri-apps/tauri release tarball, pinned by version and
# SHA-256. The npm `@tauri-apps/cli` in pnpm-lock.yaml is the same version; keep the two aligned
# when bumping (apps/desktop/package.json devDependencies."@tauri-apps/cli").
#
# Usage: .github/scripts/install-cargo-tauri.sh [DEST_DIR]   (default: $HOME/.cargo/bin)
# Environment: none required. Exit 0 only when `cargo-tauri --version` prints the pinned version.
set -euo pipefail

TAURI_CLI_VERSION="2.11.5"
# SHA-256 of the release assets at
# https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.11.5: the x86_64 Linux tarball
# (recorded 2026-09-25) and the Apple silicon and Intel zips the macOS legs use (recorded
# 2026-09-28).
case "$(uname -s)-$(uname -m)" in
Linux-x86_64)
	asset="cargo-tauri-x86_64-unknown-linux-gnu.tgz"
	TAURI_CLI_SHA256="e75e2a1e8d3bceba327a10640a08b0bfef6d7559e8e13274629189001f2fce08"
	;;
Darwin-arm64)
	asset="cargo-tauri-aarch64-apple-darwin.zip"
	TAURI_CLI_SHA256="7734f1d942dbe6e5fea91c1575452f4bf2cc942e6902f2d9d78513baa8527b24"
	;;
Darwin-x86_64)
	asset="cargo-tauri-x86_64-apple-darwin.zip"
	TAURI_CLI_SHA256="535b790c9d1995f67fdb21296a2abbf8026c7846b206cff9b93984eb740eab8b"
	;;
*)
	echo "install-cargo-tauri: only x86_64 Linux and macOS (Apple silicon, Intel) are pinned (got $(uname -s) $(uname -m))" >&2
	exit 2
	;;
esac

dest=${1:-"$HOME/.cargo/bin"}

if command -v cargo-tauri >/dev/null 2>&1 && [[ "$(cargo-tauri --version 2>/dev/null)" == "tauri-cli ${TAURI_CLI_VERSION}" ]]; then
	echo "install-cargo-tauri: tauri-cli ${TAURI_CLI_VERSION} already installed"
	exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v${TAURI_CLI_VERSION}/${asset}"
# GitHub's release downloads answer 500 now and then for several seconds in a row (CI 2026-09-29,
# the macOS x64 leg: four attempts within 8 s all failed): retry for about half a minute.
curl --fail --silent --show-error --location --retry 6 --retry-delay 5 --output "$tmp/$asset" "$url"
# The digest is compared here rather than with `--check`: macOS 15's BSD sha256sum takes none of
# GNU's flags, and older macOS has only shasum.
if command -v shasum >/dev/null 2>&1; then
	actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
else
	actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
fi
[[ "$actual" == "$TAURI_CLI_SHA256" ]] || { echo "install-cargo-tauri: $asset has SHA-256 $actual, expected $TAURI_CLI_SHA256" >&2; exit 1; }
case "$asset" in
*.zip) unzip -q -o "$tmp/$asset" cargo-tauri -d "$tmp" ;;
*) tar -xzf "$tmp/$asset" -C "$tmp" cargo-tauri ;;
esac
mkdir -p "$dest"
install -m 0755 "$tmp/cargo-tauri" "$dest/cargo-tauri"
case ":$PATH:" in
*":$dest:"*) ;;
*)
	if [[ -n "${GITHUB_PATH:-}" ]]; then echo "$dest" >>"$GITHUB_PATH"; fi
	export PATH="$dest:$PATH"
	;;
esac
installed=$("$dest/cargo-tauri" --version)
[[ "$installed" == "tauri-cli ${TAURI_CLI_VERSION}" ]] || { echo "install-cargo-tauri: unexpected version '$installed'" >&2; exit 1; }
echo "install-cargo-tauri: $installed -> $dest/cargo-tauri"
