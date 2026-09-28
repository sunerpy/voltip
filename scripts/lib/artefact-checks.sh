# shellcheck shell=bash
# Checks on a built binary, shared by scripts/build-{linux,windows}-x64.sh and
# release-candidate.yml (the macOS leg too: `strings -a` makes Apple's strings read the whole
# Mach-O, not just __TEXT).
#
# `grep` reads the whole stream here on purpose: under `set -o pipefail`, `grep -q` stops at the
# first match, the producer (`strings`, `readelf`) dies of SIGPIPE, and the pipeline counts as
# failed. The provider-key scan then passed exactly when a key was found early in a large binary.

# Fail when a provider API key (Groq `gsk_…`, OpenAI `sk-…`) is baked into <file>. The client may
# carry the revocable app token for the built-in edge; provider keys live on the edge hosts.
# voltip_scan_provider_keys <file> <label>
voltip_scan_provider_keys() {
  # A binary that is not there is not clean: strings would fail and the pipeline read as "no key".
  [ -f "$1" ] && [ -r "$1" ] || {
    echo "$2: $1 is missing or unreadable — nothing to scan" >&2
    return 1
  }
  if strings -a -n 20 "$1" | grep -E "gsk_[A-Za-z0-9]{20,}|sk-[A-Za-z0-9]{32,}" >/dev/null; then
    echo "$2: provider API key found inside $1 — refusing to ship" >&2
    return 1
  fi
}

# Whether the output of a command matches an extended regex, read to the end:
# voltip_output_has <regex> <command> [args…]
voltip_output_has() {
  local regex=$1
  shift
  "$@" | grep -E -- "$regex" >/dev/null
}
