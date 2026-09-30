#!/usr/bin/env bash
# The keychain hand-over of an in-app update, end to end on the LOGIN keychain (user report
# 2026-09-30: every update asked for voltip.identity.*, because each build signed with the
# self-signed certificate has a keychain partition of its own; docs/runbook.md 发布 · macOS 签名与
# 钥匙串, crates/voltip-identity per_build.rs and handoff.rs, apps/desktop/src-tauri
# keychain_handoff.rs). A keychain made with `security create-keychain` skips the partition check,
# so this uses the runner's login keychain, as a user's Mac has it.
#
#   1. The harness (crates/voltip-identity/examples/keychain_handoff.rs), signed like the app,
#      stands in for the build an update replaces: it keeps a device identity in items of its own.
#   2. It starts the app (build B) with the identity handed over, as an update does. B starts past
#      its keychain read without asking, keeps that identity, stores it in items of its own and
#      removes the harness's.
#   3. B started again reads its own items, without asking.
#   4. Build C, started without a hand-over (a hand install), goes for B's items and has to ask:
#      nobody answers here, so it stops at the keychain read. That is the negative control.
#
# Usage: .github/scripts/check-keychain-handoff.sh <Voltip.app (ad hoc)> <keychain_handoff binary>
# On a GitHub-hosted macOS runner. The signing certificate sits in a keychain of this job only; in
# the login keychain only Voltip's items are touched, and removed at the end. Only the checks'
# results are printed, never the app's own log lines.
set -euo pipefail

adhoc=${1:?usage: check-keychain-handoff.sh <Voltip.app> <keychain_handoff binary>}
harness_bin=${2:?usage: check-keychain-handoff.sh <Voltip.app> <keychain_handoff binary>}
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
login="$HOME/Library/Keychains/login.keychain-db"
signkc="$work/sign.keychain-db"
password=$("$openssl" rand -hex 16)
service_prefix="dev.voltip.desktop"
entries=(voltip.identity.x25519 voltip.identity.meta)
user=${USER:?USER}

existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
forget() { # forget: every Voltip identity item in the login keychain
	local entry
	for entry in "${entries[@]}"; do
		while security delete-generic-password -s "$service_prefix/$entry" "$login" >/dev/null 2>&1; do :; done
	done
}
restore() {
	pkill -f "$work/app/Voltip.app/Contents/MacOS/voltip-desktop" 2>/dev/null || true
	forget
	security list-keychains -d user -s "${existing[@]}" || true
	security delete-keychain "$signkc" 2>/dev/null || true
}
trap restore EXIT
forget

failed=0
fail() {
	echo "::error title=Keychain hand-over::$*"
	failed=1
}

# The recipe of docs/runbook.md (轮换), under another name, imported for codesign only.
security create-keychain -p "$password" "$signkc"
security set-keychain-settings -lut 21600 "$signkc"
security unlock-keychain -p "$password" "$signkc"
security list-keychains -d user -s "${existing[@]}" "$signkc"
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Handoff Check" \
	-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
	-addext "basicConstraints=critical,CA:false" -keyout "$work/check.key" -out "$work/check.pem" 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
	-inkey "$work/check.key" -in "$work/check.pem" -out "$work/check.p12" -passout pass:check
security import "$work/check.p12" -k "$signkc" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$signkc" >/dev/null
sha1=$(security find-identity -p codesigning "$signkc" | awk '/"Voltip Handoff Check"/ {print $2; exit}')
[ -n "$sha1" ] || { echo "::error title=Keychain hand-over::no signing identity" >&2; exit 1; }

# sign <name> <version>: a copy of the app signed with the certificate (another version, another cdhash).
sign() {
	rm -rf "$work/$1.app"
	cp -R "$adhoc" "$work/$1.app"
	/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $2" "$work/$1.app/Contents/Info.plist"
	codesign --force --deep --sign "$sha1" "$work/$1.app" 2>/dev/null
}
cdhash() {
	codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p' | head -1
}
# install <bundle>: put it at the one install path, as the updater leaves it.
install() {
	rm -rf "$work/app"
	mkdir -p "$work/app"
	cp -R "$1" "$work/app/Voltip.app"
}
app="$work/app/Voltip.app/Contents/MacOS/voltip-desktop"
# past <log> <seconds>: whether the app got past its keychain read at start-up ("compute devices"
# comes after the core started with the identity) within that time.
past() {
	for _ in $(seq "$2"); do
		grep -q "compute devices" "$1" && return 0
		sleep 1
	done
	return 1
}
stop() {
	pkill -f "$app" 2>/dev/null || true
	for _ in $(seq 20); do pgrep -f "$app" >/dev/null || return 0; sleep 0.5; done
	pkill -9 -f "$app" 2>/dev/null || true
}
# accounts <entry>: the accounts the login keychain keeps it under (attributes only).
accounts() {
	security dump-keychain "$login" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$service_prefix/$1\"" '
		/^keychain: / { if (hit) print acct; hit = 0; acct = "" }
		index($0, s) { hit = 1 }
		/"acct"<blob>=/ { acct = $0; sub(/.*"acct"<blob>="/, "", acct); sub(/"$/, "", acct) }
		END { if (hit) print acct }' | sort | tr '\n' ' ' | sed 's/ $//'
}

sign B 3001
sign C 3002
cp "$harness_bin" "$work/harness"
codesign --force --sign "$sha1" --identifier dev.voltip.desktop "$work/harness" 2>/dev/null
harness=$(cdhash "$work/harness")
b=$(cdhash "$work/B.app")
c=$(cdhash "$work/C.app")
[ -n "$harness" ] && [ -n "$b" ] && [ -n "$c" ] && [ "$b" != "$c" ] || { echo "::error title=Keychain hand-over::cdhashes: harness '$harness', B '$b', C '$c'" >&2; exit 1; }

# 1. The build an update replaces keeps an identity in items of its own.
device=$("$work/harness" create "$harness" | sed -n 's/^device //p')
[ -n "$device" ] || { echo "::error title=Keychain hand-over::the harness stored no identity" >&2; exit 1; }
for entry in "${entries[@]}"; do
	got=$(accounts "$entry")
	[ "$got" = "$user.signed.$harness" ] && echo "ok: $entry kept by the old build under $got" || fail "$entry after the old build: '$got'"
done

# 2. It starts B with the identity handed over.
install "$work/B.app"
"$work/harness" handoff "$harness" "$app" >"$work/B.log" 2>&1 &
if past "$work/B.log" 60; then
	echo "ok: B started past its keychain read"
else
	fail "B did not get past its keychain read"
fi
grep -q "keychain hand-over received from the previous build" "$work/B.log" && echo "ok: B took the hand-over" || fail "B took no hand-over"
grep -q "handed over" "$work/B.log" && echo "ok: the old build handed over" || fail "the old build did not hand over"
! grep -q "macOS asks once" "$work/B.log" || fail "B went for an older build's item"
! grep -q "created device identity" "$work/B.log" || fail "B created a new identity instead of keeping the old one"
stop
for entry in "${entries[@]}"; do
	got=$(accounts "$entry")
	[ "$got" = "$user.signed.$b" ] && echo "ok: $entry is B's own now, the old build's is gone" || fail "$entry after B: '$got', want '$user.signed.$b'"
done

# 3. B again, without a hand-over: its own items.
"$app" >"$work/B-again.log" 2>&1 &
if past "$work/B-again.log" 60 && ! grep -q "macOS asks once" "$work/B-again.log"; then
	echo "ok: B started again past its keychain read without asking"
else
	fail "B started again did not read its own items silently"
fi
stop

# 4. C without a hand-over (a hand install) has to ask for B's items: nobody answers.
install "$work/C.app"
"$app" >"$work/C.log" 2>&1 &
if past "$work/C.log" 30; then
	fail "C got past its keychain read without a hand-over: the partition check did not stop it"
else
	echo "ok: C, without a hand-over, stopped at the keychain read"
fi
grep -q "macOS asks once" "$work/C.log" && echo "ok: C went for an earlier build's item" || fail "C did not go for an earlier build's item"
stop
exit "$failed"
