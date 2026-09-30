#!/usr/bin/env bash
# Keychain access across updates (docs/runbook.md 发布 · macOS 签名; manual checklist items 14–15):
# a keychain entry that a build signed the way releases are made stored is read by the next build
# without a prompt. An entry that ad-hoc code stored (Voltip before 0.0.7) is not, until one
# 「始终允许」 adds a signed build to it; from then on the next build reads it too. (User report
# 2026-09-30: updating 0.0.7 → 0.0.10 asked once more for voltip.identity.*, an entry from the
# ad-hoc days that the 0.0.7 prompt had only let through once.)
#
# A throwaway certificate made with the release recipe, in a keychain of this job only, and three
# probes of the same code built apart (so each has its own cdhash, like two releases). Each probe
# adds, reads or allows a generic password with user interaction off: where a dialog would be
# needed it gets an error instead. On GitHub's macOS runners: passwordless sudo trusts the
# certificate for code signing, and that trust setting stays, as in macos-signing-keychain.sh.
set -euo pipefail

# The release certificate was made with OpenSSL 3; macOS's own /usr/bin/openssl (LibreSSL) writes
# extensions that codesign reports as unknown and will not sign with.
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
keychain="$work/keychain-check.keychain-db"
password=$("$openssl" rand -hex 16)
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
restore() {
	security list-keychains -d user -s "${existing[@]}" || true
	security delete-keychain "$keychain" 2>/dev/null || true
}
trap restore EXIT
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security list-keychains -d user -s "$keychain" "${existing[@]}"

cat >"$work/probe.c" <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
// Exported by Security.framework, not declared in its public headers (the `security` tool uses it).
extern OSStatus SecKeychainItemSetAccessWithPassword(SecKeychainItemRef, SecAccessRef, UInt32, const void *);
// probe <keychain> <service> add | read | allow <app> <keychain password>; prints the OSStatus.
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
  } else if (strcmp(op, "allow") == 0 && argc == 6) {
    // What 「始终允许」 does: the application at argv[4] joins the entry's trusted applications.
    SecKeychainItemRef item = NULL; SecAccessRef access = NULL; SecTrustedApplicationRef app = NULL;
    CFArrayRef acls = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, NULL, NULL, &item);
    if (s == 0) s = SecKeychainItemCopyAccess(item, &access);
    if (s == 0) acls = SecAccessCopyMatchingACLList(access, kSecACLAuthorizationDecrypt);
    if (s == 0 && (!acls || CFArrayGetCount(acls) == 0)) s = errSecNoAccessForItem;
    if (s == 0) s = SecTrustedApplicationCreateFromPath(argv[4], &app);
    for (CFIndex i = 0; s == 0 && i < CFArrayGetCount(acls); i++) {
      SecACLRef acl = (SecACLRef)CFArrayGetValueAtIndex(acls, i);
      CFArrayRef apps = NULL; CFStringRef desc = NULL; SecKeychainPromptSelector selector = 0;
      s = SecACLCopyContents(acl, &apps, &desc, &selector);
      CFMutableArrayRef more = apps ? CFArrayCreateMutableCopy(NULL, 0, apps) : CFArrayCreateMutable(NULL, 0, &kCFTypeArrayCallBacks);
      CFArrayAppendValue(more, app);
      if (s == 0) s = SecACLSetContents(acl, more, desc, selector);
    }
    if (s == 0) s = SecKeychainItemSetAccessWithPassword(item, access, (UInt32)strlen(argv[5]), argv[5]);
  } else {
    return 2;
  }
  printf("%d\n", (int)s);
  return 0;
}
C
for build in a b adhoc; do
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
sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$work/check.pem"
sha1=$(security find-identity -v -p codesigning "$keychain" | awk '/"Voltip Keychain Check"/ {print $2}')
[ -n "$sha1" ] || { echo "::error title=Keychain across updates::the check's certificate is not a signing identity" >&2; exit 1; }
codesign -f -s "$sha1" -i dev.voltip.desktop "$work/probe-a" "$work/probe-b"
codesign -f -s - -i dev.voltip.desktop "$work/probe-adhoc"

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

# expect ok|refused <what> <probe> <service> <op> [args]
expect() {
	local want=$1 what=$2
	shift 2
	local status
	status=$("$work/probe-$1" "$keychain" "${@:2}")
	if { [ "$want" = ok ] && [ "$status" = 0 ]; } || { [ "$want" = refused ] && [ "$status" != 0 ]; }; then
		echo "ok: $what (OSStatus $status)"
	else
		echo "::error title=Keychain across updates::$what: OSStatus $status, expected $want"
		failed=1
	fi
}
expect ok "build a stores an entry" a stored-by-a add
expect ok "build b, a later build, reads it without a prompt" b stored-by-a read
expect ok "ad-hoc code stores an entry (Voltip before 0.0.7)" adhoc stored-ad-hoc add
expect refused "build b cannot read that entry without a prompt" b stored-ad-hoc read
expect ok "「始终允许」 adds build a to that entry" a stored-ad-hoc allow "$work/probe-a" "$password"
expect ok "build b reads that entry without a prompt from then on" b stored-ad-hoc read

security dump-keychain -a "$keychain" 2>/dev/null | grep -E '"svce"|requirement:' || true
exit "$failed"
