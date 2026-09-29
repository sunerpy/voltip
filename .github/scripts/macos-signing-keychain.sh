#!/usr/bin/env bash
# The release signing identity of one macOS candidate leg (release-candidate.yml bundle-macos,
# docs/runbook.md 发布 · macOS 签名): the project's self-signed "Voltip Code Signing" certificate, the
# same for every release so a Mac sees every update as the same app and keeps its permissions and
# keychain access.
#
#   import   decode MACOS_CERTIFICATE (base64 .p12) into a keychain of this job only, unlocked for
#            codesign, trust the certificate for code signing (passwordless sudo on GitHub's macOS
#            runners), and check that the identity is the one MACOS_SIGNING_IDENTITY (SHA-1) and
#            --expect-sha1 name. Tauri then signs with APPLE_SIGNING_IDENTITY.
#   remove   delete that keychain again, and with it the private key. The trust setting stays:
#            removing an admin trust setting waits for an authorisation dialog nobody answers on a
#            runner (candidate 36550741255 hung there, 2026-09-29), and GitHub discards the
#            runner with the job.
#
# Usage: .github/scripts/macos-signing-keychain.sh import --expect-sha1 <SHA-1>
#        .github/scripts/macos-signing-keychain.sh remove
# Nothing secret is printed: the certificate and its password travel in the environment only.
set -euo pipefail

keychain="${RUNNER_TEMP:?RUNNER_TEMP}/voltip-signing.keychain-db"
certificate="$RUNNER_TEMP/voltip-signing.cer"

case "${1:-}" in
import)
	if [ "${2:-}" != --expect-sha1 ] || [ -z "${3:-}" ]; then
		echo "macos-signing-keychain: import --expect-sha1 <SHA-1>" >&2
		exit 2
	fi
	expected=$3
	: "${MACOS_CERTIFICATE:?MACOS_CERTIFICATE (base64 .p12) is not set}"
	: "${MACOS_CERTIFICATE_PASSWORD:?MACOS_CERTIFICATE_PASSWORD is not set}"
	: "${MACOS_SIGNING_IDENTITY:?MACOS_SIGNING_IDENTITY (certificate SHA-1) is not set}"
	if [ "$(tr '[:lower:]' '[:upper:]' <<<"$MACOS_SIGNING_IDENTITY")" != "$(tr '[:lower:]' '[:upper:]' <<<"$expected")" ]; then
		echo "::error::MACOS_SIGNING_IDENTITY is not the certificate .github/release-targets.json names ($expected)" >&2
		exit 1
	fi
	umask 077
	p12="$RUNNER_TEMP/voltip-signing.p12"
	trap 'rm -f "$p12"' EXIT
	printf '%s' "$MACOS_CERTIFICATE" | base64 --decode >"$p12"
	password=$(openssl rand -base64 24)
	security create-keychain -p "$password" "$keychain"
	security set-keychain-settings -lut 21600 "$keychain"
	security unlock-keychain -p "$password" "$keychain"
	security import "$p12" -k "$keychain" -f pkcs12 -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
	security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
	# Ours first in the search list, the runner's own after it.
	existing=()
	while IFS= read -r line; do
		line=${line//\"/}
		line=${line## }
		line=${line%% }
		[ -n "$line" ] && existing+=("$line")
	done < <(security list-keychains -d user)
	security list-keychains -d user -s "$keychain" "${existing[@]}"
	# A self-signed certificate is its own root: trusted for code signing on this runner only.
	security find-certificate -c "Voltip Code Signing" -p "$keychain" >"$certificate"
	sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$certificate"
	identities=$(security find-identity -v -p codesigning "$keychain")
	printf '%s\n' "$identities"
	grep -qi "$expected" <<<"$identities" || { echo "::error::the imported identity is not $expected" >&2; exit 1; }
	echo "macos-signing-keychain: Voltip Code Signing ($expected) ready in $keychain"
	;;
remove)
	rm -f "$certificate"
	security delete-keychain "$keychain" 2>/dev/null || true
	echo "macos-signing-keychain: removed"
	;;
*)
	echo "macos-signing-keychain: import --expect-sha1 <SHA-1> | remove" >&2
	exit 2
	;;
esac
