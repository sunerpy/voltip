#!/usr/bin/env bash
# Put the prebuilt sherpa-onnx runtime that sherpa-onnx-sys's build script would download into DIR
# and point SHERPA_ONNX_ARCHIVE_DIR at it: the build script then copies the archive from there
# (sherpa-onnx-sys 1.13.8 build.rs). Its own download is one GET from GitHub's releases with no
# retry, and every CI build runs it again (forget-sherpa-onnx-build.sh); a 502, 503 or 504 there
# failed verify-rust, windows-cross and smoke-desktop on 2026-11-08 (runs 42471046202 and
# 42478584338: "Failed to download sherpa-onnx archive … status code 504"). Here each archive comes
# through fetch-pinned.sh: retried for a bounded time and kept only with the SHA-256 pinned below.
#
# Usage: .github/scripts/fetch-sherpa-onnx-archives.sh DIR ARCHIVE...
#   ARCHIVE: host (this machine's own: Linux x86_64, macOS arm64 or x86_64), linux-x64, win-x64,
#   osx-arm64, osx-x64 — the shared-library archives (sherpa-onnx's `shared` feature, the
#   workspace Cargo.toml). Every archive a job's builds need must be named: with the variable set,
#   a build script that finds no archive for its target fails. With GITHUB_ENV set,
#   SHERPA_ONNX_ARCHIVE_DIR=DIR goes into it for the job's later steps.
# The version comes from Cargo.lock and must be the pinned one: a sherpa-onnx-sys update fails here
# until the new archives' digests are recorded.
set -euo pipefail

SHERPA_ONNX_VERSION="1.13.8"
# SHA-256 of the v1.13.8 release assets at https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.8,
# as the release lists them (recorded 2026-11-09; the Windows one also matches a download of
# 2026-11-08).
pin() {
	case "$1" in
	linux-x64) echo "sherpa-onnx-v${SHERPA_ONNX_VERSION}-linux-x64-shared-lib.tar.bz2 3892d184be41027e18165e67f549cd4e4cdd8dcd73ac5579e97afd55e14e30b6" ;;
	win-x64) echo "sherpa-onnx-v${SHERPA_ONNX_VERSION}-win-x64-shared-MT-Release-lib.tar.bz2 b8eedf41bd6d3779218887b48367bb7a3ece5aaa7667f01f69ee823a12b0a9e7" ;;
	osx-arm64) echo "sherpa-onnx-v${SHERPA_ONNX_VERSION}-osx-arm64-shared-lib.tar.bz2 ae77050cdae565496059d96f5ab33d77b397a0864282a4e4a26e3b3b3effb948" ;;
	osx-x64) echo "sherpa-onnx-v${SHERPA_ONNX_VERSION}-osx-x64-shared-lib.tar.bz2 0f88371565a06372889c76266e253abd1b415fcc6f98232cabd968933efb0711" ;;
	*) return 1 ;;
	esac
}

[[ $# -ge 2 ]] || { echo "usage: $0 DIR ARCHIVE... (host, linux-x64, win-x64, osx-arm64, osx-x64)" >&2; exit 2; }
dir=$1
shift
here=$(cd "$(dirname "$0")" && pwd)
lock="$here/../../Cargo.lock"
locked=$(awk '/^name = "sherpa-onnx-sys"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$lock")
if [[ "$locked" != "$SHERPA_ONNX_VERSION" ]]; then
	echo "fetch-sherpa-onnx-archives: Cargo.lock has sherpa-onnx-sys '$locked', this script pins $SHERPA_ONNX_VERSION: record the new release's archive digests here" >&2
	exit 1
fi

for archive in "$@"; do
	if [[ "$archive" == host ]]; then
		case "$(uname -s)-$(uname -m)" in
		Linux-x86_64) archive=linux-x64 ;;
		Darwin-arm64) archive=osx-arm64 ;;
		Darwin-x86_64) archive=osx-x64 ;;
		*) echo "fetch-sherpa-onnx-archives: no archive pinned for this machine ($(uname -s) $(uname -m))" >&2; exit 2 ;;
		esac
	fi
	read -r name sha256 < <(pin "$archive") || { echo "fetch-sherpa-onnx-archives: unknown archive '$archive'" >&2; exit 2; }
	"$here/fetch-pinned.sh" "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${SHERPA_ONNX_VERSION}/$name" "$sha256" "$dir/$name"
done

if [[ -n "${GITHUB_ENV:-}" ]]; then
	echo "SHERPA_ONNX_ARCHIVE_DIR=$dir" >>"$GITHUB_ENV"
fi
echo "fetch-sherpa-onnx-archives: SHERPA_ONNX_ARCHIVE_DIR=$dir"
