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
# sha256sum of cargo-tauri-x86_64-unknown-linux-gnu.tgz from
# https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.11.5 (recorded 2026-09-25).
TAURI_CLI_SHA256="e75e2a1e8d3bceba327a10640a08b0bfef6d7559e8e13274629189001f2fce08"

dest=${1:-"$HOME/.cargo/bin"}
arch=$(uname -m)
[[ "$arch" == x86_64 ]] || { echo "install-cargo-tauri: only x86_64 Linux is pinned (got $arch)" >&2; exit 2; }

if command -v cargo-tauri >/dev/null 2>&1 && [[ "$(cargo-tauri --version 2>/dev/null)" == "tauri-cli ${TAURI_CLI_VERSION}" ]]; then
	echo "install-cargo-tauri: tauri-cli ${TAURI_CLI_VERSION} already installed"
	exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
asset="cargo-tauri-x86_64-unknown-linux-gnu.tgz"
url="https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v${TAURI_CLI_VERSION}/${asset}"
curl --fail --silent --show-error --location --retry 3 --output "$tmp/$asset" "$url"
echo "${TAURI_CLI_SHA256}  $tmp/$asset" | sha256sum --check --strict --quiet
tar -xzf "$tmp/$asset" -C "$tmp" cargo-tauri
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
