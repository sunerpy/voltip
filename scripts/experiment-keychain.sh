#!/usr/bin/env bash
# Experiment (never merged): what a new build signed with the release certificate's recipe may read
# from the keychain without a prompt. Run on a GitHub-hosted macOS runner (passwordless sudo).
set -euo pipefail
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
cd "$work"
kc="$work/check.keychain-db"
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

cat > probe.c <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
// Exported by Security.framework but not in its public headers (the `security` tool uses it).
extern OSStatus SecKeychainItemSetAccessWithPassword(SecKeychainItemRef, SecAccessRef, UInt32, const void *);
// probe <keychain> <service> add|read|allow <app> <keychain password>
int main(int argc, char **argv) {
  SecKeychainRef kc = NULL;
  if (argc < 4 || SecKeychainOpen(argv[1], &kc) != 0) return 2;
  SecKeychainSetUserInteractionAllowed(false);
  const char *svc = argv[2], *acct = "user", *op = argv[3];
  UInt32 sl = (UInt32)strlen(svc), al = (UInt32)strlen(acct);
  OSStatus s;
  if (strcmp(op, "add") == 0) {
    s = SecKeychainAddGenericPassword(kc, sl, svc, al, acct, 6, "secret", NULL);
  } else if (strcmp(op, "read") == 0) {
    UInt32 len = 0; void *data = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, &len, &data, NULL);
    if (data) SecKeychainItemFreeContent(NULL, data);
  } else if (strcmp(op, "allow") == 0 && argc == 6) {
    // What 「始终允许」 does: the app at argv[4] joins the item's trusted applications.
    SecKeychainItemRef item = NULL; SecAccessRef access = NULL; SecTrustedApplicationRef app = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, NULL, NULL, &item);
    if (s == 0) s = SecKeychainItemCopyAccess(item, &access);
    CFArrayRef acls = s == 0 ? SecAccessCopyMatchingACLList(access, kSecACLAuthorizationDecrypt) : NULL;
    if (s == 0 && (!acls || CFArrayGetCount(acls) == 0)) s = -1;
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
  } else {
    return 2;
  }
  printf("%s %s %s: %d\n", BUILD, op, svc, (int)s);
  return 0;
}
C
clang -Wno-deprecated-declarations -DBUILD='"build-a"' -framework Security -framework CoreFoundation -o probe-a probe.c
clang -Wno-deprecated-declarations -DBUILD='"build-b"' -framework Security -framework CoreFoundation -o probe-b probe.c
clang -Wno-deprecated-declarations -DBUILD='"adhoc"' -framework Security -framework CoreFoundation -o probe-adhoc probe.c

# The release certificate's recipe (docs/runbook.md 发布 · macOS 签名), with another name.
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Keychain Check" \
  -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
  -addext "basicConstraints=critical,CA:false" -keyout check.key -out check.pem 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
  -inkey check.key -in check.pem -out check.p12 -passout pass:check
security import check.p12 -k "$kc" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pw" "$kc" >/dev/null
sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain check.pem
sha=$(security find-identity -v -p codesigning "$kc" | awk '/"Voltip Keychain Check"/ {print $2}')
for b in a b; do codesign -f -s "$sha" -i dev.voltip.desktop "probe-$b"; done
codesign -f -s - -i dev.voltip.desktop probe-adhoc
for b in a b adhoc; do codesign -d -r- "probe-$b" 2>&1 | grep designated; done

./probe-a "$kc" made-by-a add
./probe-b "$kc" made-by-a read
./probe-adhoc "$kc" made-by-adhoc add
./probe-b "$kc" made-by-adhoc read
./probe-a "$kc" made-by-adhoc allow "$work/probe-a" "$pw"
./probe-b "$kc" made-by-adhoc read
security dump-keychain -a "$kc" 2>/dev/null | grep -E '"svce"|requirement' || true

security list-keychains -d user -s "${existing[@]}"
security delete-keychain "$kc"
