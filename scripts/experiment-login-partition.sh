#!/usr/bin/env bash
# Experiment (never merged): does this runner's LOGIN keychain enforce partition lists, as a real
# Mac's does? (User report 2026-09-30: every update prompts; the items carry
# `partition_id: cdhash:<build>`.) securityd skips the check on databases older than
# CommonBlob::version_partition, which is what `security create-keychain` made for every check so
# far. Two probes signed with the same self-signed certificate (different cdhashes) create, list,
# read and delete an item, in the login keychain and in a created one, with user interaction off.
set -uo pipefail
openssl=$(brew --prefix openssl@3)/bin/openssl
work=$(mktemp -d)
cd "$work" || exit 1
login="$HOME/Library/Keychains/login.keychain-db"

echo "== keychains"
security list-keychains -d user
security default-keychain -d user
ls -la "$HOME/Library/Keychains/" | sed -n '1,12p'
security show-keychain-info "$login" 2>&1 || true

# Signing identity in a keychain of its own (the recipe of docs/runbook.md 轮换).
signkc="$work/sign.keychain-db"
pw=$("$openssl" rand -hex 16)
security create-keychain -p "$pw" "$signkc"
security set-keychain-settings -lut 21600 "$signkc"
security unlock-keychain -p "$pw" "$signkc"
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
security list-keychains -d user -s "${existing[@]}" "$signkc"
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "/CN=Voltip Partition Check" \
	-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
	-addext "basicConstraints=critical,CA:false" -keyout check.key -out check.pem 2>/dev/null
"$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
	-inkey check.key -in check.pem -out check.p12 -passout pass:check
security import check.p12 -k "$signkc" -f pkcs12 -P check -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pw" "$signkc" >/dev/null
sha1=$(security find-identity -p codesigning "$signkc" | awk '/"Voltip Partition Check"/ {print $2; exit}')
echo "signing identity: ${sha1:-none}"

cat > probe.c <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
#ifndef BUILD
#define BUILD 0
#endif
#define STR2(x) #x
#define STR(x) STR2(x)
static const char *build = "probe build " STR(BUILD);
// probe add|read|list|delete <service> [keychain]; the default keychain when none is given.
int main(int argc, char **argv) {
  if (argc < 3) return 2;
  SecKeychainSetUserInteractionAllowed(false);
  const char *op = argv[1], *svc = argv[2], *acct = "probe";
  SecKeychainRef kc = NULL;
  if (argc > 3 && SecKeychainOpen(argv[3], &kc) != 0) return 2;
  UInt32 sl = (UInt32)strlen(svc), al = (UInt32)strlen(acct);
  OSStatus s;
  if (strcmp(op, "add") == 0) {
    s = SecKeychainAddGenericPassword(kc, sl, svc, al, acct, 6, "secret", NULL);
  } else if (strcmp(op, "read") == 0) {
    UInt32 n = 0; void *d = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, &n, &d, NULL);
    if (s == 0) SecKeychainItemFreeContent(NULL, d);
  } else if (strcmp(op, "list") == 0) {
    CFStringRef service = CFStringCreateWithCString(NULL, svc, kCFStringEncodingUTF8);
    const void *keys[] = {kSecClass, kSecAttrService, kSecReturnAttributes, kSecMatchLimit};
    const void *vals[] = {kSecClassGenericPassword, service, kCFBooleanTrue, kSecMatchLimitAll};
    CFDictionaryRef q = CFDictionaryCreate(NULL, keys, vals, 4, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFTypeRef out = NULL;
    s = SecItemCopyMatching(q, &out);
    if (s == 0) printf("list found %ld item(s)\n", (long)CFArrayGetCount((CFArrayRef)out));
  } else if (strcmp(op, "delete") == 0) {
    SecKeychainItemRef it = NULL;
    s = SecKeychainFindGenericPassword(kc, sl, svc, al, acct, NULL, NULL, &it);
    if (s == 0) s = SecKeychainItemDelete(it);
  } else {
    return 2;
  }
  printf("%s %s (%s): %d\n", op, svc, build, (int)s);
  return s != 0;
}
C
clang -DBUILD=1 -Wno-deprecated-declarations -framework Security -framework CoreFoundation -o probe-a probe.c
clang -DBUILD=2 -Wno-deprecated-declarations -framework Security -framework CoreFoundation -o probe-b probe.c
for p in probe-a probe-b; do
	codesign --force --sign "$sha1" --identifier dev.voltip.probe "$p" 2>&1 | tail -1
	codesign -dvvv "$p" 2>&1 | grep -E "^CDHash=|^TeamIdentifier=|^Authority=" | tr '\n' ' '
	echo
done

acl() { # acl <keychain> <service>: the item's partition and requirement lines
	security dump-keychain -a "$1" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$2\"" '
		/^keychain: / { if (h) printf "%s", b; b = ""; h = 0 }
		{ b = b $0 "\n" }
		index($0, s) { h = 1 }
		END { if (h) printf "%s", b }' | grep -E '"acct"|authorizations|description|requirement'
}

run() { # run <label> <keychain or empty>
	local label=$1 kc=${2:-} svc="exp.partition.$1"
	echo "== $label"
	./probe-a add "$svc" ${kc:+"$kc"}
	acl "${kc:-$login}" "$svc"
	./probe-b list "$svc" ${kc:+"$kc"}
	./probe-b read "$svc" ${kc:+"$kc"}
	./probe-a read "$svc" ${kc:+"$kc"}
	./probe-b delete "$svc" ${kc:+"$kc"}
	./probe-a read "$svc" ${kc:+"$kc"}
	./probe-a delete "$svc" ${kc:+"$kc"} || true
}

run login ""
made="$work/made.keychain-db"
security create-keychain -p "$pw" "$made"
security unlock-keychain -p "$pw" "$made"
run created "$made"
echo "== error codes: -25308 errSecInteractionNotAllowed, -25293 errSecAuthFailed, -25300 errSecItemNotFound"
