#!/usr/bin/env bash
# Does a new build signed with the same self-signed certificate read a keychain item the previous
# build created, without a prompt? Run on a GitHub-hosted macOS runner (passwordless sudo).
# Nothing here touches the login keychain: a temporary keychain is created and searched first.
set -euo pipefail
set -x
# The release certificate was made with OpenSSL 3; macOS's /usr/bin/openssl (LibreSSL) writes
# extensions codesign reports as "Unknown critical cert extension" and will not sign with.
openssl=$(brew --prefix openssl@3)/bin/openssl
"$openssl" version
work=$(mktemp -d)
cd "$work"
kc="$work/exp.keychain-db"
pw=experiment
security create-keychain -p "$pw" "$kc"
security set-keychain-settings -lut 21600 "$kc"
security unlock-keychain -p "$pw" "$kc"
# Search the experiment keychain first, keep the rest.
old=$(security list-keychains -d user | sed 's/[" ]//g')
# shellcheck disable=SC2086
security list-keychains -d user -s "$kc" $old

cat > probe.c <<'C'
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
  SecKeychainRef kc = NULL;
  if (SecKeychainOpen(argv[1], &kc) != 0) return 2;
  SecKeychainSetUserInteractionAllowed(false);
  const char *svc = argv[2], *acct = "user";
  if (argc > 3 && strcmp(argv[3], "add") == 0) {
    OSStatus s = SecKeychainAddGenericPassword(kc, (UInt32)strlen(svc), svc, (UInt32)strlen(acct), acct, 6, "secret", NULL);
    printf("%s add %s: %d\n", VERSION, svc, (int)s);
    return s != 0;
  }
  UInt32 len = 0; void *data = NULL;
  OSStatus s = SecKeychainFindGenericPassword(kc, (UInt32)strlen(svc), svc, (UInt32)strlen(acct), acct, &len, &data, NULL);
  printf("%s read %s: %d%s\n", VERSION, svc, (int)s, s == 0 ? " (no prompt needed)" : (s == -25308 ? " (errSecInteractionNotAllowed: a prompt would have been shown)" : ""));
  if (data) SecKeychainItemFreeContent(NULL, data);
  return 0;
}
C
clang -Wno-deprecated-declarations -DVERSION='"build-a"' -framework Security -o probe-a probe.c
clang -Wno-deprecated-declarations -DVERSION='"build-b"' -framework Security -o probe-b probe.c

# One variant per certificate: the release certificate's shape (no OU), and one with an OU.
variant() {
  local name=$1 subject=$2
  echo "::group::variant $name ($subject)"
  "$openssl" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes -subj "$subject" \
    -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:false" -keyout "$name.key" -out "$name.pem" 2>/dev/null
  "$openssl" pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
    -inkey "$name.key" -in "$name.pem" -out "$name.p12" -passout pass:x
  security import "$name.p12" -k "$kc" -f pkcs12 -P x -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pw" "$kc" >/dev/null
  sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$name.pem"
  security find-identity -v -p codesigning "$kc"
  local cn
  cn=$(sed -n 's/.*CN=\([^/]*\).*/\1/p' <<<"$subject")
  local sha
  sha=$(security find-identity -v -p codesigning "$kc" | awk -v cn="$cn" 'index($0, "\"" cn "\"") {print $2}')
  for b in a b; do
    cp "probe-$b" "$name-probe-$b"
    codesign -f -s "$sha" -i dev.voltip.experiment "$name-probe-$b"
    codesign -dvvv "$name-probe-$b" 2>&1 | grep -E "^(CDHash|TeamIdentifier|Authority)=" | sed "s/^/  $b: /"
    codesign -d -r- "$name-probe-$b" 2>&1 | grep designated | sed "s/^/  $b: /"
  done
  "./$name-probe-a" "$kc" "svc-$name" add
  "./$name-probe-a" "$kc" "svc-$name"
  "./$name-probe-b" "$kc" "svc-$name"
  echo "::endgroup::"
}
variant plain "/CN=Voltip Exp Plain"
echo "::group::control: an item made by ad-hoc code (Voltip before 0.0.7)"
cp probe-a adhoc-probe
codesign -f -s - -i dev.voltip.experiment adhoc-probe
codesign -dvvv adhoc-probe 2>&1 | grep -E "^(CDHash|TeamIdentifier|Signature)=" | sed "s/^/  adhoc: /"
./adhoc-probe "$kc" svc-adhoc add
./adhoc-probe "$kc" svc-adhoc
./plain-probe-b "$kc" svc-adhoc
echo "::endgroup::"
variant team "/CN=Voltip Exp Team/OU=ABCDE12345/O=Voltip"

echo "::group::the items' access lists"
set +e
security dump-keychain -a "$kc" >dump.txt 2>&1
echo "dump exit=$?"
for svc in svc-plain svc-adhoc svc-team; do
  echo "--- $svc"
  awk -v s="\"svce\"<blob>=\"$svc\"" 'index($0, s) {f=1} f {print} f && /^keychain:/ && ++n > 0 && !index($0, s) {exit}' dump.txt | sed -n '1,60p'
done
grep -oE '7B22[0-9A-Fa-f]+' dump.txt | while read -r hex; do echo "$hex" | xxd -r -p; echo; done
set -e
echo "::endgroup::"
security list-keychains -d user -s $old
security delete-keychain "$kc"
