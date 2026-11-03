# shellcheck shell=bash
# This version's build key (docs/pairing.md 「设备票据」, crates/voltip-access/build.rs):
# K_build(v) = HMAC-SHA256(K_master, "voltip-access/build/v1/" ‖ v), v being the root package.json's
# version. The master secret comes from VOLTIP_ACCESS_MASTER (CI: the one step that holds it) or
# from ~/.config/voltip/access/master.key (local packaging; VOLTIP_ACCESS_MASTER_FILE names another
# file). It is never exported to a build, and neither key is ever printed.
#
#   voltip_build_key     export VOLTIP_BUILD_KEY and VOLTIP_BUILD_KEY_VERSION (local packaging);
#                        without a master secret nothing is set and the build has no key
#   voltip_build_key_ci  the same into $GITHUB_ENV, masked, with VOLTIP_REQUIRE_BUILD_KEY=1; a
#                        missing master secret fails

# The key for package.json's version from the master secret in the environment variable named $1.
voltip_derive_build_key() {
  python3 -I - "$1" <<'PY'
import hashlib, hmac, json, os, sys
master = bytes.fromhex(os.environ[sys.argv[1]].strip())
if len(master) < 32:
    sys.exit("build-key: the master secret is shorter than 32 bytes")
version = json.load(open("package.json", encoding="utf-8"))["version"]
print(hmac.new(master, ("voltip-access/build/v1/" + version).encode(), hashlib.sha256).hexdigest(), version)
PY
}

voltip_build_key() {
  local file=${VOLTIP_ACCESS_MASTER_FILE:-$HOME/.config/voltip/access/master.key} derived
  # A key already in the environment (CI's derive step) is the one to build with.
  if [ -n "${VOLTIP_BUILD_KEY:-}" ]; then
    echo "build-key: using the build key in the environment for ${VOLTIP_BUILD_KEY_VERSION:-?} (value not shown)"
    return 0
  fi
  if [ -z "${VOLTIP_ACCESS_MASTER:-}" ]; then
    [ -f "$file" ] || { echo "build-key: no master secret — building without the build key (no device tickets)"; return 0; }
    derived=$(VOLTIP_ACCESS_MASTER=$(cat "$file") voltip_derive_build_key VOLTIP_ACCESS_MASTER) || return 1
  else
    derived=$(voltip_derive_build_key VOLTIP_ACCESS_MASTER) || return 1
  fi
  export VOLTIP_BUILD_KEY=${derived% *} VOLTIP_BUILD_KEY_VERSION=${derived#* }
  echo "build-key: derived the build key for $VOLTIP_BUILD_KEY_VERSION (value not shown)"
}

voltip_build_key_ci() {
  [ -n "${VOLTIP_ACCESS_MASTER:-}" ] || { echo "::error::repository secret VOLTIP_ACCESS_MASTER is empty; a release package must carry the build key (see .github/README-secrets.md)"; return 1; }
  local derived key version
  derived=$(voltip_derive_build_key VOLTIP_ACCESS_MASTER) || return 1
  key=${derived% *} version=${derived#* }
  echo "::add-mask::$key"
  {
    echo "VOLTIP_BUILD_KEY=$key"
    echo "VOLTIP_BUILD_KEY_VERSION=$version"
    echo "VOLTIP_REQUIRE_BUILD_KEY=1"
  } >>"$GITHUB_ENV"
  echo "build-key: the build key for $version is in this job's environment (masked)"
}
