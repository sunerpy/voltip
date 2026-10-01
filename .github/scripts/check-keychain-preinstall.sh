#!/usr/bin/env bash
# The pre-install keychain hand-over of a macOS in-app update, end to end on the LOGIN keychain.
# The updater-verified app is staged while the running bundle still exists; both processes validate
# the other's designated requirement, the staged build writes and reads back its own cdhash items,
# and only then acknowledges. No secret value is printed.
#
# Usage: check-keychain-preinstall.sh <Voltip.app (ad hoc)> <keychain_preinstall binary>
set -euo pipefail

adhoc=${1:?usage: check-keychain-preinstall.sh <Voltip.app> <keychain_preinstall binary>}
harness_bin=${2:?usage: check-keychain-preinstall.sh <Voltip.app> <keychain_preinstall binary>}
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
login="$HOME/Library/Keychains/login.keychain-db"
signkc="$work/sign.keychain-db"
password=$($openssl rand -hex 16)
service_prefix=dev.voltip.desktop
entries=(voltip.identity.x25519 voltip.identity.meta)
user="voltip-preinstall-$$"
bad_user="$user-bad"
existing=()
app="$work/good/Voltip.app/Contents/MacOS/voltip-desktop"
app_pid=

while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)

accounts() { # accounts <entry> <user>: attributes only, never values
	security dump-keychain "$login" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$service_prefix/$1\"" -v p="$2.signed." '
		/^keychain: / { if (hit && index(acct, p) == 1) print acct; hit = 0; acct = "" }
		index($0, s) { hit = 1 }
		/"acct"<blob>=/ { acct = $0; sub(/.*"acct"<blob>="/, "", acct); sub(/"$/, "", acct) }
		END { if (hit && index(acct, p) == 1) print acct }' | sort | tr '\n' ' ' | sed 's/ $//'
}
forget_user() { # delete only this run's random accounts
	local who=$1 entry account
	for entry in "${entries[@]}"; do
		while IFS= read -r account; do
			[ -z "$account" ] || security delete-generic-password -s "$service_prefix/$entry" -a "$account" "$login" >/dev/null 2>&1 || true
		done < <(accounts "$entry" "$who" | tr ' ' '\n')
	done
}
restore() {
	[ -z "${app_pid:-}" ] || { kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; }
	forget_user "$user"
	forget_user "$bad_user"
	security list-keychains -d user -s "${existing[@]}" >/dev/null 2>&1 || true
	security delete-keychain "$signkc" >/dev/null 2>&1 || true
	rm -rf "$work"
}
trap restore EXIT INT TERM
forget_user "$user"
forget_user "$bad_user"

fail() { echo "::error title=Keychain pre-install hand-over::$*"; exit 1; }
cdhash() { codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p' | head -1; }
make_tar() { local bundle=$1 out=$2; (cd "$(dirname "$bundle")" && COPYFILE_DISABLE=1 tar -czf "$out" "$(basename "$bundle")"); }
past() { local log=$1; for _ in $(seq 60); do grep -q 'compute devices' "$log" && return 0; sleep 1; done; return 1; }
stop_app() {
	[ -z "${app_pid:-}" ] || { kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; app_pid=; }
}

security create-keychain -p "$password" "$signkc"
security set-keychain-settings -lut 21600 "$signkc"
security unlock-keychain -p "$password" "$signkc"
security list-keychains -d user -s "${existing[@]}" "$signkc"
$openssl req -x509 -newkey rsa:2048 -sha256 -days 1 -nodes -subj '/CN=Voltip Preinstall Check' \
	-addext 'keyUsage=critical,digitalSignature' -addext 'extendedKeyUsage=critical,codeSigning' \
	-addext 'basicConstraints=critical,CA:false' -keyout "$work/check.key" -out "$work/check.pem" 2>/dev/null
$openssl pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
	-inkey "$work/check.key" -in "$work/check.pem" -out "$work/check.p12" -passout pass:check
security import "$work/check.p12" -k "$signkc" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$signkc" >/dev/null
sha1=$(security find-identity -p codesigning "$signkc" | awk '/Voltip Preinstall Check/ {print $2; exit}')
[ -n "$sha1" ] || fail 'no signing identity'

mkdir -p "$work/good" "$work/bad"
cp -R "$adhoc" "$work/good/Voltip.app"
/usr/libexec/PlistBuddy -c 'Set :CFBundleVersion 9101' "$work/good/Voltip.app/Contents/Info.plist"
codesign --force --deep --sign "$sha1" "$work/good/Voltip.app" 2>/dev/null
cp "$harness_bin" "$work/harness"
for dylib in "$(dirname "$harness_bin")"/*.dylib; do
	[ -f "$dylib" ] && cp "$dylib" "$work/"
done
codesign --force --sign "$sha1" --identifier dev.voltip.desktop "$work/harness" 2>/dev/null
make_tar "$work/good/Voltip.app" "$work/B.app.tar.gz"
cp -R "$work/good/Voltip.app" "$work/bad/Voltip.app"
codesign --force --deep --sign - "$work/bad/Voltip.app" 2>/dev/null
make_tar "$work/bad/Voltip.app" "$work/bad.app.tar.gz"
old=$(cdhash "$work/harness")
b=$(cdhash "$work/good/Voltip.app")
bad=$(cdhash "$work/bad/Voltip.app")
[ -n "$old" ] && [ -n "$b" ] && [ -n "$bad" ] && [ "$old" != "$b" ] || fail "cdhashes: old=$old B=$b bad=$bad"
want="identifier \"dev.voltip.desktop\" and certificate leaf = H\"$(tr '[:upper:]' '[:lower:]' <<<"$sha1")\""
for signed in "$work/harness" "$work/good/Voltip.app"; do
	got=$(codesign -d -r- "$signed" 2>&1 | sed -n 's/^designated => //p')
	[ "$got" = "$want" ] || fail "$signed requirement '$got', want '$want'"
done

# Positive: staged B persists before ACK; normal B startup reads its own items and removes old.
USER="$user" "$work/harness" "$work/B.app.tar.gz" >"$work/prepare.log" 2>&1
grep -q 'prepared 2 entries' "$work/prepare.log" || fail "staged B did not persist: $(tail -5 "$work/prepare.log")"
for entry in "${entries[@]}"; do
	got=$(accounts "$entry" "$user")
	want_accounts=$(printf '%s\n%s\n' "$user.signed.$old" "$user.signed.$b" | sort | tr '\n' ' ' | sed 's/ $//')
	[ "$got" = "$want_accounts" ] || fail "$entry after staging: '$got', want '$want_accounts'"
done
USER="$user" "$app" --start-hidden >"$work/B.log" 2>&1 & app_pid=$!
past "$work/B.log" || fail "installed B did not start: $(tail -10 "$work/B.log")"
! grep -q 'macOS asks once\|created device identity' "$work/B.log" || fail 'installed B fell back to an older item or a new identity'
stop_app
for entry in "${entries[@]}"; do
	got=$(accounts "$entry" "$user")
	[ "$got" = "$user.signed.$b" ] || fail "$entry after installed B: '$got'"
done
echo 'ok: staged B persisted before ACK; installed B read its own partition and removed the old one'

# Negative: a staged app with another requirement is refused before it receives any entry.
set +e
USER="$bad_user" "$work/harness" "$work/bad.app.tar.gz" >"$work/bad.log" 2>&1
bad_status=$?
set -e
[ "$bad_status" -ne 0 ] || fail 'the wrong-signature staged app was accepted'
grep -q 'not this app, signed as this build is\|before the hand-over' "$work/bad.log" || fail "wrong-signature reason missing: $(tail -10 "$work/bad.log")"
for entry in "${entries[@]}"; do
	got=$(accounts "$entry" "$bad_user")
	[ "$got" = "$bad_user.signed.$old" ] || fail "$entry after refused staged app: '$got'"
done
echo 'ok: a wrong-signature staged app was stopped before any hand-over'
