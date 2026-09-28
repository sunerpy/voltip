#!/usr/bin/env bash
# `cargo tauri bundle` on a macOS runner, tried up to three times. hdiutil on GitHub's macOS images
# now and then fails to create, attach or detach the dmg ("Resource busy"), which aborts
# bundle_dmg.sh: the 0.0.4 release's x86_64 leg failed there while the same commit's CI job built
# the same dmg. A failed attempt's Voltip images are detached and its partial dmg removed before
# the next one; the .app and the compiled binary are reused, so a retry costs about a minute.
#
# Usage (from apps/desktop): .github/scripts/tauri-bundle-retry.sh <bundle-dir> -- <bundle args…>
#   <bundle-dir>  ../../target/<triple>/release/bundle
set -euo pipefail

bundle_dir=${1:?bundle directory}
shift
[ "${1:-}" = -- ] && shift

for attempt in 1 2 3; do
	if cargo tauri bundle --verbose "$@"; then
		exit 0
	fi
	[ "$attempt" -lt 3 ] || break
	echo "::warning::cargo tauri bundle failed (attempt $attempt of 3); detaching leftover images and trying again"
	# Only this product's volumes and bundle_dmg.sh's own mounts, never the runner's.
	hdiutil info | awk '$NF ~ /^\/Volumes\/(Voltip|dmg\.)/ { print $1 }' | while IFS= read -r device; do
		hdiutil detach -force "$device" || true
	done
	rm -f "$bundle_dir"/dmg/*.dmg "$bundle_dir"/dmg/rw.*.dmg
	# Back-off before the retry: give diskimages-helper a moment to let go of the device.
	sleep 5
done
echo "::error::cargo tauri bundle failed three times" >&2
exit 1
