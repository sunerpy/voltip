#!/usr/bin/env bash
# Prepare a CI runner's Gradle for the phone app's Android build (scripts/build-android-rn.sh), so
# that one slow or failed download no longer fails the job. Nothing the build produces changes.
#   1. The Gradle distribution. The wrapper downloads it once, with a 10 s read timeout and no
#      retry; that failed CI's android job twice on 2026-11-08 (runs 42473232618 and 42477129450:
#      "Downloading … gradle-9.4.1-bin.zip failed: timeout (10000ms)"). The distribution comes
#      through fetch-pinned.sh instead (bounded retries, the SHA-256 pinned below) into the
#      directory the wrapper looks in, and the wrapper unpacks it from there without a download.
#      Which Gradle that is comes from Expo's project template (`expo prebuild` writes the
#      wrapper's properties from expo/template.tgz); another one than the pinned version fails
#      here until its digest is recorded.
#   2. Dependency downloads. Gradle tries a repository three times, one and two seconds apart,
#      before the build fails (ErrorHandlingModuleComponentRepository, Gradle 9.4.1: system
#      properties org.gradle.internal.repository.max.tentatives and .initial.backoff); a Kotlin
#      compiler jar that Maven Central redirects to github.com timed out three times and failed
#      the job (2026-11-08, run 42475602631). GRADLE_USER_HOME/gradle.properties gets eight tries,
#      two seconds apart and doubling (2, 4, … 128 s: about four minutes of waiting between them).
#      Gradle retries only failures it takes for transient: timeouts, refused or reset connections,
#      HTTP 5xx, 408 and 429.
#
# Usage: .github/scripts/prepare-gradle.sh APP_DIR          (apps/mobile-rn, after pnpm install)
#        .github/scripts/prepare-gradle.sh --dist-dir URL   (prints the wrapper's <name>/<hash>)
set -euo pipefail

GRADLE_DIST_URL="https://services.gradle.org/distributions/gradle-9.4.1-bin.zip"
# SHA-256 of gradle-9.4.1-bin.zip: the value services.gradle.org publishes beside it (.sha256) and
# the digest GitHub lists for the release asset the URL redirects to (recorded 2026-11-09).
GRADLE_DIST_SHA256="2ab2958f2a1e51120c326cad6f385153bb11ee93b3c216c5fccebfdfbb7ec6cb"
MAX_TENTATIVES=8
INITIAL_BACKOFF_MS=2000

# Where the wrapper keeps a distribution under wrapper/dists (distributionBase and zipStoreBase
# GRADLE_USER_HOME, as the template's properties say): the file name without its extension, then
# the MD5 of the URL in base 36 (Gradle's PathAssembler).
dist_dir() {
	python3 -I - "$1" <<'EOF'
import hashlib, posixpath, sys
from urllib.parse import urlsplit
url = sys.argv[1]
n = int.from_bytes(hashlib.md5(url.encode("utf-8")).digest(), "big")
digits, out = "0123456789abcdefghijklmnopqrstuvwxyz", ""
while n:
    n, r = divmod(n, 36)
    out = digits[r] + out
name = posixpath.basename(urlsplit(url).path)
print(f"{name.rsplit('.', 1)[0] if '.' in name else name}/{out or '0'}")
EOF
}

if [[ "${1:-}" == "--dist-dir" ]]; then
	[[ $# -eq 2 ]] || { echo "usage: $0 --dist-dir URL" >&2; exit 2; }
	dist_dir "$2"
	exit 0
fi
[[ $# -eq 1 && -d "$1" ]] || { echo "usage: $0 APP_DIR" >&2; exit 2; }
app=$1
here=$(cd "$(dirname "$0")" && pwd)

template=$(cd "$app" && node -p "require.resolve('expo/template.tgz')")
url=$(tar -xzOf "$template" package/android/gradle/wrapper/gradle-wrapper.properties |
	sed -n 's/^distributionUrl=//p' | tr -d '\r' | sed 's/\\:/:/g')
if [[ "$url" != "$GRADLE_DIST_URL" ]]; then
	echo "prepare-gradle: Expo's template ($template) names the Gradle distribution '$url', this script pins $GRADLE_DIST_URL: record the new one's SHA-256 here" >&2
	exit 1
fi

home=${GRADLE_USER_HOME:-$HOME/.gradle}
zip="$home/wrapper/dists/$(dist_dir "$url")/$(basename "$url")"
"$here/fetch-pinned.sh" "$url" "$GRADLE_DIST_SHA256" "$zip"

properties="$home/gradle.properties"
mkdir -p "$home"
touch "$properties"
grep -v -E '^systemProp\.org\.gradle\.internal\.repository\.(max\.tentatives|initial\.backoff)=' "$properties" >"$properties.new" || true
{
	echo "systemProp.org.gradle.internal.repository.max.tentatives=$MAX_TENTATIVES"
	echo "systemProp.org.gradle.internal.repository.initial.backoff=$INITIAL_BACKOFF_MS"
} >>"$properties.new"
mv -f "$properties.new" "$properties"
# Only these two lines: the file may hold other settings, such as a signing password on a desktop.
echo "prepare-gradle: $zip ready for the wrapper; $properties retries a repository $MAX_TENTATIVES times, $INITIAL_BACKOFF_MS ms apart and doubling"
