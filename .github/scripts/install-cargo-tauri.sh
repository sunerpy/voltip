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
# (recorded 2026-09-25) and the Apple silicon zip the macOS legs use (recorded 2026-09-28).
case "$(uname -s)-$(uname -m)" in
Linux-x86_64)
	asset="cargo-tauri-x86_64-unknown-linux-gnu.tgz"
	TAURI_CLI_SHA256="e75e2a1e8d3bceba327a10640a08b0bfef6d7559e8e13274629189001f2fce08"
	;;
Darwin-arm64)
	asset="cargo-tauri-aarch64-apple-darwin.zip"
	TAURI_CLI_SHA256="7734f1d942dbe6e5fea91c1575452f4bf2cc942e6902f2d9d78513baa8527b24"
	;;
*)
	echo "install-cargo-tauri: only x86_64 Linux and Apple silicon macOS are pinned (got $(uname -s) $(uname -m))" >&2
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
curl --fail --silent --show-error --location --retry 3 --output "$tmp/$asset" "$url"
# macOS has shasum, not sha256sum.
if command -v sha256sum >/dev/null 2>&1; then
	echo "${TAURI_CLI_SHA256}  $tmp/$asset" | sha256sum --check --strict --quiet
else
	echo "${TAURI_CLI_SHA256}  $tmp/$asset" | shasum -a 256 --check --strict --quiet
fi
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
