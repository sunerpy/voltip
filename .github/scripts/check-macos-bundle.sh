#!/usr/bin/env bash
# Check a macOS build the way a Mac that downloads it meets it (release-candidate.yml bundle-macos and the
# macos jobs of ci.yml): a sealed signature — the project's release certificate for a release, ad hoc
# otherwise — so Gatekeeper offers "Open Anyway" instead of calling the app damaged; every Mach-O built for
# the architecture the dmg is named for and for no macOS newer than the documented 11; the version the
# build names; and a dmg that mounts with the app and the Applications link.
#
# Usage: .github/scripts/check-macos-bundle.sh <bundle-dir> <version> <aarch64|x64> [--notices]
#                                              [--expect-requirement REQ]
#   <bundle-dir>  target/<aarch64|x86_64>-apple-darwin/release/bundle
#   <arch>        the dmg's architecture word: aarch64 (Apple silicon) or x64 (Intel)
#   --notices     also require Contents/Resources/THIRD-PARTY-NOTICES.txt (release builds)
#   --expect-requirement REQ
#                 a release build: the app's designated requirement is REQ (release-targets.json
#                 macos_signing, docs/runbook.md 发布 · macOS 签名) and every Mach-O inside is signed by the
#                 same certificate (scripts/release/macos-signature.py); an ad-hoc build fails. Without it
#                 the build must be signed ad hoc, as every local and ordinary CI build is.
set -euo pipefail

bundle=${1:?bundle directory}
version=${2:?version}
arch=${3:?architecture: aarch64 or x64}
shift 3
notices=false
requirement=
while [ $# -gt 0 ]; do
	case "$1" in
	--notices) notices=true && shift ;;
	--expect-requirement) requirement=${2:?--expect-requirement needs the designated requirement} && shift 2 ;;
	*) echo "check-macos-bundle: unknown option '$1'" >&2 && exit 2 ;;
	esac
done
case "$arch" in
aarch64) machine=arm64 ;;
x64) machine=x86_64 ;;
*) echo "check-macos-bundle: unknown architecture '$arch' (aarch64 or x64)" >&2; exit 2 ;;
esac
app="$bundle/macos/Voltip.app"
dmg="$bundle/dmg/Voltip_${version}_${arch}.dmg"
signature_check="$(dirname "$0")/../../scripts/release/macos-signature.py"

codesign --verify --deep --strict --verbose=2 "$app"
signature=$(codesign -dv "$app" 2>&1)
printf '%s\n' "$signature"
if [ -n "$requirement" ]; then
	python3 "$signature_check" app "$app" --expect-requirement "$requirement" || { echo "::error::Voltip.app is not signed with the release certificate" >&2; exit 1; }
else
	grep -q '^Signature=adhoc' <<<"$signature" || { echo "::error::Voltip.app is not signed ad hoc" >&2; exit 1; }
fi

while IFS= read -r file; do
	archs=$(lipo -archs "$file")
	[ "$archs" = "$machine" ] || { echo "::error::$file is built for '$archs', not $machine" >&2; exit 1; }
	# LC_BUILD_VERSION (minos), or LC_VERSION_MIN_MACOSX (version) in older dylibs.
	minos=$(otool -l "$file" | awk '/LC_BUILD_VERSION|LC_VERSION_MIN_MACOSX/ { b = 1 } b && ($1 == "minos" || $1 == "version") { print $2; exit }')
	echo "$file: $archs, minos ${minos:-unknown}"
	python3 -c 'import sys; v = tuple(int(p) for p in sys.argv[1].split(".")); sys.exit(0 if v <= (11, 0) else 1)' "${minos:-99}" ||
		{ echo "::error::$file needs macOS ${minos:-unknown}, newer than the documented macOS 11" >&2; exit 1; }
	if [ -n "$requirement" ]; then
		python3 "$signature_check" code "$file" --expect-requirement "$requirement" || { echo "::error::$file is not signed with the release certificate" >&2; exit 1; }
	fi
done < <(find "$app/Contents/MacOS" "$app/Contents/Frameworks" -type f)

if [ "$notices" = true ]; then
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
if [ -n "$requirement" ]; then
	python3 "$signature_check" app "$mnt/Voltip.app" --expect-requirement "$requirement" || { echo "::error::the dmg's app is not signed with the release certificate" >&2; exit 1; }
	sealed="signed with the release certificate"
else
	sealed="ad-hoc seal intact"
fi
echo "check-macos-bundle: $(du -h "$dmg" | cut -f1) $arch dmg with the app and the Applications link; $sealed; $machine, minos <= 11.0"
