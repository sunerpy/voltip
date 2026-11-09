#!/usr/bin/env bash
# Install the Rust Tauri CLI (`cargo tauri`) that scripts/build-windows-x64.sh and the release
# bundle legs call, from the official tauri-apps/tauri release tarball, pinned by version and
# SHA-256. The npm `@tauri-apps/cli` in pnpm-lock.yaml is the same version; keep the two aligned
# when bumping (apps/desktop/package.json devDependencies."@tauri-apps/cli").
#
# Usage: .github/scripts/install-cargo-tauri.sh [DEST_DIR]   (default: $HOME/.cargo/bin)
# Environment: none required. Exit 0 only when `cargo-tauri --version` prints the pinned version.
set -euo pipefail

TAURI_CLI_VERSION="2.12.4"
# SHA-256 of the release assets at
# https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.4: the x86_64 Linux tarball and
# the Apple silicon and Intel zips the macOS legs use (recorded 2026-11-06; the Linux tarball's
# digest checked on a download, the zips' against the digests GitHub lists for the assets).
case "$(uname -s)-$(uname -m)" in
Linux-x86_64)
	asset="cargo-tauri-x86_64-unknown-linux-gnu.tgz"
	TAURI_CLI_SHA256="820ae3c7580ca637cad15f65638960ee2a96e5a491e224691c86d6f25e6c9e8a"
	;;
Darwin-arm64)
	asset="cargo-tauri-aarch64-apple-darwin.zip"
	TAURI_CLI_SHA256="f71a34ada728ca2d1a51c28d6d21639e1e080898ec69ec8d49c5da3b7e8bc1be"
	;;
Darwin-x86_64)
	asset="cargo-tauri-x86_64-apple-darwin.zip"
	TAURI_CLI_SHA256="164d184c0e50800dcae7bb31f0815066ebd27efc3e3bb73117974bcd2e982192"
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
# GitHub's release downloads answer 500, 502 and 504 for seconds (CI 2026-09-29, the macOS x64 leg:
# four attempts within 8 s) or minutes (2026-11-08, run 42471046202: both macOS legs failed here,
# the Intel one after six retries five seconds apart, the Apple silicon one at once: its curl
# reported the 504 as error 56, which `--retry` alone does not retry). fetch-pinned.sh retries
# every error for up to ten minutes and keeps the zip only when its SHA-256 is the pinned one.
"$(dirname "$0")/fetch-pinned.sh" "$url" "$TAURI_CLI_SHA256" "$tmp/$asset"
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
