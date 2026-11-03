#!/usr/bin/env bash
# The build key's build-time contract (docs/pairing.md 「设备票据」, crates/voltip-access/build.rs),
# with a throwaway key: a keyed build gets exactly that key back and never holds it whole, a key
# for another version or a release build without a key does not build. `make verify` runs it.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib/artefact-checks.sh
version=$(python3 -c 'import json; print(json.load(open("package.json"))["version"])')
key=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
expect=$(python3 -c 'import hashlib, hmac, sys; print(hmac.new(bytes.fromhex(sys.argv[1]), b"prove|d|c|1|n", hashlib.sha256).hexdigest())' "$key")
unset VOLTIP_BUILD_KEY VOLTIP_BUILD_KEY_VERSION VOLTIP_REQUIRE_BUILD_KEY VOLTIP_EXPECT_PROOF

echo "check-build-key: a keyed build of voltip-access $version"
VOLTIP_BUILD_KEY=$key VOLTIP_BUILD_KEY_VERSION=$version VOLTIP_REQUIRE_BUILD_KEY=1 VOLTIP_EXPECT_PROOF=$expect \
  cargo test -q -p voltip-access --lib the_build_key_is_the_one_ci_baked_in
bin=$(VOLTIP_BUILD_KEY=$key VOLTIP_BUILD_KEY_VERSION=$version VOLTIP_REQUIRE_BUILD_KEY=1 VOLTIP_EXPECT_PROOF=$expect \
  cargo test -q -p voltip-access --lib --no-run --message-format=json \
  | python3 -c 'import json,sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "voltip_access":
        print(m["executable"])')
VOLTIP_BUILD_KEY=$key VOLTIP_BUILD_KEY_VERSION=$version voltip_scan_build_key "$bin" "check-build-key"

echo "check-build-key: a key for another version must not build"
if VOLTIP_BUILD_KEY=$key VOLTIP_BUILD_KEY_VERSION=0.0.0-other cargo check -q -p voltip-access 2>/dev/null; then
  echo "check-build-key: a key for another version built" >&2
  exit 1
fi
echo "check-build-key: a release build without a key must not build"
if VOLTIP_REQUIRE_BUILD_KEY=1 cargo check -q -p voltip-access 2>/dev/null; then
  echo "check-build-key: VOLTIP_REQUIRE_BUILD_KEY=1 built without a key" >&2
  exit 1
fi
# Leave the crate built without a key, as every other gate expects.
cargo check -q -p voltip-access
echo "check-build-key: ok"
