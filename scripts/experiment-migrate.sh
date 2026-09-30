#!/usr/bin/env bash
# Experiment (never merged): the keychain state of a user who updated from ad-hoc Voltip (≤ 0.0.6)
# and pressed 「始终允许」 for one signed build, and what a migration needs. Probes as in
# experiment-keychain.sh, with a throwaway certificate made by the release recipe, NOT trusted
# (as on a user's Mac).
set -euo pipefail
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
cd "$work"
kc="$work/migrate.keychain-db"
pw=$("$openssl" rand -hex 16)
security create-keychain -p "$pw" "$kc"
security set-keychain-settings -lut 21600 "$kc"
security unlock-keychain -p "$pw" "$kc"
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
security list-keychains -d user -s "$kc" "${existing[@]}"

cat >probe.c <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
extern OSStatus SecKeychainItemSetAccessWithPassword(SecKeychainItemRef, SecAccessRef, UInt32, const void *);
extern OSStatus SecTrustedApplicationSetRequirement(SecTrustedApplicationRef, SecRequirementRef);
// probe <kc> <service> add | read | delete | allow <app> <pw> | allow-cdhash <app> <cdhash> <pw>
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
  } else if (strcmp(op, "delete") == 0) {
    SecKeychainItemRef item = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, NULL, NULL, &item);
    if (s == 0) s = SecKeychainItemDelete(item);
  } else if ((strcmp(op, "allow") == 0 && argc == 6) || (strcmp(op, "allow-cdhash") == 0 && argc == 7)) {
    SecKeychainItemRef item = NULL; SecAccessRef access = NULL; SecTrustedApplicationRef app = NULL;
    CFArrayRef acls = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, NULL, NULL, &item);
    if (s == 0) s = SecKeychainItemCopyAccess(item, &access);
    if (s == 0) acls = SecAccessCopyMatchingACLList(access, kSecACLAuthorizationDecrypt);
    if (s == 0 && (!acls || CFArrayGetCount(acls) == 0)) s = errSecNoAccessForItem;
    if (s == 0) s = SecTrustedApplicationCreateFromPath(argv[4], &app);
    if (s == 0 && strcmp(op, "allow-cdhash") == 0) {
      // What the dialog's 「始终允许」 appears to record: this one build, by its code hash.
      char text[128]; snprintf(text, sizeof text, "cdhash H\"%s\"", argv[5]);
      CFStringRef str = CFStringCreateWithCString(NULL, text, kCFStringEncodingUTF8);
      SecRequirementRef req = NULL;
      s = SecRequirementCreateWithString(str, kSecCSDefaultFlags, &req);
      if (s == 0) s = SecTrustedApplicationSetRequirement(app, req);
    }
    for (CFIndex i = 0; s == 0 && i < CFArrayGetCount(acls); i++) {
      SecACLRef acl = (SecACLRef)CFArrayGetValueAtIndex(acls, i);
      CFArrayRef apps = NULL; CFStringRef desc = NULL; SecKeychainPromptSelector sel = 0;
      s = SecACLCopyContents(acl, &apps, &desc, &sel);
      CFMutableArrayRef more = apps ? CFArrayCreateMutableCopy(NULL, 0, apps) : CFArrayCreateMutable(NULL, 0, &kCFTypeArrayCallBacks);
      CFArrayAppendValue(more, app);
      if (s == 0) s = SecACLSetContents(acl, more, desc, sel);
    }
    const char *p = argv[argc - 1];
    if (s == 0) s = SecKeychainItemSetAccessWithPassword(item, access, (UInt32)strlen(p), p);
  } else {
    return 2;
  }
  printf("%d\n", (int)s);
  return 0;
}
C
for b in a b adhoc; do clang -Wno-deprecated-declarations -DBUILD="\"$b\"" -framework Security -framework CoreFoundation -o "probe-$b" probe.c; done
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Migrate Check" \
	-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
	-addext "basicConstraints=critical,CA:false" -keyout check.key -out check.pem 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 -inkey check.key -in check.pem -out check.p12 -passout pass:check
security import check.p12 -k "$kc" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pw" "$kc" >/dev/null
# Not trusted: codesign accepts an untrusted identity once its certificate is trusted for signing
# in *this* keychain only? Try without any trust first.
sha=$(security find-identity -p codesigning "$kc" | awk '/"Voltip Migrate Check"/ {print $2; exit}')
echo "identity: $sha (valid? $(security find-identity -v -p codesigning "$kc" | grep -c 'Voltip Migrate Check'))"
if ! codesign -f -s "$sha" -i dev.voltip.desktop probe-a 2>/dev/null; then
	echo "note: codesign needs the certificate trusted; trusting it for signing only"
	sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain check.pem
fi
codesign -f -s "$sha" -i dev.voltip.desktop probe-a probe-b
codesign -f -s - -i dev.voltip.desktop probe-adhoc
cdhash_a=$(codesign -dvvv probe-a 2>&1 | sed -n 's/^CDHash=//p')
say() { printf '%-72s %s\n' "$1" "$2"; }
say "ad-hoc stores a legacy entry" "$(./probe-adhoc "$kc" legacy add)"
say "build a reads it (no trust yet)" "$(./probe-a "$kc" legacy read)"
say "「始终允许」 for build a, by its code hash" "$(./probe-a "$kc" legacy allow-cdhash "$work/probe-a" "$cdhash_a" "$pw")"
say "build a reads it" "$(./probe-a "$kc" legacy read)"
say "build b, the next update, reads it (the user's report: refused)" "$(./probe-b "$kc" legacy read)"
say "build b deletes it (not trusted)" "$(./probe-b "$kc" legacy delete)"
./probe-adhoc "$kc" legacy2 add >/dev/null
./probe-a "$kc" legacy2 allow-cdhash "$work/probe-a" "$cdhash_a" "$pw" >/dev/null
say "build a deletes a legacy entry it may read" "$(./probe-a "$kc" legacy2 delete)"
say "build a stores the migrated entry" "$(./probe-a "$kc" migrated add)"
say "build b reads the migrated entry (the fix: no prompt)" "$(./probe-b "$kc" migrated read)"
security dump-keychain -a "$kc" 2>/dev/null | grep -E '"svce"|requirement:' || true
security list-keychains -d user -s "${existing[@]}"
security delete-keychain "$kc"
