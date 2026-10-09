#!/usr/bin/env bash
# The keychain migration end to end, with the real app (user report 2026-09-30; docs/runbook.md
# 发布 · macOS 签名; crates/voltip-identity/src/secret_store.rs `read_moving`):
#
#   1. The ad-hoc app (as Voltip was up to 0.0.6) stores its identity. The items trust its own
#      cdhash only.
#   2. The same app, signed with a certificate made by the release recipe (not trusted, as on a
#      user's Mac), is allowed to read those items, as the user's 「允许」/「始终允许」 does.
#   3. It moves them to items of its own (account `<user>.signed.<its cdhash>`), trusted by its
#      signing requirement, and removes the old ones.
#   4. Another build signed the same way moves them on to items of its own and starts past its
#      keychain read.
#
# The keychain here is made with `security create-keychain`, which skips the partition check
# (securityd `validatePartition`), so no step can ask: this checks the moves and the access lists,
# not the dialog. On a user's Mac step 4 asks once; an in-app update hands the items over instead,
# which check-keychain-preinstall.sh checks on the login keychain.
#
# Usage: .github/scripts/check-keychain-migration.sh <Voltip.app (ad hoc)>
# On a GitHub-hosted macOS runner. A temporary keychain is the user's default keychain for the
# run; the login keychain is not touched. Only the checks' results are printed, never the app's
# own log lines.
set -euo pipefail

adhoc=${1:?usage: check-keychain-migration.sh <Voltip.app>}
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
keychain="$work/migration-check.keychain-db"
password=$("$openssl" rand -hex 16)
service_prefix="dev.voltip.desktop"
entries=(voltip.identity.x25519 voltip.identity.meta)
user=${USER:?USER}

default=$(security default-keychain -d user | sed 's/[" ]//g')
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
restore() {
	pkill -f "$work/app/Voltip.app/Contents/MacOS/voltip-desktop" 2>/dev/null || true
	security default-keychain -d user -s "$default" || true
	security list-keychains -d user -s "${existing[@]}" || true
	security delete-keychain "$keychain" 2>/dev/null || true
}
trap restore EXIT
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security list-keychains -d user -s "$keychain" "${existing[@]}"
security default-keychain -d user -s "$keychain"

failed=0
fail() {
	echo "::error title=Keychain migration::$*"
	failed=1
}

# The recipe of docs/runbook.md (轮换), under another name, imported for codesign only.
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Migration Check" \
	-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
	-addext "basicConstraints=critical,CA:false" -keyout "$work/check.key" -out "$work/check.pem" 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
	-inkey "$work/check.key" -in "$work/check.pem" -out "$work/check.p12" -passout pass:check
security import "$work/check.p12" -k "$keychain" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
sha1=$(security find-identity -p codesigning "$keychain" | awk '/"Voltip Migration Check"/ {print $2; exit}')
[ -n "$sha1" ] || { echo "::error title=Keychain migration::no signing identity" >&2; exit 1; }

# sign <name> [version]: a copy of the app signed with that certificate (another version, another cdhash).
sign() {
	rm -rf "$work/$1.app"
	cp -R "$adhoc" "$work/$1.app"
	[ -z "${2:-}" ] || /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $2" "$work/$1.app/Contents/Info.plist"
	codesign --force --deep --sign "$sha1" "$work/$1.app" 2>/dev/null
}

# run <label> <bundle> <log>: start the app at one install path, as the updater replaces it;
# true once it has passed its keychain read at start-up ("compute devices" comes after the core
# started with the identity).
run() {
	rm -rf "$work/app"
	mkdir -p "$work/app"
	cp -R "$2" "$work/app/Voltip.app"
	"$work/app/Voltip.app/Contents/MacOS/voltip-desktop" >"$3" 2>&1 &
	local pid=$! i
	for i in $(seq 90); do
		if grep -q "compute devices" "$3"; then
			echo "ok: $1 started past its keychain read in ${i}s"
			kill "$pid" 2>/dev/null || true
			wait "$pid" 2>/dev/null || true
			return 0
		fi
		kill -0 "$pid" 2>/dev/null || break
		sleep 1
	done
	fail "$1 did not get past its keychain read (running: $(kill -0 "$pid" 2>/dev/null && echo yes || echo no))"
	kill "$pid" 2>/dev/null || true
	return 1
}

# trusted <entry> <account>: the requirements the item trusts.
trusted() {
	security dump-keychain -a "$keychain" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$service_prefix/$1\"" -v a="\"acct\"<blob>=\"$2\"" '
		/^keychain:/ { if (hit && acc) found = found block; block = ""; hit = 0; acc = 0 }
		{ block = block $0 "\n" }
		index($0, s) { hit = 1 }
		index($0, a) { acc = 1 }
		END { if (hit && acc) found = found block; printf "%s", found }' | sed -n 's/^ *requirement: //p'
}

cat >"$work/allow.c" <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
extern OSStatus SecKeychainItemSetAccessWithPassword(SecKeychainItemRef, SecAccessRef, UInt32, const void *);
// allow <keychain> <service> <account> <app> <keychain password>: <app> joins the item's
// trusted applications, which is what the user's 「始终允许」 does for the build it was pressed for.
int main(int argc, char **argv) {
  if (argc != 6) return 2;
  SecKeychainRef kc = NULL; SecKeychainItemRef item = NULL; SecAccessRef access = NULL;
  SecTrustedApplicationRef app = NULL; CFArrayRef acls = NULL;
  OSStatus s = SecKeychainOpen(argv[1], &kc);
  SecKeychainSetUserInteractionAllowed(false);
  if (s == 0) s = SecKeychainFindGenericPassword(kc, (UInt32)strlen(argv[2]), argv[2], (UInt32)strlen(argv[3]), argv[3], NULL, NULL, &item);
  if (s == 0) s = SecKeychainItemCopyAccess(item, &access);
  if (s == 0) acls = SecAccessCopyMatchingACLList(access, kSecACLAuthorizationDecrypt);
  if (s == 0 && (!acls || CFArrayGetCount(acls) == 0)) s = errSecNoAccessForItem;
  if (s == 0) s = SecTrustedApplicationCreateFromPath(argv[4], &app);
  for (CFIndex i = 0; s == 0 && i < CFArrayGetCount(acls); i++) {
    SecACLRef acl = (SecACLRef)CFArrayGetValueAtIndex(acls, i);
    CFArrayRef apps = NULL; CFStringRef desc = NULL; SecKeychainPromptSelector sel = 0;
    s = SecACLCopyContents(acl, &apps, &desc, &sel);
    CFMutableArrayRef more = apps ? CFArrayCreateMutableCopy(NULL, 0, apps) : CFArrayCreateMutable(NULL, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(more, app);
    if (s == 0) s = SecACLSetContents(acl, more, desc, sel);
  }
  if (s == 0) s = SecKeychainItemSetAccessWithPassword(item, access, (UInt32)strlen(argv[5]), argv[5]);
  printf("%d\n", (int)s);
  return s != 0;
}
C
clang -Wno-deprecated-declarations -framework Security -framework CoreFoundation -o "$work/allow" "$work/allow.c"

# 1. The ad-hoc app stores its identity.
run "the ad-hoc app" "$adhoc" "$work/adhoc.log" || exit 1
for entry in "${entries[@]}"; do
	req=$(trusted "$entry" "$user")
	case $req in
	cdhash*) echo "ok: $entry is stored by the ad-hoc app and trusts its cdhash only" ;;
	*) fail "$entry after the ad-hoc app: trusts '$req'" ;;
	esac
done

# 2. The signed build is allowed to read those items (the user's one answer to the prompt).
sign signed-a 1001
for entry in "${entries[@]}"; do
	"$work/allow" "$keychain" "$service_prefix/$entry" "$user" "$work/signed-a.app" "$password" >/dev/null || fail "could not allow the signed build for $entry"
done

# 3. It moves them to items of its own.
run "the signed build" "$work/signed-a.app" "$work/signed-a.log" || true
signed_a=$(codesign -dvvv "$work/signed-a.app" 2>&1 | sed -n 's/^CDHash=//p' | head -1)
moved=$(grep -c "keychain item moved to one this build owns" "$work/signed-a.log" || true)
if [ "$moved" -eq "${#entries[@]}" ]; then
	echo "ok: the signed build moved ${moved} items"
else
	fail "the signed build moved $moved items, not ${#entries[@]}"
fi
want="identifier \"dev.voltip.desktop\" and certificate leaf = H\"$(tr '[:upper:]' '[:lower:]' <<<"$sha1")\""
for entry in "${entries[@]}"; do
	req=$(trusted "$entry" "$user.signed.$signed_a")
	if [ "$req" = "$want" ]; then
		echo "ok: $entry now trusts the signing requirement: $req"
	else
		fail "$entry (owned): trusts '$req', want '$want'"
	fi
	if [ -z "$(trusted "$entry" "$user")" ]; then
		echo "ok: the old $entry is gone"
	else
		fail "the old $entry is still there"
	fi
done

# 4. The next build moves them on to items of its own (no dialog in this keychain, see above).
sign signed-b 1002
run "the next signed build" "$work/signed-b.app" "$work/signed-b.log" || true
exit "$failed"
