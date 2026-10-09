#!/usr/bin/env bash
# Keychain partitions across updates (docs/runbook.md 发布 · macOS 签名与钥匙串; manual checklist
# item 15): the platform facts the in-app update's hand-over rests on (crates/voltip-identity
# per_build.rs and handoff.rs, check-keychain-preinstall.sh).
#
# User report 2026-09-30: every update asked for voltip.identity.*. In the LOGIN keychain every item
# carries a partition list, and a build signed with a self-signed certificate (no Apple Team ID)
# has the partition `cdhash:<that build>`. So a later build, signed the same way, is refused an
# item an earlier build stored (a user sees the dialog) — but it can list such an item and remove
# it without reading it. Keychains made with `security create-keychain` skip the partition check;
# the checks before this one used such a keychain, which is how they missed it.
#
# A throwaway certificate made with the release recipe, in a keychain of this job used only for
# signing, and two probes of the same code built apart (so each has its own cdhash, like two
# releases). Each probe works on the keychain it is given with user interaction off: where a
# dialog would be needed it gets an error instead. The login keychain of GitHub's runners is
# unlocked; only the probes' own items are touched, and removed at the end.
set -euo pipefail

# The release certificate was made with OpenSSL 3; macOS's own /usr/bin/openssl (LibreSSL) writes
# extensions that codesign reports as unknown and will not sign with.
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
keychain="$work/keychain-check.keychain-db"
login="$HOME/Library/Keychains/login.keychain-db"
password=$("$openssl" rand -hex 16)
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
restore() {
	for service in partition-login-a partition-login-b; do
		security delete-generic-password -s "$service" "$login" >/dev/null 2>&1 || true
	done
	security list-keychains -d user -s "${existing[@]}" || true
	security delete-keychain "$keychain" 2>/dev/null || true
}
trap restore EXIT
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security list-keychains -d user -s "${existing[@]}" "$keychain"

cat >"$work/probe.c" <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
// probe <keychain> <service> add | read | list | remove; prints the OSStatus. `list` asks for the
// items' attributes and `remove` for references (then deletes them), both through
// SecItemCopyMatching with the keychain as the search list, never for the value.
int main(int argc, char **argv) {
  SecKeychainRef kc = NULL;
  if (argc < 4 || SecKeychainOpen(argv[1], &kc) != 0) return 2;
  SecKeychainSetUserInteractionAllowed(false);
  const char *svc = argv[2], *acct = "voltip", *op = argv[3];
  UInt32 sl = (UInt32)strlen(svc), al = (UInt32)strlen(acct);
  OSStatus s;
  if (strcmp(op, "add") == 0) {
    s = SecKeychainAddGenericPassword(kc, sl, svc, al, acct, 6, "secret", NULL);
  } else if (strcmp(op, "read") == 0) {
    UInt32 len = 0; void *data = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, &len, &data, NULL);
    if (data) SecKeychainItemFreeContent(NULL, data);
  } else if (strcmp(op, "list") == 0 || strcmp(op, "remove") == 0) {
    int refs = strcmp(op, "remove") == 0;
    CFStringRef service = CFStringCreateWithCString(NULL, svc, kCFStringEncodingUTF8);
    CFArrayRef list = CFArrayCreate(NULL, (const void **)&kc, 1, &kCFTypeArrayCallBacks);
    const void *keys[] = {kSecClass, kSecAttrService, refs ? kSecReturnRef : kSecReturnAttributes, kSecMatchLimit, kSecMatchSearchList};
    const void *vals[] = {kSecClassGenericPassword, service, kCFBooleanTrue, kSecMatchLimitAll, list};
    CFDictionaryRef query = CFDictionaryCreate(NULL, keys, vals, 5, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFTypeRef found = NULL;
    s = SecItemCopyMatching(query, &found);
    for (CFIndex i = 0; s == 0 && refs && i < CFArrayGetCount((CFArrayRef)found); i++)
      s = SecKeychainItemDelete((SecKeychainItemRef)CFArrayGetValueAtIndex((CFArrayRef)found, i));
  } else {
    return 2;
  }
  printf("%d\n", (int)s);
  return 0;
}
C
for build in a b; do
	clang -Wno-deprecated-declarations -DBUILD="\"$build\"" -framework Security -framework CoreFoundation -o "$work/probe-$build" "$work/probe.c"
done

# The recipe of docs/runbook.md (轮换), under another name.
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Keychain Check" \
	-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
	-addext "basicConstraints=critical,CA:false" -keyout "$work/check.key" -out "$work/check.pem" 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
	-inkey "$work/check.key" -in "$work/check.pem" -out "$work/check.p12" -passout pass:check
security import "$work/check.p12" -k "$keychain" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
sha1=$(security find-identity -p codesigning "$keychain" | awk '/"Voltip Keychain Check"/ {print $2; exit}')
[ -n "$sha1" ] || { echo "::error title=Keychain across updates::the check's certificate is not a signing identity" >&2; exit 1; }
codesign -f -s "$sha1" -i dev.voltip.desktop "$work/probe-a" "$work/probe-b"

failed=0
# Both signed probes carry the requirement releases carry, with this certificate.
want="identifier \"dev.voltip.desktop\" and certificate leaf = H\"$(tr '[:upper:]' '[:lower:]' <<<"$sha1")\""
for build in a b; do
	got=$(codesign -d -r- "$work/probe-$build" 2>&1 | sed -n 's/^designated => //p')
	if [ "$got" = "$want" ]; then
		echo "ok: build $build is signed as releases are: $got"
	else
		echo "::error title=Keychain across updates::build $build: designated requirement $got, want $want"
		failed=1
	fi
done
[ "$(cksum <"$work/probe-a")" != "$(cksum <"$work/probe-b")" ] || { echo "::error::the two builds are the same file"; failed=1; }

# expect ok|refused <what> <probe> <keychain> <service> <op>
expect() {
	local want=$1 what=$2
	shift 2
	local status
	status=$("$work/probe-$1" "${@:2}")
	if { [ "$want" = ok ] && [ "$status" = 0 ]; } || { [ "$want" = refused ] && [ "$status" != 0 ]; }; then
		echo "ok: $what (OSStatus $status)"
	else
		echo "::error title=Keychain across updates::$what: OSStatus $status, expected $want"
		failed=1
	fi
}
# partition <service>: the item's partition list in the login keychain
partition() {
	security dump-keychain -a "$login" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$1\"" '
		/^keychain: / { if (h) printf "%s", b; b = ""; h = 0 }
		{ b = b $0 "\n" }
		index($0, s) { h = 1 }
		END { if (h) printf "%s", b }' | awk '/partition_id/ { p = 1; next } p && /description:/ { sub(/^ *description: /, ""); print; exit }'
}

# The login keychain, as a user's Mac has it.
expect ok "build a stores an entry in the login keychain" a "$login" partition-login-a add
got=$(partition partition-login-a)
case $got in
cdhash:*) echo "ok: its partition list is build a alone: $got" ;;
*) echo "::error title=Keychain across updates::the entry's partition list is '$got', not one cdhash"; failed=1 ;;
esac
expect ok "build a reads its own entry" a "$login" partition-login-a read
expect refused "build b, a later build, is refused that entry (a user sees the dialog)" b "$login" partition-login-a read
expect ok "build b lists that entry without reading it" b "$login" partition-login-a list
expect ok "build b removes that entry without reading it" b "$login" partition-login-a remove
expect refused "the entry is gone" a "$login" partition-login-a read

# A keychain made with `security create-keychain` skips the partition check: why the earlier
# checks, which used one, never saw the dialog.
expect ok "build a stores an entry in a created keychain" a "$keychain" partition-made add
expect ok "build b reads it there: no partition check" b "$keychain" partition-made read
exit "$failed"
