#!/usr/bin/env bash
# Check a macOS build the way a Mac that downloads it meets it (release.yml bundle-macos and
# macos.yml): a sealed ad-hoc signature, so Gatekeeper offers "Open Anyway" instead of calling the
# app damaged; nothing built for a macOS newer than the documented 11; the version the build
# names; and a dmg that mounts with the app and the Applications link.
#
# Usage: .github/scripts/check-macos-bundle.sh <bundle-dir> <version> [--notices]
#   <bundle-dir>  target/aarch64-apple-darwin/release/bundle
#   --notices     also require Contents/Resources/THIRD-PARTY-NOTICES.txt (release builds)
set -euo pipefail

bundle=${1:?bundle directory}
version=${2:?version}
notices=${3:-}
app="$bundle/macos/Voltip.app"
dmg="$bundle/dmg/Voltip_${version}_aarch64.dmg"

codesign --verify --deep --strict --verbose=2 "$app"
signature=$(codesign -dv "$app" 2>&1)
printf '%s\n' "$signature"
grep -q '^Signature=adhoc' <<<"$signature" || { echo "::error::Voltip.app is not signed ad hoc" >&2; exit 1; }

while IFS= read -r file; do
	minos=$(otool -l "$file" | awk '/LC_BUILD_VERSION/ { b = 1 } b && $1 == "minos" { print $2; exit }')
	echo "$file: minos ${minos:-unknown}"
	python3 -c 'import sys; v = tuple(int(p) for p in sys.argv[1].split(".")); sys.exit(0 if v <= (11, 0) else 1)' "${minos:-99}" ||
		{ echo "::error::$file needs macOS ${minos:-unknown}, newer than the documented macOS 11" >&2; exit 1; }
done < <(find "$app/Contents/MacOS" "$app/Contents/Frameworks" -type f)

if [ "$notices" = --notices ]; then
	test -s "$app/Contents/Resources/THIRD-PARTY-NOTICES.txt" || { echo "::error::the app carries no THIRD-PARTY-NOTICES.txt" >&2; exit 1; }
fi
reported=$("$app/Contents/MacOS/voltip-desktop" --version)
[ "$reported" = "voltip $version" ] || { echo "::error::the app reports '$reported', not voltip $version" >&2; exit 1; }

hdiutil verify "$dmg"
mnt=$(mktemp -d)
hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null
trap 'hdiutil detach "$mnt" >/dev/null || true' EXIT
[ -L "$mnt/Applications" ] || { echo "::error::the dmg has no Applications link" >&2; exit 1; }
codesign --verify --deep --strict "$mnt/Voltip.app"
echo "check-macos-bundle: $(du -h "$dmg" | cut -f1) dmg with the app and the Applications link; ad-hoc seal intact; minos <= 11.0"
