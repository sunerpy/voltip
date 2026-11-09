#!/usr/bin/env bash
# Download one file pinned by SHA-256, retrying for a bounded time, and put it at DEST only once its
# digest matches. CI's downloads from GitHub's releases go through it (install-cargo-tauri.sh,
# install-cargo-ndk.sh, fetch-sherpa-onnx-archives.sh, prepare-gradle.sh, the Windows upgrade
# probe): github.com and its asset host answered 502 and 504 for minutes at a time on 2026-11-08
# (CI runs 42469017816, 42471046202, 42478584338), and six attempts five seconds apart ran out
# within half a minute.
#
# Usage: .github/scripts/fetch-pinned.sh URL SHA256 DEST
#   A DEST that already holds the pinned bytes is kept and nothing is downloaded. Otherwise curl
#   writes DEST.part.<pid> beside DEST and only bytes with the pinned digest are moved to DEST; a
#   mismatch is never retried away (exit 1, DEST left as it was). Exit 2: bad arguments, or a curl
#   older than 7.71 (no --retry-all-errors).
# Environment (defaults in brackets):
#   FETCH_PINNED_RETRIES [60]          curl --retry: attempts after the first
#   FETCH_PINNED_RETRY_DELAY [10]      curl --retry-delay: seconds between attempts
#   FETCH_PINNED_RETRY_MAX_TIME [600]  curl --retry-max-time: no new attempt this many seconds on
#   FETCH_PINNED_MAX_TIME [900]        curl --max-time: one attempt's limit
# Every failed attempt is retried (--retry-all-errors): an HTTP error, a refused or reset
# connection, a transfer cut short, or one slower than 1 KB/s for a minute. `--retry` alone skips
# some of them: the Apple silicon runner's curl reported a 504 as error 56 and gave up at once
# (run 42471046202).
set -euo pipefail

[[ $# -eq 3 ]] || { echo "usage: $0 URL SHA256 DEST" >&2; exit 2; }
url=$1 want=$2 dest=$3
[[ "$want" =~ ^[0-9a-f]{64}$ ]] || { echo "fetch-pinned: '$want' is not a lowercase SHA-256" >&2; exit 2; }

# macOS has shasum (and, from 15 on, a BSD sha256sum without GNU's flags); Linux has both.
sha256() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{print $1}'
	else
		sha256sum "$1" | awk '{print $1}'
	fi
}

if [[ -f "$dest" && "$(sha256 "$dest")" == "$want" ]]; then
	echo "fetch-pinned: $dest already holds sha256 $want"
	exit 0
fi

version=$(curl --version | awk 'NR == 1 {print $2}')
IFS=. read -r major minor _ <<<"$version"
if ! [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ ]] || ((major < 7 || (major == 7 && minor < 71))); then
	echo "fetch-pinned: curl $version has no --retry-all-errors (7.71 or later)" >&2
	exit 2
fi

mkdir -p "$(dirname "$dest")"
part="$dest.part.$$"
trap 'rm -f "$part"' EXIT
echo "fetch-pinned: $url (curl $version)"
# --disable first: no ~/.curlrc changes what is fetched. curl truncates the output file before each
# new attempt, so a retried transfer never appends to a partial one.
if ! curl --disable --fail --silent --show-error --location \
	--connect-timeout 30 --speed-limit 1024 --speed-time 60 --max-time "${FETCH_PINNED_MAX_TIME:-900}" \
	--retry "${FETCH_PINNED_RETRIES:-60}" --retry-delay "${FETCH_PINNED_RETRY_DELAY:-10}" \
	--retry-max-time "${FETCH_PINNED_RETRY_MAX_TIME:-600}" --retry-all-errors \
	--output "$part" "$url"; then
	echo "fetch-pinned: $url still failed after the retries above" >&2
	exit 1
fi
have=$(sha256 "$part")
if [[ "$have" != "$want" ]]; then
	echo "fetch-pinned: $url has SHA-256 $have ($(wc -c <"$part" | tr -d ' ') bytes), pinned $want" >&2
	exit 1
fi
mv -f "$part" "$dest"
trap - EXIT
echo "fetch-pinned: $dest ($(wc -c <"$dest" | tr -d ' ') bytes, sha256 $have)"
